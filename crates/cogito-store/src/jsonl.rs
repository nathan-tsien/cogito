//! JSONL-file-backed `ConversationStore`.
//!
//! **Scope: dev/debug only.** This backend is the v0.1 default while the
//! `postgres` backend (v0.4) is being built. It is intentionally simple:
//!
//! - One file per session at `<root>/<session_id>.jsonl`.
//! - Per-event userspace flush via `tokio::fs::File::flush`.
//! - **No `sync_data` / `fsync`**: process crash is OK; power loss may
//!   lose recent events. Use Postgres (v0.4) for production durability.
//! - No rotation, no path sharding, no internal index.
//!
//! See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §5 for rationale.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::{
    ConversationEvent, ConversationStore, SCHEMA_VERSION, SessionId, StoreError,
};
use dashmap::DashMap;
use futures::stream::BoxStream;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// JSONL backend for `ConversationStore`. Dev/debug only — see module docs.
pub struct JsonlStore {
    root: PathBuf,
    handles: DashMap<SessionId, Arc<Mutex<File>>>,
}

impl JsonlStore {
    /// Create a new store rooted at `root`. No I/O is performed; the
    /// directory is created lazily on first append.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            handles: DashMap::new(),
        }
    }

    fn path_for(&self, session_id: &SessionId) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    async fn handle_for(&self, session_id: &SessionId) -> Result<Arc<Mutex<File>>, StoreError> {
        if let Some(existing) = self.handles.get(session_id) {
            return Ok(Arc::clone(&existing));
        }
        tokio::fs::create_dir_all(&self.root).await?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.path_for(session_id))
            .await?;
        let arc = Arc::new(Mutex::new(file));
        // Race-tolerant insert: if another task raced us, prefer the
        // existing entry to ensure all writers share one handle.
        let entry = self
            .handles
            .entry(*session_id)
            .or_insert_with(|| Arc::clone(&arc));
        Ok(Arc::clone(&entry))
    }
}

#[async_trait]
impl ConversationStore for JsonlStore {
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError> {
        let handle = self.handle_for(&event.session_id).await?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut file = handle.lock().await;
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }

    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError> {
        if let Some(handle) = self.handles.get(&session_id) {
            let mut file = handle.lock().await;
            file.flush().await?;
        }
        Ok(())
    }

    async fn close(&self, session_id: SessionId) -> Result<(), StoreError> {
        if let Some((_, handle)) = self.handles.remove(&session_id) {
            let mut file = handle.lock().await;
            file.flush().await?;
            // File handle drops with the Arc.
        }
        Ok(())
    }

    async fn latest_seq(&self, session_id: SessionId) -> Result<Option<u64>, StoreError> {
        let path = self.path_for(&session_id);
        if !path.exists() {
            return Ok(None);
        }
        let text = tokio::fs::read_to_string(&path).await?;
        let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
            return Ok(None);
        };
        let event: ConversationEvent = serde_json::from_str(last)?;
        Ok(Some(event.seq))
    }

    fn replay(
        &self,
        session_id: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>> {
        let path = self.path_for(&session_id);
        Box::pin(async_stream::try_stream! {
            let file = match File::open(&path).await {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => Err(StoreError::from(e))?,
            };
            let mut lines = BufReader::new(file).lines();
            while let Some(line) = lines.next_line().await.map_err(StoreError::from)? {
                if line.trim().is_empty() {
                    continue;
                }
                let event: ConversationEvent = serde_json::from_str(&line)?;
                if event.schema_version > SCHEMA_VERSION {
                    Err(StoreError::UnsupportedSchemaVersion {
                        found: event.schema_version,
                        supported: SCHEMA_VERSION,
                    })?;
                }
                if event.seq > from_seq {
                    yield event;
                }
            }
        })
    }
}

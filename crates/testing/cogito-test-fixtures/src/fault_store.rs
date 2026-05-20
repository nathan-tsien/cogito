//! Test-only `ConversationStore` wrapper that injects faults after writing
//! the N-th event. Used by chaos tests to simulate process crashes:
//! - X path: `panic!` with the given message (simulates abrupt crash).
//! - Y path: signal a `oneshot` then return Ok (test then drives clean shutdown).
//!
//! Production code is zero-modified — fault injection lives entirely here
//! by wrapping any `ConversationStore` trait object.

// PanicAt is the only intentional panic site in the codebase; the chaos
// test machinery needs it to simulate abrupt actor death. Clippy denies
// `panic!` at the workspace level, so we allow it locally.
#![allow(clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::{ConversationStore, StoreError};
use futures::stream::BoxStream;
use tokio::sync::{Mutex, oneshot};

/// Wraps any `ConversationStore` and triggers a configurable fault after
/// the N-th successful `append`. The event IS persisted before the fault
/// fires — this models "wrote, then crashed".
pub struct FaultInjectingStore<S> {
    inner: Arc<S>,
    written_count: AtomicU64,
    trigger: Mutex<FaultTrigger>,
}

/// Fault behavior configured via [`FaultInjectingStore::set_trigger`].
/// Default is [`FaultTrigger::Disabled`] (pure pass-through).
pub enum FaultTrigger {
    /// No fault injected; pass-through behavior.
    Disabled,
    /// After the N-th `append` (1-indexed) completes, `panic!` with the
    /// given message. Used by the X path to simulate abrupt process death.
    PanicAt {
        /// 1-indexed count: panic AFTER the N-th append.
        event_no: u64,
        /// Panic message.
        message: &'static str,
    },
    /// After the N-th `append` completes, fire the oneshot then return Ok.
    /// The test side `await`s the receiver, then calls
    /// `SessionHandle::shutdown` to model a clean graceful crash boundary.
    NotifyAt {
        /// 1-indexed count: notify AFTER the N-th append.
        event_no: u64,
        /// Sender; consumed on the matching append (taken via `Option::take`).
        signal: Option<oneshot::Sender<()>>,
    },
}

impl<S> FaultInjectingStore<S> {
    /// Wrap an existing store. Default trigger is `Disabled`.
    pub fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            written_count: AtomicU64::new(0),
            trigger: Mutex::new(FaultTrigger::Disabled),
        }
    }

    /// Reconfigure the trigger. Useful for staging different scenarios
    /// within one test.
    pub async fn set_trigger(&self, trigger: FaultTrigger) {
        *self.trigger.lock().await = trigger;
    }

    /// Inspect how many appends have completed so far.
    #[must_use]
    pub fn written_count(&self) -> u64 {
        self.written_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl<S> ConversationStore for FaultInjectingStore<S>
where
    S: ConversationStore,
{
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError> {
        self.inner.append(event).await?;
        let n = self.written_count.fetch_add(1, Ordering::SeqCst) + 1;

        let mut trigger = self.trigger.lock().await;
        match &mut *trigger {
            FaultTrigger::PanicAt { event_no, message } if *event_no == n => {
                panic!("FaultInjectingStore: {message} (event_no={n})");
            }
            FaultTrigger::NotifyAt { event_no, signal } if *event_no == n => {
                if let Some(tx) = signal.take() {
                    // Ignore send errors — receiver dropped is the test side's
                    // problem, not ours.
                    let _ = tx.send(());
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError> {
        self.inner.flush(session_id).await
    }

    async fn close(&self, session_id: SessionId) -> Result<(), StoreError> {
        self.inner.close(session_id).await
    }

    async fn latest_seq(&self, session_id: SessionId) -> Result<Option<u64>, StoreError> {
        self.inner.latest_seq(session_id).await
    }

    fn replay(
        &self,
        session_id: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>> {
        self.inner.replay(session_id, from_seq)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::ids::EventId;
    use cogito_protocol::{ConversationEvent, EventPayload, SCHEMA_VERSION, SessionMeta};
    use cogito_store_jsonl::JsonlStore;
    use ulid::Ulid;

    fn evt(session_id: SessionId, seq: u64) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::from(Ulid::new()),
            session_id,
            turn_id: None,
            seq,
            ts: Utc::now(),
            payload: EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "test".into(),
                    ..Default::default()
                },
            },
        }
    }

    #[tokio::test]
    async fn notify_at_fires_after_nth_append() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let inner = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let store = FaultInjectingStore::new(inner);

        let (tx, mut rx) = oneshot::channel();
        store
            .set_trigger(FaultTrigger::NotifyAt {
                event_no: 2,
                signal: Some(tx),
            })
            .await;

        let session = SessionId::new();
        store.append(&evt(session, 0)).await?;
        assert!(
            rx.try_recv().is_err(),
            "should not notify before event_no=2"
        );

        store.append(&evt(session, 1)).await?;
        // After the 2nd append, the oneshot should be fired.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut rx)
                .await
                .is_ok(),
            "notify did not fire after event_no=2"
        );

        // Third append should pass through (trigger already consumed).
        store.append(&evt(session, 2)).await?;
        assert_eq!(store.written_count(), 3);

        Ok(())
    }

    #[tokio::test]
    async fn disabled_passes_through() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let inner = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let store = FaultInjectingStore::new(inner);

        let session = SessionId::new();
        for i in 0..5 {
            store.append(&evt(session, i)).await?;
        }
        assert_eq!(store.written_count(), 5);

        Ok(())
    }

    // NOTE: PanicAt is not unit-tested here — `#[should_panic]` interacts
    // badly with tokio's runtime teardown and the chaos test in P5.6 will
    // exercise it under real session conditions. The semantics are obvious
    // from inspection (the match arm calls `panic!`).
}

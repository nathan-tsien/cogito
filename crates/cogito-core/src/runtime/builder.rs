//! `Runtime` and `RuntimeBuilder` — the entry point. Caller injects a
//! tokio `Handle`, opens sessions, and observes their lifecycle through
//! the returned `SessionHandle`s.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::runtime::Handle as TokioHandle;
use tokio_util::sync::CancellationToken;

use super::handle::SessionHandle;
use super::types::{OpenMode, SessionId};

/// The DI container and session registry. One `Runtime` per cogito-using
/// process is the typical pattern; opening N sessions spawns N actor
/// tasks on the injected tokio runtime.
pub struct Runtime {
    /// Tokio runtime handle that all `SessionActor` tasks are spawned onto.
    handle: TokioHandle,
    /// Active sessions keyed by `SessionId`. Inserted on `open_session`,
    /// removed when the actor task exits (clean shutdown or panic).
    sessions: DashMap<SessionId, SessionHandle>,
    /// Reserved for v0.4 process-level shutdown coordination
    /// (ADR-0010). Stored but not consumed in v0.1.
    shutdown_token: CancellationToken,
}

impl Runtime {
    /// Open or attach a session. See `OpenMode` for the three semantics.
    /// Awaits until the replay phase completes (or fails) so the caller
    /// sees any `ResumeError` synchronously instead of on the first send.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeError::SessionAlreadyOpen` if `id` is already in
    /// the registry. Returns `RuntimeError::ResumeFailed` if the replay
    /// phase rejects the persisted state.
    #[allow(clippy::unused_async)] // Plan 2 (Sprint 1+) replaces todo!() with real await points
    #[allow(clippy::todo)] // intentional stub — Plan 2 fills in the body
    pub async fn open_session(
        &self,
        id: SessionId,
        mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        let _ = (id, mode);
        let _ = &self.handle;
        let _ = &self.sessions;
        let _ = &self.shutdown_token;
        todo!(
            "Plan 2 (Sprint 1+): spawn SessionActor with catch_unwind, \
             run replay phase, install in sessions DashMap, return \
             SessionHandle once the ready oneshot fires"
        )
    }

    /// Begin a builder. Caller injects all dependencies.
    #[must_use]
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }
}

/// Builder for `Runtime`. Caller may set `handle()` explicitly or let it
/// default to `tokio::runtime::Handle::current()` at `build()` time.
#[derive(Default)]
pub struct RuntimeBuilder {
    handle: Option<TokioHandle>,
    shutdown_token: Option<CancellationToken>,
}

impl RuntimeBuilder {
    /// Override the tokio `Handle`. Defaults to `Handle::current()` at
    /// `build()` time.
    #[must_use]
    pub fn handle(mut self, handle: TokioHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Reserve a process-level cancellation token. v0.1 stores it but
    /// does not consume it; v0.4 wires it through `shutdown_all()` per
    /// ADR-0010.
    #[must_use]
    pub fn shutdown_token(mut self, token: CancellationToken) -> Self {
        self.shutdown_token = Some(token);
        self
    }

    /// Finalize.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeError::NoTokioRuntime` if no `Handle` was injected
    /// AND `Handle::try_current()` fails (no tokio runtime is active in
    /// the calling thread).
    pub fn build(self) -> Result<Arc<Runtime>, RuntimeError> {
        let handle = match self.handle {
            Some(h) => h,
            None => TokioHandle::try_current()
                .map_err(|e| RuntimeError::NoTokioRuntime(e.to_string()))?,
        };
        let runtime = Runtime {
            handle,
            sessions: DashMap::new(),
            shutdown_token: self.shutdown_token.unwrap_or_default(),
        };
        Ok(Arc::new(runtime))
    }
}

/// Errors from the Runtime layer surface (not from inside a turn).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// `Handle::try_current()` failed at build time.
    #[error("no current tokio runtime: {0}")]
    NoTokioRuntime(String),
    /// The session id was already open in this `Runtime`.
    #[error("session already open: {0}")]
    SessionAlreadyOpen(SessionId),
    /// Resume-phase failure for `OpenMode::Resume` or `OpenMode::Attach`.
    #[error("resume failed: {0}")]
    ResumeFailed(String),
}

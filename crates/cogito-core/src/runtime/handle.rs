//! Caller-side handle to one session. Cheap to clone; multiple handles
//! to the same session share the underlying actor.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::stream::StreamEvent;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{SessionCommand, ShutdownOutcome};

/// Shared state between a `SessionHandle` and the `SessionActor` task it
/// fronts. Held by `Arc` on the caller side so multiple handles to the
/// same session route through the same actor.
#[allow(dead_code)] // Plan 2 fills in the consumers
pub(super) struct SessionShared {
    /// Inbound command channel to the actor.
    pub(super) mailbox_tx: mpsc::Sender<SessionCommand>,
    /// Outbound broadcast of real-time events to all subscribers.
    pub(super) events_tx: broadcast::Sender<StreamEvent>,
    /// Token for the *currently* in-flight turn. The actor replaces it on
    /// each turn start; the caller's `cancel_turn` always operates on
    /// whichever token is current at call time. Guarded by a `Mutex` so
    /// the actor can swap atomically without a writer lock.
    pub(super) current_cancel_token: parking_lot::Mutex<CancellationToken>,
}

/// Caller-facing handle to a session. Clone freely; all clones funnel into
/// the same `SessionActor` task.
#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<SessionShared>,
}

impl SessionHandle {
    /// Construct from the shared state. Crate-private — only the `Runtime`
    /// (during `open_session`) creates these.
    #[allow(dead_code)] // Plan 2 (Sprint 1+): called from Runtime::open_session
    pub(super) fn new(shared: Arc<SessionShared>) -> Self {
        Self { shared }
    }

    /// Send a new user message; the actor will spawn a `TurnDriver`.
    /// Awaits (mailbox backpressure) if the actor is overwhelmed.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    #[allow(clippy::unused_async)] // Plan 2 (Sprint 2) replaces todo!() with real await points
    #[allow(clippy::todo)] // intentional stub — Plan 2 fills in the body
    pub async fn send_user(&self, text: impl Into<String>) -> Result<(), SessionError> {
        let _ = text.into();
        let _ = &self.shared;
        todo!(
            "Plan 2 (Sprint 2): wrap in SessionCommand::Input(NewMessage) \
             and send on mailbox_tx; map SendError to SessionClosed"
        )
    }

    /// Subscribe to the real-time event stream. Multiple subscribers are
    /// allowed; slow subscribers receive `Lagged(n)` errors per
    /// `broadcast::Receiver` semantics and must decide how to recover
    /// (e.g., resubscribe and re-fetch state from the persisted log).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.shared.events_tx.subscribe()
    }

    /// Cancel the current turn (if any). Cooperative: tools that want to
    /// honor cancellation must `select!` on `ExecCtx.cancel`. Has no
    /// effect if no turn is running. If the actor is in `PausedOnJob`,
    /// also sends an `InternalCancel` command so the actor can call
    /// `jobs.cancel`.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    #[allow(clippy::unused_async)] // Plan 2 (Sprint 2) replaces todo!() with real await points
    #[allow(clippy::todo)] // intentional stub — Plan 2 fills in the body
    pub async fn cancel_turn(&self) -> Result<(), SessionError> {
        let _ = &self.shared;
        todo!(
            "Plan 2 (Sprint 2): self.shared.current_cancel_token.lock().cancel(); \
             then send SessionCommand::InternalCancel and await ack oneshot"
        )
    }

    /// Gracefully shut the session down. Drains the mailbox, flushes the
    /// store writer, and waits up to `deadline` for any in-flight turn
    /// to complete before forcing a cancel + abort.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::ShuttingDown` if another shutdown is already
    /// in progress for this session.
    #[allow(clippy::unused_async)] // Plan 2 (Sprint 2) replaces todo!() with real await points
    #[allow(clippy::todo)] // intentional stub — Plan 2 fills in the body
    pub async fn shutdown(self, deadline: Duration) -> Result<ShutdownOutcome, SessionError> {
        let _ = deadline;
        let _ = self.shared;
        todo!(
            "Plan 2 (Sprint 2): send SessionCommand::Shutdown {{ deadline, ack }} \
             on mailbox_tx; await ack oneshot; map errors to SessionError"
        )
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        // Plan 2 (Sprint 2): if this is the last Arc clone, best-effort
        // send Shutdown with default timeout so the actor task doesn't
        // leak. Today the Arc keeps the channel alive and the actor would
        // park forever — the explicit shutdown() call is the safe path.
    }
}

/// Errors from caller-facing `SessionHandle` operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// Actor task has exited and is no longer accepting commands.
    #[error("session is closed")]
    SessionClosed,
    /// Caller tried to use the handle after `shutdown` returned.
    #[error("shutdown already in progress")]
    ShuttingDown,
}

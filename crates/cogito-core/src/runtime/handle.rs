//! Caller-side handle to one session. Cheap to clone; multiple handles
//! to the same session share the underlying actor.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::job::JobCompletionEvent;
use cogito_protocol::stream::StreamEvent;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{NewMessage, SessionCommand, SessionId, ShutdownOutcome};

/// Shared state between a `SessionHandle` and the per-session loop task it
/// fronts. Held by `Arc` on the caller side so multiple handles to the
/// same session route through the same task.
pub(super) struct SessionShared {
    /// Identifier of the session this handle fronts.
    pub(super) session_id: SessionId,
    /// Inbound command channel to the actor.
    pub(super) mailbox_tx: mpsc::Sender<SessionCommand>,
    /// Outbound broadcast of real-time events to all subscribers.
    pub(super) events_tx: broadcast::Sender<StreamEvent>,
    /// Token for the *currently* in-flight turn. The actor replaces it on
    /// each turn start; the caller's `cancel_turn` always operates on
    /// whichever token is current at call time.
    ///
    /// Uses `parking_lot::Mutex` for non-poisoning ergonomics — this lock
    /// sits in the cancel hot path and a poison on actor panic would force
    /// every subsequent cancel to bubble an unrelated `PoisonError`.
    pub(super) current_cancel_token: parking_lot::Mutex<CancellationToken>,
    /// Sender side of the job-completion channel exposed so `JobManager`
    /// can deliver events to this session (Sprint 4).
    #[allow(dead_code)] // Sprint 4 wires JobManager -> SessionHandle
    pub(super) job_completion_tx: mpsc::Sender<JobCompletionEvent>,
}

/// Caller-facing handle to a session. Clone freely; all clones funnel into
/// the same per-session loop task.
#[derive(Clone)]
pub struct SessionHandle {
    pub(super) shared: Arc<SessionShared>,
}

impl SessionHandle {
    /// Construct from the shared state. Crate-private — only `Runtime`
    /// (during `open_session`) creates these.
    pub(super) fn new(shared: Arc<SessionShared>) -> Self {
        Self { shared }
    }

    /// Send a new user message; the actor will spawn a `TurnDriver`.
    /// Awaits (mailbox backpressure) if the actor is overwhelmed.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn send_user(&self, text: impl Into<String>) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::Input(NewMessage { text: text.into() }))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }

    /// Subscribe to the real-time event stream. Multiple subscribers are
    /// allowed; slow subscribers receive `Lagged(n)` errors per
    /// `broadcast::Receiver` semantics and must decide how to recover.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.shared.events_tx.subscribe()
    }

    /// Cancel the current turn (if any). Cooperative: tools that want to
    /// honor cancellation must `select!` on `ExecCtx.cancel`. Has no
    /// effect if no turn is running. Also sends an `InternalCancel` command
    /// so the actor can cancel jobs in `PausedOnJob` state.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn cancel_turn(&self) -> Result<(), SessionError> {
        // Fire the token first so the running TurnDriver can cooperate.
        self.shared.current_cancel_token.lock().cancel();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shared
            .mailbox_tx
            .send(SessionCommand::InternalCancel { ack: tx })
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })?;
        // Wait for the actor to acknowledge the cancel.
        let _ = rx.await;
        Ok(())
    }

    /// Gracefully shut the session down. Drains the mailbox, waits up to
    /// `deadline` for any in-flight turn to complete, then exits.
    ///
    /// **Multi-handle semantics**: Calling `shutdown` on one clone closes
    /// the session for **all** clones. Subsequent operations on surviving
    /// clones will return `SessionError::SessionClosed` once the actor exits.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has already exited.
    pub async fn shutdown(self, deadline: Duration) -> Result<ShutdownOutcome, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shared
            .mailbox_tx
            .send(SessionCommand::Shutdown { deadline, ack: tx })
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })?;
        rx.await.map_err(|_| SessionError::SessionClosed {
            session_id: self.shared.session_id,
        })
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shared) == 1 {
            tracing::warn!(
                session_id = %self.shared.session_id,
                "last SessionHandle dropped without calling shutdown(); \
                 the session actor task may leak until process exit"
            );
        }
    }
}

/// Errors from caller-facing `SessionHandle` operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// Actor task has exited and is no longer accepting commands.
    #[error("session {session_id} is closed")]
    SessionClosed {
        /// Identifier of the closed session.
        session_id: SessionId,
    },
    /// Caller tried to use the handle after `shutdown` started.
    #[error("session {session_id} shutdown already in progress")]
    ShuttingDown {
        /// Identifier of the session being shut down.
        session_id: SessionId,
    },
}

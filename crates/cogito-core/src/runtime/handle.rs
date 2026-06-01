//! Caller-side handle to one session. Cheap to clone; multiple handles
//! to the same session share the underlying actor.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::job::{JobCompletionEvent, JobId};
use cogito_protocol::stream::StreamEvent;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{SessionCommand, SessionId, ShutdownOutcome, TurnTrigger};

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
    /// Token slot for the *currently* in-flight turn. Held as an
    /// `Arc<parking_lot::Mutex<...>>` so the actor's swap on each
    /// `spawn_turn_driver` is observable to every `SessionHandle` clone —
    /// `cancel_turn` then locks this Arc, reads the live token, and fires it.
    ///
    /// Sharing one Arc with `SessionState.current_cancel_token` is mandatory:
    /// a sibling clone of the initial token would only ever cancel turn 1
    /// (see the `cancel_after_first_turn` regression test).
    ///
    /// Uses `parking_lot::Mutex` for non-poisoning ergonomics — this lock
    /// sits in the cancel hot path and a poison on actor panic would force
    /// every subsequent cancel to bubble an unrelated `PoisonError`.
    pub(super) current_cancel_token: Arc<parking_lot::Mutex<CancellationToken>>,
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

    /// Construct a fully detached `SessionHandle` suitable for tests that
    /// only need a value to plug into a struct field. The handle's
    /// channels are bounded with capacity 1 and have no receiver task
    /// behind them, so every method that talks to the actor
    /// (`submit`, `cancel_turn`, `shutdown`, ...) will fail with
    /// `SessionError::SessionClosed`. Tests must only construct this
    /// handle and never invoke its async methods.
    ///
    /// Gated behind the `test-support` feature so the stub never enters
    /// production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_stub() -> Self {
        use tokio::sync::{broadcast, mpsc};
        use tokio_util::sync::CancellationToken;

        let (mailbox_tx, _mailbox_rx) = mpsc::channel(1);
        let (events_tx, _events_rx) = broadcast::channel(1);
        let (job_completion_tx, _job_completion_rx) = mpsc::channel(1);
        let shared = Arc::new(SessionShared {
            session_id: SessionId::new(),
            mailbox_tx,
            events_tx,
            current_cancel_token: Arc::new(parking_lot::Mutex::new(CancellationToken::new())),
            job_completion_tx,
        });
        Self { shared }
    }

    /// Submit a [`TurnTrigger`]. The session loop spawns a `TurnDriver`
    /// if no turn is in flight. **Canonical entry point** for any new
    /// trigger source — `submit_user_text` is a convenience shim that
    /// calls `submit(TurnTrigger::UserText(text.into()))`. See ADR-0016 §2.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn submit(&self, trigger: TurnTrigger) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::Trigger(trigger))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }

    /// Replace one or more of this session's providers. Each `Some` field
    /// of `spec` swaps the corresponding live provider; the change takes
    /// effect at the next turn boundary. `tenant_id` / `user_id` are
    /// ignored (identity is fixed at open time). See ADR-0028.
    ///
    /// # Errors
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn update_session(
        &self,
        spec: crate::runtime::SessionSpec,
    ) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::UpdateSession(Box::new(spec)))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }

    /// Submit a user-typed text message; the actor will spawn a `TurnDriver`.
    /// Awaits (mailbox backpressure) if the actor is overwhelmed.
    ///
    /// Convenience wrapper around [`SessionHandle::submit`] — equivalent
    /// to `submit(TurnTrigger::UserText(text.into()))`. Retained because
    /// user-typed text is the dominant path and callers should not have
    /// to spell out the enum for it. See ADR-0016 §2.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn submit_user_text(&self, text: impl Into<String>) -> Result<(), SessionError> {
        self.submit(TurnTrigger::UserText(text.into())).await
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
    /// effect if no turn is running. If the session is paused on a
    /// background job, additionally asks the actor to call
    /// `JobManager::cancel(job_id)` so the spawned task actually stops —
    /// the per-session cancel token alone cannot reach a job that the
    /// `JobManager` owns.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn cancel_turn(&self) -> Result<(), SessionError> {
        // Fire the token first so the running TurnDriver can cooperate.
        // Preserved as the first action so the existing per-turn cancel
        // behavior (covered by `cancel_after_first_turn`) is unchanged.
        self.shared.current_cancel_token.lock().cancel();

        // Preserve the existing CancelAck round-trip so callers continue
        // to observe ordered "cancel-then-resume" semantics.
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shared
            .mailbox_tx
            .send(SessionCommand::InternalCancel { ack: tx })
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })?;
        let _ = rx.await;

        // If the session is paused on a job, route the cancel through
        // the JobManager. A best-effort send: if the mailbox closed
        // between the ack above and now, we already returned earlier or
        // there is nothing else to do.
        if let Some(job_id) = self.snapshot_paused_job_id().await {
            let _ = self
                .shared
                .mailbox_tx
                .send(SessionCommand::CancelJob { job_id })
                .await;
        }
        Ok(())
    }

    /// Ask the actor for the currently paused job id, if any. Returns
    /// `None` when the session is not in `PausedOnJob`, when the actor
    /// has already exited, or when the reply channel was dropped.
    async fn snapshot_paused_job_id(&self) -> Option<JobId> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self
            .shared
            .mailbox_tx
            .send(SessionCommand::SnapshotInFlight { reply: tx })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
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

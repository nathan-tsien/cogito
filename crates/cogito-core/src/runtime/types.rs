//! Channel-protocol value types used between caller, actor, store writer,
//! and `JobManager`.

use cogito_protocol::job::{JobCompletionEvent, JobId, JobOutcome};
use tokio::sync::oneshot;

/// Opaque session identifier. Caller picks the string; cogito does not
/// interpret it (typical: ulid or user-domain id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl<S: Into<String>> From<S> for SessionId {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// How `Runtime::open_session` should treat an existing session id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenMode {
    /// Session must not exist in the store. Writes a `SessionStarted` event.
    New,
    /// Session must exist; replay through H03 establishes resume point.
    /// Panics on missing log (contract violation).
    Resume,
    /// Like `Resume` but returns `Err(ResumeError::NotFound)` instead of
    /// panicking on a missing log.
    Attach,
}

/// Outcome of an attempted `SessionHandle::shutdown`.
#[derive(Debug)]
pub struct ShutdownOutcome {
    /// True if the actor drained cleanly without forced abort.
    pub clean: bool,
    /// If a turn was in-flight when shutdown started, a human-readable
    /// description of the last known state (for caller-side logging only —
    /// the persisted event log is the authority).
    pub in_flight_cancelled: Option<String>,
}

/// Commands the caller (and the actor's own internal subsystems) may send
/// into the mailbox. `CancelTurn` does *not* go through this enum — it fires
/// the per-turn `CancellationToken` directly to bypass FIFO ordering. See
/// spec §4 for the rationale.
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionCommand {
    /// Caller-driven new user input. Triggers a new `TurnDriver`.
    Input(NewMessage),

    /// Synthesized by the actor after receiving a `JobCompletionEvent` on
    /// the `job_completion` channel. Re-spawns `TurnDriver` with resume state.
    JobCompleted {
        /// Identifies which background job completed.
        job_id: JobId,
        /// The terminal result of the job.
        outcome: JobOutcome,
    },

    /// Sent by `SessionHandle::cancel_turn` when the actor is in
    /// `PausedOnJob` (the cancel token alone cannot reach a non-existent
    /// `TurnDriver` task; this asks the actor to call `jobs.cancel`).
    InternalCancel {
        /// Signals the caller once the cancel has been forwarded.
        ack: oneshot::Sender<()>,
    },

    /// Graceful shutdown with a deadline. Actor drains the mailbox,
    /// flushes the store writer, then exits.
    Shutdown {
        /// How long to wait for the in-flight turn to finish before forcing a cancel.
        deadline: std::time::Duration,
        /// Signals the caller with the outcome once the actor exits.
        ack: oneshot::Sender<ShutdownOutcome>,
    },
}

/// User-facing input for a new turn. Wrapped in `SessionCommand::Input` so
/// the command enum stays trivially extensible.
#[derive(Debug, Clone)]
pub struct NewMessage {
    /// Plain text content of the user's message. v0.2 may extend this to
    /// `Vec<ContentBlock>` for multimodal input.
    pub text: String,
}

/// Translate a `JobCompletionEvent` from the dedicated job-completion mpsc
/// into a `SessionCommand::JobCompleted` for FIFO mailbox ordering. The
/// actor uses this `From` impl whenever it dequeues from `job_completion_rx`.
impl From<JobCompletionEvent> for SessionCommand {
    fn from(event: JobCompletionEvent) -> Self {
        SessionCommand::JobCompleted {
            job_id: event.job_id,
            outcome: event.outcome,
        }
    }
}

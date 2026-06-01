//! Channel-protocol value types used between caller, actor, store writer,
//! and `JobManager`.

use cogito_protocol::job::{JobCompletionEvent, JobId};
use tokio::sync::oneshot;

// Re-export the canonical session identifier from the protocol layer so all
// runtime code uses the same type without an extra import path.
pub use cogito_protocol::ids::SessionId;
pub use cogito_protocol::turn_trigger::TurnTrigger;

/// How `Runtime::open_session` should treat an existing session id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenMode {
    /// Session must not exist in the store. Writes a `SessionStarted` event.
    New,
    /// Session must exist; replay through H03 establishes resume point.
    /// Returns `Err(RuntimeError::ResumeFailed { id, reason })` with a
    /// clear "no such session in store" reason when the log is empty.
    /// `Resume` is the "I know this session exists" form — missing log
    /// is a caller bug.
    Resume,
    /// Like `Resume` but tolerant of a missing log: replay treats an
    /// empty store as a fresh session rather than an error.
    Attach,
}

/// Outcome of an attempted `SessionHandle::shutdown`.
#[derive(Debug)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    /// Actor drained the mailbox and exited cleanly. `in_flight_cancelled`
    /// is `Some(reason)` if a turn was still running at the shutdown deadline
    /// and had to be force-cancelled; `None` if the turn finished on its own.
    Clean {
        /// Human-readable description of the force-cancelled turn, or `None`
        /// if no turn was in-flight at shutdown time.
        in_flight_cancelled: Option<String>,
    },
    /// Resume failed before the mailbox loop started (schema mismatch or
    /// malformed event log).
    ResumeFailed(String),
    /// `JobManager` couldn't honor `on_complete` callback during `PausedOnJob`
    /// recovery (e.g. job id unknown to the manager). Runtime configuration
    /// failure, not a turn failure — no event is written to the log.
    JobManagerUnavailable(String),
}

/// Commands the caller (and the actor's own internal subsystems) may send
/// into the mailbox. `CancelTurn` does *not* go through this enum — it fires
/// the per-turn `CancellationToken` directly to bypass FIFO ordering. See
/// spec §4 for the rationale.
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionCommand {
    /// Caller-driven trigger. Spawns a new `TurnDriver` when no turn is
    /// in flight. v0.1 only carries `TurnTrigger::UserText`; future
    /// variants (multimedia user content, skill invocations, hook
    /// fires) land additively per ADR-0016 + ADR-0007 track B.
    Trigger(TurnTrigger),

    /// Replace one or more per-session providers; effective next turn.
    /// See ADR-0028. `tenant_id` / `user_id` on the spec are ignored
    /// (session identity is fixed at open time). Boxed because
    /// `SessionSpec` is large relative to the other variants.
    UpdateSession(Box<crate::runtime::SessionSpec>),

    /// Synthesized by the actor after receiving a `JobCompletionEvent` on
    /// the `job_completion` channel. Re-spawns `TurnDriver` with resume state.
    JobCompleted {
        /// The terminal job-completion event delivered by `JobManager`.
        event: JobCompletionEvent,
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

    /// Cancel a still-running job for the currently paused turn. Forwarded
    /// from `SessionHandle::cancel_turn` when `state.in_flight` is
    /// `PausedOnJob`. The actor calls `JobManager::cancel(job_id)`; the
    /// subsequent `JobCompleted { outcome: Cancelled }` flows through Arm
    /// 3 as normal.
    CancelJob {
        /// The job to cancel.
        job_id: JobId,
    },

    /// Probe `in_flight` from the handle. The actor replies with the
    /// `JobId` if the session is `PausedOnJob`, otherwise `None`. Used by
    /// `SessionHandle::cancel_turn` to decide whether to follow up with a
    /// `CancelJob` command.
    SnapshotInFlight {
        /// Reply channel: receives the paused job id, or `None` if the
        /// session is not currently `PausedOnJob`.
        reply: oneshot::Sender<Option<JobId>>,
    },
}

/// Translate a `JobCompletionEvent` from the dedicated job-completion mpsc
/// into a `SessionCommand::JobCompleted` for FIFO mailbox ordering. The
/// actor uses this `From` impl whenever it dequeues from `job_completion_rx`.
impl From<JobCompletionEvent> for SessionCommand {
    fn from(event: JobCompletionEvent) -> Self {
        SessionCommand::JobCompleted { event }
    }
}

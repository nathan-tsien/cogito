//! `SessionActor`: the long-lived per-session tokio task. Hosts the
//! `actor_main` loop that selects on the mailbox, on the in-flight
//! `TurnDriver` task (if any), and on the job-completion channel.
//!
//! Implementation is Plan 2 (Sprint 1 / 2). This module currently
//! exposes only the struct skeleton so other modules compile.

use std::time::Instant;

use cogito_protocol::job::JobId;
use cogito_protocol::turn::TurnOutcome;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// In-flight turn state held by the actor across mailbox iterations.
/// Cleared back to `None` on every terminal turn outcome.
#[allow(dead_code)] // Plan 2 fills in the consumers
pub(super) enum InFlight {
    /// A `TurnDriver` task is running; `turn_join` is its handle.
    Active {
        /// Join handle for the running `TurnDriver` task.
        turn_join: JoinHandle<TurnOutcome>,
        /// Wall-clock time at which this turn started (for observability).
        started_at: Instant,
    },
    /// The turn paused awaiting a `JobManager` callback. `job_id` is the
    /// router key; `paused_at_event_id` lets resume reconstruct context.
    PausedOnJob {
        /// The background job this session is waiting on.
        job_id: JobId,
        /// Event id at which the turn suspended (used by resume to find
        /// the correct replay offset).
        paused_at_event_id: String,
    },
}

/// Placeholder for actor entrypoint state. Plan 2 implements
/// `actor_main`, `replay_and_position`, `try_start_turn`,
/// `try_resume_from_job`, `handle_internal_cancel`, and `shutdown` here.
#[allow(dead_code)] // Plan 2 fills in the consumers
pub(super) struct ActorState {
    /// What the actor is currently doing, if anything.
    pub(super) in_flight: Option<InFlight>,
    /// Per-turn cancellation token — replaced whenever a new `TurnDriver`
    /// spawns, allowing `SessionHandle::cancel_turn` to operate on the
    /// most recent turn.
    pub(super) current_cancel_token: CancellationToken,
}

//! H03 Resume Coordinator — pure function from event log to resume decision.
//!
//! Sprint 3 implements the full decision table per spec §4–§5.
//! Pure function: same input → same output, no I/O, no clock, no random.
//! The actor calls `replay()` on startup and uses the result to bootstrap
//! either the FSM (via `TurnEntry`) or its own `InFlight` state.

use cogito_protocol::event::ConversationEvent;
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::ids::TurnId;
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::tool::ToolResult;

/// Output of H03 Resume Coordinator. Pure projection from the event log.
/// Never persisted (see spec §6 落盘语义).
#[derive(Debug, Clone, PartialEq)]
pub struct ResumeDecision {
    /// What state to resume into.
    pub point: ResumePoint,
    /// `seq` of the last event in the log when this decision was computed.
    /// `None` iff the log is empty (which also implies `point == FreshTurn`).
    /// Actor initializes its event-seq generator to `last_event_seq + 1`.
    pub last_event_seq: Option<u64>,
}

/// Resume entry point. Six variants covering every valid log shape.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ResumePoint {
    /// Empty log, or last turn ended in `TurnCompleted` / `TurnFailed`.
    /// Actor idles until the next user `Input`.
    FreshTurn,

    /// In-flight turn where the most recent model call did not complete
    /// (no `ModelCallCompleted` after the latest `ModelCallStarted`).
    /// FSM enters `Init`; H04 rebuilds prompt from the event log; one
    /// model call gets re-billed.
    RestartCurrentTurn {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
    },

    /// Most recent `ModelCallCompleted` is the latest event in the turn
    /// AND no `ToolUseRecorded` follows. Actor crashed between writing
    /// the sealing event and writing `TurnCompleted`. FSM enters
    /// `ModelCompleted` with output rebuilt from events; fast-paths to
    /// `Completed` without re-calling the model.
    ResumeFromModelCompleted {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
        /// Model output rebuilt from the event log.
        rebuilt_output: ModelOutput,
    },

    /// Tool dispatch round in progress. May have 0+ completed results.
    /// FSM enters `ToolDispatching`. `enter_turn` re-runs H07 on `pending`
    /// to re-validate against current schemas, and triggers H10+H05 to
    /// rebuild the tool surface.
    ResumeFromToolDispatching {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
        /// `ToolUseRecorded` since the latest `ModelCallCompleted` with no
        /// matching `ToolResultRecorded`. Order preserved from the log.
        pending: Vec<ResumePendingCall>,
        /// `(call_id, ToolResult)` pairs already in the log.
        completed: Vec<(String, ToolResult)>,
    },

    /// Turn paused on an async job. `TurnPaused` is the latest event;
    /// no `JobCompletedRecorded { job_id }` follows. Actor enters
    /// `InFlight::PausedOnJob` and re-registers `on_complete`.
    ResumePausedJob {
        /// The turn that was paused.
        turn_id: TurnId,
        /// The async job this turn is waiting on.
        job_id: JobId,
    },

    /// Async job completed but Brain didn't consume the
    /// `JobCompletedRecorded` event before the crash. FSM enters
    /// `ToolDispatching` with the just-completed result injected as the
    /// last entry of `completed_before_pause` + `call_id` resolved.
    ResumeAfterJobCompletion {
        /// The turn that was paused.
        turn_id: TurnId,
        /// The async job that completed.
        job_id: JobId,
        /// Outcome of the completed job.
        outcome: JobOutcome,
        /// Resolved by walking back to the latest unmatched `ToolUseRecorded`
        /// before `TurnPaused` (Sprint 3 invariant: ≤1 async dispatch per
        /// turn; Sprint 4 may add `call_id` to `TurnPaused` payload).
        call_id: String,
        /// Tool calls dispatched and completed before the pause.
        completed_before_pause: Vec<(String, ToolResult)>,
        /// Tool calls declared by the model but not yet dispatched at
        /// pause time. (Sprint 3 always empty; Sprint 4 may be non-empty.)
        pending_after_pause: Vec<ResumePendingCall>,
    },
}

/// Raw tool-call triple recovered from a `ToolUseRecorded` event.
/// Pre-validation — `enter_turn` re-runs through H07 before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePendingCall {
    /// The call ID from the original tool-use event.
    pub call_id: String,
    /// The tool name from the original tool-use event.
    pub tool_name: String,
    /// The raw arguments from the original tool-use event.
    pub args: serde_json::Value,
}

/// Errors from `replay`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    /// Event log contradicts itself (e.g., `JobCompletedRecorded` with no
    /// matching prior `TurnPaused`; nested `TurnStarted` without
    /// terminator).
    #[error("malformed event log: {0}")]
    Malformed(String),
    /// Event log was written by a newer schema version than this build supports.
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    /// A tool referenced by a recovered call is no longer registered.
    /// (Sprint 3 returns this from `enter_turn` re-validation, not from
    /// `replay()` itself; reserved here for completeness.)
    #[error("tool `{tool_name}` (call_id `{call_id}`) no longer registered")]
    ToolUnavailable {
        /// The call ID of the unavailable tool invocation.
        call_id: String,
        /// The name of the tool that is no longer registered.
        tool_name: String,
    },
    /// Persisted tool args fail current schema validation.
    #[error("tool `{tool_name}` schema rejects persisted args: {reason}")]
    ToolSchemaDrift {
        /// The name of the tool whose schema drifted.
        tool_name: String,
        /// Description of why the persisted args fail validation.
        reason: String,
    },
}

/// Sprint 3 P3.2 stub: returns `FreshTurn` with `last_event_seq` derived
/// from the events slice. Full decision table lands in P3.3.
///
/// # Errors
///
/// Currently does not error; signature reserves room for Sprint 3.
pub fn replay(events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    let last_event_seq = events.last().map(|e| e.seq);
    Ok(ResumeDecision {
        point: ResumePoint::FreshTurn,
        last_event_seq,
    })
}

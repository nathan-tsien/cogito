//! Turn terminal-state values. The Runtime layer returns these from
//! `SessionActor` after each turn completes. Caller observes via
//! `SessionHandle` or the `StreamEvent` stream.

use serde::{Deserialize, Serialize};

use crate::job::JobId;

/// Terminal outcome of a single turn iteration. Note the FSM may loop
/// internally (multiple sub-turns when the model calls sync tools and
/// continues); a turn ends only when one of these variants is produced.
///
/// **Serde representation note**: internally-tagged with `tag = "kind"`.
/// All variants are unit or struct (no newtype-with-primitive), and no
/// field name collides with the tag, so internal tagging is safe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnOutcome {
    /// Model returned `end_turn` without further tool calls.
    Completed,
    /// A tool returned `InvokeOutcome::Async`; the turn will resume when
    /// the matching `JobCompletionEvent` arrives.
    Paused {
        /// Identifier of the async job that must complete before resuming.
        job_id: JobId,
    },
    /// `SessionHandle::cancel_turn` fired during execution.
    Cancelled,
    /// Unrecoverable failure; details in `reason`. `recorded_event_id`
    /// points to the last persisted `ConversationEvent` for diagnosis.
    Failed {
        /// Structured reason for the failure.
        reason: TurnFailureReason,
        /// Identifier of the last persisted event, for diagnosis.
        recorded_event_id: String,
    },
}

/// Why a turn ended in `Failed`. Only Runtime-level errors (store I/O,
/// gateway hard failure, panic, timeout, hook reject) escape here; tool
/// errors stay inside `ToolResult::Error` and never bubble up.
///
/// **Serde representation note**: internally-tagged with `tag = "kind"`.
/// `ModelGatewayFailed` is a struct variant `{ message: String }` rather
/// than `ModelGatewayFailed(String)` because internally-tagged enums
/// cannot wrap a JSON primitive (the tag would have nowhere to go).
/// Similarly, `TurnPanicked.location` is an owned `String` rather than
/// `&'static str` so the type can derive `Deserialize` without a lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnFailureReason {
    /// `ConversationStore::append` returned an error.
    StoreUnavailable,
    /// `ModelGateway::stream` returned `Err(...)`.
    ModelGatewayFailed {
        /// Human-readable error from the gateway.
        message: String,
    },
    /// A panic was caught by Layer 2 (`TurnDriver` task). See spec §9.
    TurnPanicked {
        /// Location string captured from the panic payload.
        location: String,
    },
    /// `tokio::time::timeout` fired around the turn task.
    TurnTimedOut,
    /// An H09 hook returned `HookDecision::Reject`.
    HookRejected {
        /// Name of the hook that rejected the turn.
        hook_name: String,
        /// Human-readable rejection message.
        message: String,
    },
    /// Resume re-validation failed — the persisted log references tools
    /// or schemas that are no longer compatible with the running build.
    /// See `cogito-core::harness::resume::ResumeError` for the protocol-side
    /// equivalent.
    ResumeFailed {
        /// Human-readable description of the resume failure.
        message: String,
    },
}

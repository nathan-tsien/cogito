//! Real-time event stream broadcast to subscribers (TUI, observability,
//! consumer hooks).
//!
//! `StreamEvent` is distinct from `ConversationEvent`: it is *not* persisted,
//! text deltas are *not* batched, and slow subscribers may be dropped
//! (broadcast lagged semantics). See spec §7 for the dual-stream rationale.

use serde::{Deserialize, Serialize};

/// Real-time event observable via `SessionHandle::subscribe()`.
///
/// **Serde representation note**: internally-tagged with `tag = "kind"`.
/// All variants are unit or struct (no newtype-with-primitive), and no
/// inner field collides with the tag, so internal tagging is safe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamEvent {
    /// A new turn has begun. Carries no payload; the input is on the
    /// caller's side.
    TurnStarted,

    /// The turn paused on an async tool call. The driving Brain task
    /// has exited; the actor is now in `PausedOnJob`.
    TurnPaused,

    /// A previously paused turn has been resumed by a `JobCompleted` event.
    TurnResumed,

    /// The turn was cancelled by `SessionHandle::cancel_turn`.
    TurnCancelled,

    /// The turn reached terminal Completed state (model returned
    /// `stop_reason` = `end_turn` without further tool calls).
    TurnCompleted,

    /// The turn ended with a structured failure. `reason` is a
    /// human-readable rendering of `TurnFailureReason` for subscribers
    /// (the precise enum lives in the persisted `ConversationEvent`).
    TurnFailed {
        /// Human-readable description of the failure.
        reason: String,
    },

    /// Per-chunk text delta from the model stream. Not persisted as-is;
    /// the store writer subtask batches into `AssistantMessageAppended`
    /// every 200ms or 500 chars.
    TextDelta {
        /// The text chunk emitted by the model.
        chunk: String,
    },

    /// H08 began dispatching a tool call.
    ToolDispatchStarted {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
    },

    /// H08 finished dispatching a tool call. `ok` is false for both
    /// structured errors and panics; subscribers consult the persisted
    /// `ToolResult` for detail.
    ToolDispatchEnded {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Whether the invocation succeeded.
        ok: bool,
    },
}

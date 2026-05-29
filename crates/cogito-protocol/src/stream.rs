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
    /// A new turn has begun. The input is on the caller's side.
    TurnStarted {
        /// Set when this event is forwarded from a subagent's stream,
        /// naming the parent `delegate` call. `None` for the parent's own
        /// turns. (ADR-0011 observability bridge.)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// The turn paused on an async tool call. The driving Brain task
    /// has exited; the actor is now in `PausedOnJob`.
    TurnPaused,

    /// A previously paused turn has been resumed by a `JobCompleted` event.
    TurnResumed,

    /// The turn was cancelled by `SessionHandle::cancel_turn`.
    TurnCancelled,

    /// The turn reached terminal Completed state (model returned
    /// `stop_reason` = `end_turn` without further tool calls).
    TurnCompleted {
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// The turn ended with a structured failure. `reason` is a
    /// human-readable rendering of `TurnFailureReason` for subscribers
    /// (the precise enum lives in the persisted `ConversationEvent`).
    TurnFailed {
        /// Human-readable description of the failure.
        reason: String,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// Per-chunk text delta from the model stream. Not persisted as-is;
    /// the store writer subtask batches into `AssistantMessageAppended`
    /// every 200ms or 500 chars.
    TextDelta {
        /// The text chunk emitted by the model.
        chunk: String,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// Per-chunk reasoning delta from the model stream. Not persisted
    /// as-is; the store writer batches into `ThinkingBlockRecorded`
    /// at the wire-protocol block-completion boundary. See ADR-0019 §3.
    ThinkingDelta {
        /// The reasoning chunk emitted by the model.
        chunk: String,
    },

    /// H06 detected a `$<registered>` sigil outside code blocks. Broadcast
    /// only — NOT persisted. Subscribers (REPL, TUI) surface this for live
    /// feedback; the authoritative activation lands as
    /// `EventPayload::SkillActivated` in the next turn's H11 pass.
    SkillActivationRequested {
        /// The bare skill name (or `<plugin_id>:<name>`) detected.
        skill_name: String,
    },

    /// H08 began dispatching a tool call.
    ToolDispatchStarted {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Tool arguments serialized as a JSON value. Mirrors the
        /// persisted `ToolUseRecorded.args` payload; carried on the
        /// stream so non-persisted subscribers (REPL renderer, future
        /// TUI) can surface what the model is invoking without also
        /// reading the JSONL log. Renderers choose any truncation /
        /// highlighting policy.
        args: serde_json::Value,
    },

    /// H08 finished dispatching a tool call. `ok` is false for both
    /// structured errors and panics; subscribers consult the persisted
    /// `ToolResult` for the structured `ToolErrorKind`.
    ToolDispatchEnded {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Whether the invocation succeeded.
        ok: bool,
        /// Human-readable error message, populated iff `ok == false`.
        /// Mirrors `ToolResult::Error.message`; carried on the stream
        /// so subscribers can display the failure reason without
        /// reading the JSONL log.
        error_message: Option<String>,
    },
}

#[cfg(test)]
mod thinking_stream_tests {
    use super::*;

    #[test]
    fn thinking_delta_roundtrips() -> serde_json::Result<()> {
        let evt = StreamEvent::ThinkingDelta {
            chunk: "thinking...".into(),
        };
        let json = serde_json::to_string(&evt)?;
        assert!(
            json.contains(r#""kind":"thinking_delta""#),
            "tag missing: {json}"
        );
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }
}

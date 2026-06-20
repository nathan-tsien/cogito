//! Real-time event stream broadcast to subscribers (TUI, observability,
//! consumer hooks).
//!
//! `StreamEvent` is distinct from `ConversationEvent`: it is *not* persisted,
//! text deltas are *not* batched, and slow subscribers may be dropped
//! (broadcast lagged semantics). See spec §7 for the dual-stream rationale.

use serde::{Deserialize, Serialize};

use crate::gateway::StopReason;
use crate::ids::{MessageId, TurnId};

/// Real-time event observable via `SessionHandle::subscribe()`.
///
/// **Serde representation note**: internally-tagged with `tag = "kind"`.
/// All variants are unit or struct (no newtype-with-primitive), and no
/// inner field collides with the tag, so internal tagging is safe.
///
/// **Message correlation** (ADR-0041): an assistant message (one model call's
/// output) opens with [`StreamEvent::AssistantMessageStarted`], which carries a
/// freshly-minted `message_id`. The same `message_id` rides on the delta/tool
/// events that compose the message (`TextDelta`, `ThinkingDelta`,
/// `ToolDispatchStarted`/`Ended`) and is also stamped on those events'
/// *persisted* counterparts, so a live subscriber and a history reader derive
/// the same per-message identity (reconnect dedup, in-flight upsert). The
/// message id is opaque — it encodes no role or turn structure.
///
/// **Turn correlation**: every turn-scoped variant also carries an optional
/// `turn_id` — the id of the turn it belongs to — as an auxiliary turn-linkage
/// field, so a subscriber can attribute a streamed event to its turn in the
/// persisted log (which already carries `turn_id`). It is *not* a per-message
/// identity (a turn holds several messages); use `message_id` for that.
///
/// Both `turn_id` and the delta-event `message_id` are additive, serde-defaulted,
/// and broadcast-only: `None` only on replay-reconstructed or legacy events,
/// never on a live broadcast. Same additive-optional pattern as
/// `subagent_call_id` / `TurnCompleted.stop_reason` (ADR-0007, ADR-0040, ADR-0041).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamEvent {
    /// A new turn has begun. The input is on the caller's side.
    TurnStarted {
        /// The id of the turn this event opens. Lets a subscriber learn the
        /// turn id at turn open and attribute all subsequent deltas to it.
        /// See the type-level "Turn correlation" note (ADR-0041).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// Set when this event is forwarded from a subagent's stream,
        /// naming the parent `delegate` call. `None` for the parent's own
        /// turns. (ADR-0011 observability bridge.)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// The turn paused on an async tool call. The driving Brain task
    /// has exited; the actor is now in `PausedOnJob`.
    TurnPaused {
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
    },

    /// A previously paused turn has been resumed by a `JobCompleted` event.
    TurnResumed {
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
    },

    /// The turn was cancelled by `SessionHandle::cancel_turn`.
    TurnCancelled {
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
    },

    /// The turn reached terminal `Completed` state (no further tool calls).
    ///
    /// `Completed` does not imply the model finished cleanly: a turn whose
    /// final model call was cut off by `max_tokens` also lands here, carrying
    /// truncated text as the final answer. Inspect `stop_reason` to tell the
    /// cases apart (ADR-0040).
    TurnCompleted {
        /// The turn's terminal stop reason — the final model call's
        /// `stop_reason`. `Some(StopReason::MaxTokens)` flags a truncated
        /// turn, letting a live subscriber detect truncation at the turn
        /// boundary without scanning `ModelCallCompleted` events. `None` only
        /// on replay-reconstructed or legacy events (ADR-0040).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<StopReason>,
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
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
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },

    /// An assistant message has opened — one model call's output begins.
    /// Carries the freshly-minted `message_id` so a subscriber can fold the
    /// following deltas into one message and key it to the same id the
    /// persisted log uses. Broadcast at the model-call-start boundary
    /// (ADR-0041).
    AssistantMessageStarted {
        /// Stable per-message identity (one model call = one message).
        message_id: MessageId,
        /// See the type-level "Turn correlation" note.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
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
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
        /// The message this delta composes (see `AssistantMessageStarted`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },

    /// Per-chunk reasoning delta from the model stream. Not persisted
    /// as-is; the store writer batches into `ThinkingBlockRecorded`
    /// at the wire-protocol block-completion boundary. See ADR-0019 §3.
    ThinkingDelta {
        /// The reasoning chunk emitted by the model.
        chunk: String,
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// The message this delta composes (see `AssistantMessageStarted`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },

    /// H06 detected a `$<registered>` sigil outside code blocks. Broadcast
    /// only — NOT persisted. Subscribers (REPL, TUI) surface this for live
    /// feedback; the authoritative activation lands as
    /// `EventPayload::SkillActivated` in the next turn's H11 pass.
    SkillActivationRequested {
        /// The bare skill name (or `<plugin_id>:<name>`) detected.
        skill_name: String,
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
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
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// The message whose `tool_use` block this dispatches (see
        /// `AssistantMessageStarted`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
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
        /// See `TurnStarted::turn_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        /// The message whose `tool_use` block this dispatched (see
        /// `AssistantMessageStarted`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },
}

#[cfg(test)]
mod thinking_stream_tests {
    use super::*;

    #[test]
    fn thinking_delta_roundtrips() -> serde_json::Result<()> {
        let evt = StreamEvent::ThinkingDelta {
            chunk: "thinking...".into(),
            turn_id: None,
            message_id: None,
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

#[cfg(test)]
mod turn_id_correlation_tests {
    use super::*;
    use crate::ids::TurnId;

    #[test]
    fn turn_started_carries_turn_id_roundtrip() -> serde_json::Result<()> {
        let turn_id = TurnId::new();
        let evt = StreamEvent::TurnStarted {
            turn_id: Some(turn_id),
            subagent_call_id: None,
        };
        let json = serde_json::to_string(&evt)?;
        assert!(
            json.contains("turn_id"),
            "turn_id must be present on the wire: {json}"
        );
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }

    #[test]
    fn turn_id_none_is_omitted_from_wire() -> serde_json::Result<()> {
        // Backward compatibility: a `None` correlation key serializes away,
        // matching the additive-optional precedent of `subagent_call_id`
        // and `stop_reason` (ADR-0007 / ADR-0040).
        let evt = StreamEvent::TextDelta {
            chunk: "hi".into(),
            turn_id: None,
            subagent_call_id: None,
            message_id: None,
        };
        let json = serde_json::to_string(&evt)?;
        assert!(
            !json.contains("turn_id"),
            "turn_id: None must be skipped: {json}"
        );
        Ok(())
    }

    #[test]
    fn legacy_event_without_turn_id_deserializes_to_none() -> serde_json::Result<()> {
        // A producer predating this field omits `turn_id`; consumers must
        // still parse it (serde default → None).
        let legacy =
            r#"{"kind":"tool_dispatch_ended","call_id":"c1","ok":true,"error_message":null}"#;
        let back: StreamEvent = serde_json::from_str(legacy)?;
        assert_eq!(
            back,
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
                turn_id: None,
                message_id: None,
            }
        );
        Ok(())
    }
}

#[cfg(test)]
mod message_id_tests {
    use super::*;
    use crate::ids::{MessageId, TurnId};

    #[test]
    fn assistant_message_started_carries_message_id_roundtrip() -> serde_json::Result<()> {
        let message_id = MessageId::new();
        let turn_id = TurnId::new();
        let evt = StreamEvent::AssistantMessageStarted {
            message_id,
            turn_id: Some(turn_id),
            subagent_call_id: None,
        };
        let json = serde_json::to_string(&evt)?;
        assert!(
            json.contains(r#""kind":"assistant_message_started""#),
            "tag missing: {json}"
        );
        assert!(json.contains("message_id"), "message_id missing: {json}");
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }

    #[test]
    fn text_delta_carries_message_id_and_omits_when_none() -> serde_json::Result<()> {
        let message_id = MessageId::new();
        let with = StreamEvent::TextDelta {
            chunk: "hi".into(),
            turn_id: None,
            subagent_call_id: None,
            message_id: Some(message_id),
        };
        let json = serde_json::to_string(&with)?;
        assert!(json.contains("message_id"), "message_id must ride: {json}");
        assert_eq!(serde_json::from_str::<StreamEvent>(&json)?, with);

        let without = StreamEvent::TextDelta {
            chunk: "hi".into(),
            turn_id: None,
            subagent_call_id: None,
            message_id: None,
        };
        let json_none = serde_json::to_string(&without)?;
        assert!(
            !json_none.contains("message_id"),
            "message_id: None must be skipped: {json_none}"
        );
        Ok(())
    }
}

//! Persisted event log shape. See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §2 for the design rationale.
//!
//! `ConversationEvent` is the persistent counterpart to `StreamEvent` (the
//! live broadcast). They are intentionally different types — see ADR-0006 §7.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::content::ContentBlock;
use crate::ids::{EventId, SessionId, TurnId};
use crate::job::{JobId, JobOutcome};
use crate::session::SessionMeta;
use crate::tool::ToolResult;
use crate::turn::{TurnFailureReason, TurnOutcome};

/// Schema version emitted by this build of cogito. Bumped together with
/// every breaking change to `ConversationEvent` or `EventPayload`. See
/// ADR-0005 §4 #2 and ADR-0007 for compatibility rules.
pub const SCHEMA_VERSION: u32 = 1;

/// One persisted entry in a conversation's event log.
///
/// Envelope fields are at the JSON top level. The variant-specific payload
/// is adjacently tagged with `tag = "type"` / `content = "data"`, flattened
/// into the envelope.
///
/// Note: `Eq` is deliberately not derived because several payload variants
/// transitively carry `serde_json::Value` (via `ToolResult`, `SessionMeta`,
/// `JobOutcome`), which does not implement `Eq`. This mirrors the rationale
/// recorded on [`crate::content::ContentBlock`] and [`crate::session::SessionMeta`].
///
/// `JsonSchema` is derived so `cogito-gen-schema` can call
/// `schema_for!(ConversationEvent)` to emit the canonical
/// `docs/schemas/conversation-event-v1.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConversationEvent {
    /// Schema version of the envelope and payload.
    pub schema_version: u32,

    /// Globally unique, monotonic-per-process event identifier.
    pub event_id: EventId,

    /// Session this event belongs to.
    pub session_id: SessionId,

    /// Turn this event belongs to. `None` for session-level events.
    pub turn_id: Option<TurnId>,

    /// Monotonic per-session sequence number. First event has `seq = 0`.
    pub seq: u64,

    /// Wall-clock timestamp at recorder serialization time.
    pub ts: DateTime<Utc>,

    /// Variant-specific payload.
    #[serde(flatten)]
    pub payload: EventPayload,
}

/// The variant-specific payload of a `ConversationEvent`.
///
/// Note: `Eq` is deliberately not derived; see the rationale on
/// [`ConversationEvent`]. `JsonSchema` is derived to support schema-gen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventPayload {
    /// First event of every session.
    SessionStarted {
        /// Session-level metadata.
        meta: SessionMeta,
    },

    /// A new turn has begun.
    TurnStarted {
        /// User input that triggered this turn.
        user_input: Vec<ContentBlock>,
    },

    /// One content block of assistant text has been fully emitted.
    AssistantMessageAppended {
        /// Full text of the completed content block.
        text: String,
    },

    /// The model emitted a `tool_use` content block.
    ToolUseRecorded {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Tool arguments as JSON.
        args: serde_json::Value,
    },

    /// H08 returned a `ToolResult` for a previously recorded call.
    ToolResultRecorded {
        /// Identifier matching the originating `ToolUseRecorded.call_id`.
        call_id: String,
        /// The tool result.
        result: ToolResult,
    },

    /// The turn paused on an async tool call.
    TurnPaused {
        /// Identifier of the async job being awaited.
        job_id: JobId,
    },

    /// An async job that previously paused this turn has finished.
    JobCompletedRecorded {
        /// Identifier of the completed job.
        job_id: JobId,
        /// Terminal outcome of the job (success/failed/cancelled).
        outcome: JobOutcome,
    },

    /// The turn reached terminal Completed state.
    TurnCompleted {
        /// Outcome detail.
        outcome: TurnOutcome,
    },

    /// The turn ended in failure.
    TurnFailed {
        /// Structured failure reason.
        reason: TurnFailureReason,
    },

    /// Recorded at the start of the `Init -> ContextManaged` transition.
    /// v0.1 ships an immediate companion `ContextManageCompleted` because
    /// H11 is a pass-through; ADR-0008 will replace the body with real
    /// context decisions.
    ContextManageEntered {
        /// Turn this event belongs to.
        turn_id: TurnId,
    },

    /// Recorded at the end of the `ContextManaged -> PromptBuilt`
    /// transition. v0.1 pass-through carries no decision body.
    ContextManageCompleted {
        /// Turn this event belongs to.
        turn_id: TurnId,
    },

    /// Recorded after H04 composes the prompt and H05 builds the tool
    /// surface. Carries metadata only — the full prompt is NOT persisted
    /// (event log is a state-recovery source, not a prompt cache; see
    /// ADR-0007).
    PromptComposed {
        /// Turn this event belongs to.
        turn_id: TurnId,
        /// Provider model identifier used for this call.
        model: String,
        /// Number of tool descriptors in the surface.
        surface_size: u32,
    },

    /// Recorded at the start of the `PromptBuilt -> ModelCalling`
    /// transition (right before the gateway stream opens).
    ModelCallStarted {
        /// Turn this event belongs to.
        turn_id: TurnId,
        /// Provider model identifier.
        model: String,
    },
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unreachable)]
mod tests {
    use super::*;

    fn sample_envelope(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: Some(TurnId::new()),
            seq: 0,
            ts: Utc::now(),
            payload,
        }
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn envelope_uses_adjacent_tagging_in_json() -> serde_json::Result<()> {
        let event = sample_envelope(EventPayload::AssistantMessageAppended { text: "hi".into() });
        let json = serde_json::to_string(&event)?;
        // Envelope keys appear at top level; payload is `type` + `data`.
        assert!(
            json.contains(r#""schema_version":1"#),
            "missing schema_version: {json}"
        );
        assert!(
            json.contains(r#""type":"assistant_message_appended""#),
            "missing tag: {json}"
        );
        assert!(
            json.contains(r#""data":{"text":"hi"}"#),
            "missing data body: {json}"
        );
        Ok(())
    }

    #[test]
    fn all_nine_variants_roundtrip() -> serde_json::Result<()> {
        // JobOutcome has no Default impl; use the simplest unit variant
        // (`Cancelled`) as a representative for the JobCompletedRecorded
        // fixture. The full matrix of JobOutcome variants is exercised in
        // job.rs's own tests.
        let variants = vec![
            EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    ..Default::default()
                },
            },
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text { text: "go".into() }],
            },
            EventPayload::AssistantMessageAppended { text: "ok".into() },
            EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({"p": 1}),
            },
            EventPayload::ToolResultRecorded {
                call_id: "c1".into(),
                result: ToolResult::text("out"),
            },
            EventPayload::TurnPaused {
                job_id: JobId::default(),
            },
            EventPayload::JobCompletedRecorded {
                job_id: JobId::default(),
                outcome: JobOutcome::Cancelled,
            },
            EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
            EventPayload::TurnFailed {
                reason: TurnFailureReason::TurnTimedOut,
            },
        ];
        for v in variants {
            let event = sample_envelope(v.clone());
            let json = serde_json::to_string(&event)?;
            let back: ConversationEvent = serde_json::from_str(&json)?;
            assert_eq!(event, back, "variant {v:?} did not roundtrip");
        }
        Ok(())
    }

    #[test]
    fn session_started_carries_no_turn_id() -> serde_json::Result<()> {
        // SessionStarted is session-level; turn_id should be None in idiomatic
        // usage. Serde permits any value here, but assert the canonical shape.
        let event = ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: None,
            seq: 0,
            ts: Utc::now(),
            payload: EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    ..Default::default()
                },
            },
        };
        let json = serde_json::to_string(&event)?;
        // `turn_id` is `Option<TurnId>` — when None, serde-json emits `null`
        // (no `skip_serializing_if`). Assert that to lock the wire shape.
        assert!(
            json.contains(r#""turn_id":null"#),
            "turn_id wire shape changed: {json}"
        );
        let back: ConversationEvent = serde_json::from_str(&json)?;
        assert_eq!(event, back);
        Ok(())
    }

    #[test]
    fn non_exhaustive_keeps_match_arms_safe() {
        // Compile-time check: external code cannot exhaustively match
        // EventPayload. Inside the crate we can still match, but the
        // wildcard arm is required by `#[non_exhaustive]` for downstream
        // consumers — exercise that shape here.
        let p = EventPayload::TurnCompleted {
            outcome: TurnOutcome::Completed,
        };
        match p {
            EventPayload::TurnCompleted { .. } => {}
            _ => unreachable!("wrong variant for fixture"),
        }
    }
}

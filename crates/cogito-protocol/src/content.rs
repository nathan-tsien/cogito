//! `ContentBlock` — the wire-format unit shared between models, tools,
//! and persisted events. v0.1 covers `Text` / `ToolUse` / `ToolResult`.
//! `Image` and other multimodal variants land in v0.2 (ADR-0007 storage
//! spec).

use serde::{Deserialize, Serialize};

use crate::tool::ToolResult;

/// One unit of content as defined by the Anthropic / `OpenAI` wire formats.
///
/// Adjacently-tagged (`tag = "type", content = "data"`) for forward
/// compatibility: newtype-with-sequence bodies are allowed (unlike
/// internal tagging), and new variants can be added without bumping
/// `SCHEMA_VERSION` (see ADR-0007).
///
/// Note: `PartialEq` is derived but not `Eq` because the `ToolUse.args`
/// field and the embedded `ToolResult` carry `serde_json::Value`, which
/// does not implement `Eq`. This mirrors the rationale in
/// [`crate::tool::ToolResult`].
///
/// `JsonSchema` is derived for schema-gen (Plan 2 Task 11). `args`, a
/// raw `serde_json::Value`, uses schemars's built-in impl which renders
/// as an open `true` schema (any JSON allowed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentBlock {
    /// Plain assistant or user text.
    Text {
        /// The text content.
        text: String,
    },
    /// Model-issued tool call.
    ToolUse {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Tool arguments as JSON.
        args: serde_json::Value,
    },
    /// Result fed back to the model for a previously-issued tool call.
    ToolResult {
        /// Identifier matching the originating `ToolUse.call_id`.
        call_id: String,
        /// Structured result.
        result: ToolResult,
    },
    /// Model reasoning/"thinking" content. Carried inside the assistant
    /// message's content array; placed before Text / `ToolUse` blocks in
    /// the same message per provider requirements (see ADR-0019 §4).
    /// `provider_opaque` is None for backends that need no round-trip
    /// material (OpenAI-compat), Some for Anthropic (signature) and
    /// `OpenAI` Responses (`encrypted_content` + `item_id`).
    Thinking {
        /// Human-readable reasoning text. May be empty when the provider
        /// returns only an encrypted blob (e.g. Anthropic `redacted_thinking`).
        text: String,
        /// Provider-defined opaque payload required for next-turn
        /// validation. Schema is provider-specific; cogito does not
        /// interpret the contents.
        provider_opaque: Option<serde_json::Value>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&cb)?;
        assert_eq!(json, r#"{"type":"text","data":{"text":"hello"}}"#);
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn tool_use_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::ToolUse {
            call_id: "toolu_01".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({"path": "/tmp/x"}),
        };
        let json = serde_json::to_string(&cb)?;
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn tool_result_carrying_sequence_body_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::ToolResult {
            call_id: "toolu_01".into(),
            result: ToolResult::Output(vec![serde_json::json!({
                "type": "text",
                "data": {"text": "file contents"}
            })]),
        };
        let json = serde_json::to_string(&cb)?;
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn thinking_roundtrips_with_provider_opaque() -> serde_json::Result<()> {
        let cb = ContentBlock::Thinking {
            text: "let me think...".into(),
            provider_opaque: Some(serde_json::json!({"signature": "abc123"})),
        };
        let json = serde_json::to_string(&cb)?;
        assert_eq!(
            json,
            r#"{"type":"thinking","data":{"text":"let me think...","provider_opaque":{"signature":"abc123"}}}"#
        );
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn thinking_roundtrips_without_provider_opaque() -> serde_json::Result<()> {
        let cb = ContentBlock::Thinking {
            text: "implicit reasoning".into(),
            provider_opaque: None,
        };
        let json = serde_json::to_string(&cb)?;
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }
}

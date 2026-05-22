//! Wire-level DTOs for Anthropic Messages API. Only fields cogito needs
//! are modeled; unknown fields are ignored.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Request {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    pub stream: bool,
    pub system: String,
    pub messages: Vec<RequestMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<RequestTool>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestMessage {
    pub role: String,
    pub content: Vec<RequestContentBlock>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RequestContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// Anthropic plain thinking block (extended-thinking mode). Must
    /// round-trip with the original `signature` for next-turn
    /// validation to succeed.
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        signature: String,
    },
    /// Anthropic safety-filtered thinking block. The reasoning text is
    /// encrypted; only the opaque `data` blob travels.
    RedactedThinking {
        data: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// SSE event data shapes we recognize.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum SseEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: SseMessageStart },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: SseContentBlockStart,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u32,
        delta: SseContentBlockDelta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: SseMessageDelta,
        usage: SseUsage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: SseError },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SseMessageStart {
    pub usage: SseUsage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SseContentBlockStart {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Anthropic thinking block start (extended-thinking mode).
    /// Body fields arrive in subsequent `thinking_delta` and
    /// `signature_delta` deltas; the block is sealed at
    /// `content_block_stop` for this index.
    Thinking {},
    /// Anthropic safety-filtered reasoning block. Carries an opaque
    /// `data` blob; no further deltas follow — `content_block_stop`
    /// for this index arrives immediately.
    RedactedThinking {
        #[allow(dead_code)]
        data: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// All variants ARE deltas (mirror of Anthropic's `content_block_delta`
// SSE event family). The `Delta` suffix carries semantics; stripping it
// would conflict with non-delta variants like the start types.
#[allow(clippy::enum_variant_names)]
pub(crate) enum SseContentBlockDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    /// One streamed chunk of the in-flight thinking block's text.
    ThinkingDelta {
        #[allow(dead_code)]
        thinking: String,
    },
    /// Signature for the in-flight thinking block. Arrives once,
    /// immediately before the corresponding `content_block_stop`.
    SignatureDelta {
        #[allow(dead_code)]
        signature: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SseMessageDelta {
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub(crate) struct SseUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SseError {
    pub message: String,
}

#[cfg(test)]
#[allow(clippy::panic)]
mod thinking_wire_tests {
    use super::*;

    #[test]
    fn deserializes_thinking_delta() {
        let json = r#"{"type":"thinking_delta","thinking":"I should grep."}"#;
        let d: SseContentBlockDelta = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("deserialize failed: {e}; input: {json}"));
        #[allow(clippy::panic)]
        match d {
            SseContentBlockDelta::ThinkingDelta { thinking } => {
                assert_eq!(thinking, "I should grep.");
            }
            other => panic!("expected ThinkingDelta, got {other:?}"),
        }
    }

    #[test]
    fn deserializes_signature_delta() {
        let json = r#"{"type":"signature_delta","signature":"sig_xyz"}"#;
        let d: SseContentBlockDelta = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("deserialize failed: {e}; input: {json}"));
        #[allow(clippy::panic)]
        match d {
            SseContentBlockDelta::SignatureDelta { signature } => {
                assert_eq!(signature, "sig_xyz");
            }
            other => panic!("expected SignatureDelta, got {other:?}"),
        }
    }

    #[test]
    fn deserializes_thinking_block_start() {
        let json = r#"{"type":"thinking"}"#;
        let s: SseContentBlockStart = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("deserialize failed: {e}; input: {json}"));
        assert!(matches!(s, SseContentBlockStart::Thinking {}));
    }

    #[test]
    fn deserializes_redacted_thinking_block_start() {
        let json = r#"{"type":"redacted_thinking","data":"enc_blob"}"#;
        let s: SseContentBlockStart = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("deserialize failed: {e}; input: {json}"));
        #[allow(clippy::panic)]
        match s {
            SseContentBlockStart::RedactedThinking { data } => {
                assert_eq!(data, "enc_blob");
            }
            other => panic!("expected RedactedThinking, got {other:?}"),
        }
    }
}

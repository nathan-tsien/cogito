//! Wire-level DTOs for the `OpenAI` Chat Completions API (streaming path).
//!
//! These types are `pub(crate)` only — callers interact with the gateway
//! through the `ModelGateway` trait, never directly with wire types.

use serde::{Deserialize, Serialize};

/// Top-level request body for `POST /chat/completions`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Request {
    /// Provider-specific model identifier, e.g. `"meta-llama/Llama-3.1-70B"`.
    pub model: String,
    /// Dialogue history in wire order.
    pub messages: Vec<RequestMessage>,
    /// Hard cap on output tokens.
    pub max_tokens: u32,
    /// Sampling temperature; omitted if `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top-p nucleus sampling; omitted if `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Stop sequences; omitted if empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    /// Always `true` for streaming.
    pub stream: bool,
    /// Tool definitions; omitted if empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<RequestTool>,
}

/// A single message in the Chat Completions dialogue.
///
/// `role` is one of `"system"`, `"user"`, `"assistant"`, or `"tool"`.
/// The `tool_call_id` field is only set for `role: "tool"` messages.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestMessage {
    /// Dialogue participant role.
    pub role: String,
    /// Text content; omitted for assistant messages that carry `tool_calls` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// For `role: "tool"` messages — the call id this result belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For `role: "assistant"` messages that issue tool calls.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// A model-issued tool call inside an assistant message.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolCall {
    /// Opaque identifier assigned by the model.
    pub id: String,
    /// Always `"function"` in v1.
    #[serde(rename = "type")]
    pub kind: String,
    /// Function name + JSON-encoded arguments.
    pub function: ToolCallFunction,
}

/// Function name + JSON-encoded arguments for a single tool call.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolCallFunction {
    /// Tool name, e.g. `"read_file"`.
    pub name: String,
    /// Arguments as a JSON-encoded string (the API contract).
    pub arguments: String,
}

/// A tool definition sent in the request.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestTool {
    /// Always `"function"` in v1.
    #[serde(rename = "type")]
    pub kind: String,
    /// Tool description and JSON Schema for its parameters.
    pub function: ToolDef,
}

/// Tool description and parameter schema.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolDef {
    /// Tool name.
    pub name: String,
    /// One-line description shown to the model.
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub parameters: serde_json::Value,
}

// --- Streaming response types ---

/// One chunk in the SSE stream, containing zero or more choices.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StreamChunk {
    /// Response choices; typically one element for non-batch calls.
    #[serde(default)]
    pub choices: Vec<Choice>,
}

/// One response choice inside a `StreamChunk`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Choice {
    /// Partial content delta for this chunk.
    #[serde(default)]
    pub delta: ChoiceDelta,
    /// Set in the final chunk for this choice; signals why generation stopped.
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Incremental content carried by a streaming choice.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ChoiceDelta {
    /// Role hint — only present in the first chunk; consumed during deserialization only.
    #[serde(default)]
    #[allow(dead_code)]
    pub role: Option<String>,
    /// Partial text content from the assistant.
    #[serde(default)]
    pub content: Option<String>,
    /// Partial reasoning content (`DeepSeek` official API, vLLM
    /// `--enable-reasoning`, etc.). Mutually exclusive in time with
    /// `content` per ADR-0019 §5.3.b — reasoning streams first, then
    /// the backend switches to `content`. See `decode.rs` (Task 13)
    /// for the state machine.
    #[serde(default)]
    #[allow(dead_code)]
    pub reasoning_content: Option<String>,
    /// Partial tool-call fragments; keyed by `index` for multi-call merging.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallDelta>,
}

/// Streaming fragment of a single tool call.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ToolCallDelta {
    /// Stream-level index (NOT the `block_index` we synthesize).
    pub index: u32,
    /// Call id — present only in the first fragment for this index.
    #[serde(default)]
    pub id: Option<String>,
    /// Function name + partial JSON arguments.
    #[serde(default)]
    pub function: Option<ToolCallFunctionDelta>,
}

/// Partial function name and/or arguments for a streaming tool call.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ToolCallFunctionDelta {
    /// Tool name — present only in the first fragment.
    #[serde(default)]
    pub name: Option<String>,
    /// Partial JSON arguments string to be concatenated across fragments.
    #[serde(default)]
    pub arguments: Option<String>,
}

#[cfg(test)]
mod reasoning_content_tests {
    use super::*;

    #[test]
    fn deserializes_delta_with_reasoning_content() {
        let json = r#"{"reasoning_content":"I should grep."}"#;
        let d: ChoiceDelta = serde_json::from_str(json).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("deserialize failed: {e}")
            }
        });
        assert_eq!(d.reasoning_content.as_deref(), Some("I should grep."));
        assert!(d.content.is_none());
        assert!(d.tool_calls.is_empty());
    }

    #[test]
    fn deserializes_delta_without_reasoning_content_stays_compatible() {
        let json = r#"{"content":"hi"}"#;
        let d: ChoiceDelta = serde_json::from_str(json).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("deserialize failed: {e}")
            }
        });
        assert_eq!(d.content.as_deref(), Some("hi"));
        assert!(d.reasoning_content.is_none());
    }

    #[test]
    fn deserializes_delta_with_both_content_and_reasoning() {
        // Some backends may emit both in one chunk (transition frame).
        let json = r#"{"reasoning_content":"thinking","content":"output"}"#;
        let d: ChoiceDelta = serde_json::from_str(json).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("deserialize failed: {e}")
            }
        });
        assert_eq!(d.reasoning_content.as_deref(), Some("thinking"));
        assert_eq!(d.content.as_deref(), Some("output"));
    }
}

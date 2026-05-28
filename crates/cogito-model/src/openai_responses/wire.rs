//! Wire-protocol types for the `OpenAI` Responses API.
//!
//! Reference: <https://platform.openai.com/docs/api-reference/responses>
//!
//! We model only the subset cogito uses: messages + function tools +
//! streaming text + reasoning summary items + stop reasons + usage.
//!
//! Types are `pub(crate)` only — callers go through `ModelGateway`,
//! never the wire DTOs.

// Scaffold (Task 11) declares all wire types ahead of the encoder
// (Task 12) and decoder (Task 13) that consume them. The encoder lands
// in the very next commit and the decoder right after; both populate
// every variant/field below.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Top-level POST body for `/responses`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesRequest {
    /// Provider-specific model identifier, e.g. `"o1-preview"`.
    pub model: String,
    /// Flat top-level input array; user/assistant turns, function calls,
    /// function-call outputs, and re-fed reasoning items mix together.
    pub input: Vec<InputItem>,
    /// Always `true` for our streaming path.
    pub stream: bool,
    /// Hard cap on output tokens; omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Sampling temperature; omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top-p nucleus sampling; omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Function-tool definitions; omitted when empty.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDef>,
    /// `reasoning` block; only set when an effort is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningParams>,
    /// System prompt; serialized as `instructions` (Responses convention).
    /// Omitted when the system prompt is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `reasoning.effort` toggle. Public to allow consumers to set it via
/// `OpenAiResponsesConfig.reasoning_effort` and via the
/// `provider.<name>.reasoning_effort` configuration key.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Minimal additional reasoning budget.
    Low,
    /// Provider-recommended default reasoning budget.
    Medium,
    /// Highest reasoning budget the model supports.
    High,
}

/// `reasoning` parameters bundle. Currently carries only `effort`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReasoningParams {
    /// Reasoning effort level.
    pub effort: ReasoningEffort,
}

/// One input item.
///
/// The Responses API uses a flat top-level array: user/assistant turns
/// become `message` items, tool calls become `function_call` items,
/// tool results become `function_call_output` items, and prior thinking
/// becomes `reasoning` items per ADR-0019 §5.4.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum InputItem {
    /// `{role, content}` message item.
    Message {
        /// One of `"user"`, `"assistant"`, `"system"`.
        role: String,
        /// One or more text parts (`input_text` / `output_text`).
        content: Vec<MessageContent>,
    },
    /// Model-issued tool call.
    FunctionCall {
        /// Opaque call identifier matching the outgoing tool result.
        call_id: String,
        /// Tool (function) name.
        name: String,
        /// Arguments as a JSON-encoded string.
        arguments: String,
    },
    /// Tool result fed back to the model.
    FunctionCallOutput {
        /// Call identifier matching the originating `FunctionCall.call_id`.
        call_id: String,
        /// Tool output as a flat string.
        output: String,
    },
    /// Prior reasoning re-fed into the next turn (ADR-0019 §5.4).
    Reasoning {
        /// Reasoning summary parts; typically one `summary_text` element.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        summary: Vec<ReasoningSummary>,
    },
}

/// Per-part content inside a `Message` item.
///
/// Responses uses `input_text` for user/system content and `output_text`
/// for assistant content. We emit the variant appropriate for the
/// message's role.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MessageContent {
    /// User / system text part.
    InputText {
        /// The text content.
        text: String,
    },
    /// Assistant text part.
    OutputText {
        /// The text content.
        text: String,
    },
}

/// One `summary` element on a `Reasoning` item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReasoningSummary {
    /// Always `"summary_text"` in v1.
    #[serde(rename = "type")]
    pub kind: String,
    /// Human-readable summary text.
    pub text: String,
}

/// Function-tool definition sent in the request.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolDef {
    /// Always `"function"` in v1.
    #[serde(rename = "type")]
    pub kind: String,
    /// Tool name.
    pub name: String,
    /// One-line description shown to the model.
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub parameters: serde_json::Value,
}

// --- Streaming response events ---

/// One Responses SSE event. The `type` field discriminates.
///
/// Unknown event types fall through to `Other` so harmless additions
/// from `OpenAI` do not break the decoder.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamEvent {
    /// Incremental text chunk for an in-flight output item.
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        /// Partial text chunk.
        delta: String,
    },

    /// Final sealed text for an output item.
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        /// Full accumulated text.
        text: String,
    },

    /// Incremental reasoning summary text chunk.
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryDelta {
        /// Partial reasoning text chunk.
        delta: String,
    },

    /// Final sealed reasoning summary text.
    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryDone {
        /// Full accumulated reasoning text.
        text: String,
    },

    /// Incremental function-call arguments fragment.
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgsDelta {
        /// Identifier of the `function_call` output item being updated.
        item_id: String,
        /// Partial JSON arguments fragment.
        delta: String,
    },

    /// Final sealed function-call arguments string.
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgsDone {
        /// Identifier of the `function_call` output item being sealed.
        item_id: String,
        /// Complete JSON-encoded arguments string.
        arguments: String,
    },

    /// Header announcing a new output item (message / `function_call` /
    /// reasoning). Carries the `id` and metadata needed by the decoder
    /// to correlate subsequent delta events.
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        /// The new item's header.
        item: OutputItemHeader,
    },

    /// Final event for a successful completion.
    #[serde(rename = "response.completed")]
    Completed {
        /// Terminal response object (status / `incomplete_details` / usage).
        response: ResponseFinal,
    },

    /// Final event for a failed completion.
    #[serde(rename = "response.failed")]
    Failed {
        /// Terminal response object carrying the error details.
        response: ResponseFinal,
    },

    /// Catch-all for unknown event types.
    ///
    /// Decoded so the parser stays robust against additive provider
    /// changes (e.g. new `response.tool_call.*` shapes). Treated as a
    /// no-op by the decoder.
    #[serde(other)]
    Other,
}

/// Item-header payload for `response.output_item.added`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OutputItemHeader {
    /// Item id (assigned by the server).
    pub id: String,
    /// One of `"message"`, `"function_call"`, `"reasoning"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Function-call identifier (only for `function_call` items).
    #[serde(default)]
    pub call_id: Option<String>,
    /// Function (tool) name (only for `function_call` items).
    #[serde(default)]
    pub name: Option<String>,
}

/// Terminal `response` object carried by `Completed` / `Failed` events.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ResponseFinal {
    /// One of `"completed"`, `"incomplete"`, `"failed"`.
    #[serde(default)]
    pub status: Option<String>,
    /// Reason for incomplete responses (e.g. `max_output_tokens`).
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    /// Token usage for this response.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Error details for failed responses.
    #[serde(default)]
    pub error: Option<ResponseError>,
}

/// `incomplete_details` payload.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IncompleteDetails {
    /// Free-form reason string (e.g. `"max_output_tokens"`,
    /// `"content_filter"`).
    pub reason: String,
}

/// Token usage reported by the provider in the terminal event.
#[derive(Debug, Clone, Deserialize, Default)]
#[allow(clippy::struct_field_names)]
pub(crate) struct Usage {
    /// Tokens consumed by the input.
    #[serde(default)]
    pub input_tokens: u32,
    /// Tokens produced as output.
    #[serde(default)]
    pub output_tokens: u32,
    /// Sum of input + output (informational; cogito does not use it).
    #[serde(default)]
    pub total_tokens: u32,
}

/// Error payload carried by `response.failed`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ResponseError {
    /// Human-readable error message.
    pub message: String,
    /// Optional machine-readable error code.
    #[serde(default)]
    pub code: Option<String>,
}

//! `ModelGateway` and supporting value types.
//!
//! See:
//! - `docs/components/H06-stream-demux.md` for the consumer side (H06)
//! - `docs/superpowers/specs/2026-05-19-sprint-2-minimal-loop-design.md` §Q1
//!   for the gateway-pre-aggregation decision (X mode)
//! - ADR-0006 §"Sprint 2 protocol-layer additions" for the layer-rule rationale

use serde::{Deserialize, Serialize};

/// Model invocation parameters carried in `ModelInput.params`.
///
/// Field set is intentionally minimal in v0.1; provider adapters map only
/// what the wire format supports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelParams {
    /// Provider-specific model identifier, e.g. `"claude-opus-4-7"` or
    /// `"meta-llama/Llama-3.1-70B-Instruct"`.
    pub model: String,
    /// Hard cap on output tokens for this call.
    pub max_tokens: u32,
    /// Sampling temperature; `None` lets the provider default apply.
    pub temperature: Option<f32>,
    /// Top-p nucleus sampling; `None` lets the provider default apply.
    pub top_p: Option<f32>,
    /// Optional stop sequences. Empty vector means "none".
    #[serde(default)]
    pub stop_sequences: Vec<String>,
}

/// Why the model stopped emitting. Set as the last field on `ModelOutput`.
///
/// Marked `#[non_exhaustive]` because v0.x adapters may introduce new
/// reasons (e.g. `Refusal` from policy-aware providers); reserving the
/// variant set lets future additions stay additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    /// Model signaled normal turn end.
    EndTurn,
    /// Model emitted one or more `tool_use` blocks and yielded for results.
    ToolUse,
    /// Output reached `ModelParams.max_tokens`.
    MaxTokens,
    /// One of `ModelParams.stop_sequences` matched.
    StopSequence,
}

/// Token usage reported by the provider for one model call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Usage {
    /// Tokens consumed by the input (system + history + tool schemas).
    pub input_tokens: u32,
    /// Tokens produced as output.
    pub output_tokens: u32,
}

use crate::content::ContentBlock;
use crate::tool::ToolDescriptor;

/// A single message in the dialogue history passed to a model.
///
/// `Message` is provider-agnostic. The Anthropic adapter maps it 1:1 to
/// Anthropic Messages API; the `OpenAI` Chat Completions adapter splits
/// `ContentBlock::ToolResult` blocks inside `User` messages out into
/// independent `{role: "tool", ...}` wire messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    /// User message; may carry `Text`, `Image` (v0.2+), or `ToolResult` blocks.
    User {
        /// Content blocks comprising this message.
        content: Vec<ContentBlock>,
    },
    /// Assistant message; may carry `Text` and `ToolUse` blocks.
    Assistant {
        /// Content blocks comprising this message.
        content: Vec<ContentBlock>,
    },
}

/// Fully-formed input to `ModelGateway::stream`. Produced by H04 Prompt
/// Composer at the `ContextManaged → PromptBuilt` transition.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelInput {
    /// System prompt; may be empty.
    pub system: String,
    /// Dialogue history in canonical order (oldest first).
    pub messages: Vec<Message>,
    /// Tool descriptors the model is allowed to call this turn.
    /// Adapters serialize this list to the provider's tool-schema format.
    pub tools: Vec<ToolDescriptor>,
    /// Sampling parameters and model selection.
    pub params: ModelParams,
}

/// Provider-agnostic event emitted by `ModelGateway::stream`.
///
/// Adapters **pre-aggregate** provider quirks: text deltas pass through;
/// each content block emits a sealed `*Completed` event when the wire-level
/// `content_block_stop` (Anthropic) or `finish_reason` (`OpenAI` Chat
/// Completions) arrives. H06 stays stateless w.r.t. block accumulation —
/// see spec §Q1 mode X.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelEvent {
    /// One streaming text chunk inside an in-flight text block. Forwarded
    /// to the broadcast channel for live UI; persistence waits for
    /// `TextBlockCompleted`.
    TextDelta {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Partial text for this delta.
        chunk: String,
    },
    /// A text block has been sealed by the provider; carries the full
    /// accumulated text. H06 calls `recorder.on_text_block_complete(...)`.
    TextBlockCompleted {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Full accumulated text for the completed block.
        text: String,
    },
    /// A `tool_use` block has started; `call_id` and name are known. The model
    /// has not yet finished emitting the arguments.
    ToolUseStarted {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Opaque call identifier assigned by the model.
        call_id: String,
        /// Name of the tool being called.
        name: String,
    },
    /// A `tool_use` block has been sealed by the provider; carries the full
    /// parsed argument value. (Adapter buffered partial JSON internally.)
    ToolUseCompleted {
        /// Index of the block in the response.
        block_index: u32,
        /// Opaque call identifier.
        call_id: String,
        /// Tool name.
        name: String,
        /// Fully-parsed arguments.
        args: serde_json::Value,
    },
    /// Last event on the stream. Carries terminal reason + usage.
    MessageCompleted {
        /// Reason the model stopped generating.
        stop_reason: StopReason,
        /// Token usage for the completed message.
        usage: Usage,
    },
}

/// Sealed assistant message output. Constructed by H06 by walking the
/// `ModelEvent` stream from `stream()` to completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelOutput {
    /// All content blocks the model emitted, in `block_index` order.
    pub content: Vec<ContentBlock>,
    /// Stop reason from the final `MessageCompleted` event.
    pub stop_reason: StopReason,
    /// Token usage from the final `MessageCompleted` event.
    pub usage: Usage,
}

/// Failures the gateway can report from `stream()` or during the streamed
/// `Result<ModelEvent, ModelError>` items.
///
/// Marked `#[non_exhaustive]` so adapters can introduce provider-specific
/// classifications later without a breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ModelError {
    /// Network-layer failure (DNS, TCP, TLS, timeout).
    #[error("network error: {0}")]
    Network(String),
    /// Provider returned a non-2xx HTTP response.
    #[error("provider error {status}: {message}")]
    Provider {
        /// HTTP status code, e.g. 400, 500.
        status: u16,
        /// Best-effort extracted message from the provider's error body.
        message: String,
    },
    /// Authentication failed (401 / 403, or missing credentials).
    #[error("auth failed")]
    Auth,
    /// Rate limited by the provider; honor `retry_after_secs` if set.
    #[error("rate limited (retry-after: {retry_after_secs:?})")]
    RateLimited {
        /// Seconds the provider asked us to back off (`Retry-After` header).
        retry_after_secs: Option<u64>,
    },
    /// Response body decode failed (e.g. malformed JSON in SSE event).
    #[error("decode error: {0}")]
    Decode(String),
    /// `ExecCtx.cancel` fired while the stream was in flight.
    #[error("cancelled")]
    Cancelled,
}

//! `ModelGateway` and supporting value types.
//!
//! See:
//! - `docs/components/H06-stream-demux.md` for the consumer side (H06)
//! - `docs/superpowers/specs/2026-05-19-sprint-2-minimal-loop-design.md` §Q1
//!   for the gateway-pre-aggregation decision (X mode)
//! - ADR-0006 §"Sprint 2 protocol-layer additions" for the layer-rule rationale

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::ExecCtx;
use crate::content::ContentBlock;
use crate::tool::ToolDescriptor;

/// Model invocation parameters carried in `ModelInput.params`.
///
/// Field set is intentionally minimal in v0.1; provider adapters map only
/// what the wire format supports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// A `tool_use` block has started; `call_id` and `tool_name` are known.
    /// The model has not yet finished emitting the arguments.
    ToolUseStarted {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Opaque call identifier assigned by the model.
        call_id: String,
        /// Name of the tool being called. Matches `ContentBlock::ToolUse.tool_name`.
        tool_name: String,
    },
    /// A `tool_use` block has been sealed by the provider; carries the full
    /// parsed argument value. (Adapter buffered partial JSON internally.)
    ToolUseCompleted {
        /// Index of the block in the response.
        block_index: u32,
        /// Opaque call identifier.
        call_id: String,
        /// Name of the tool that was called. Matches `ContentBlock::ToolUse.tool_name`.
        tool_name: String,
        /// Fully-parsed arguments.
        args: serde_json::Value,
    },
    /// One streaming reasoning chunk inside an in-flight thinking block.
    /// Forwarded to the broadcast channel for live UI; persistence
    /// waits for `ThinkingBlockCompleted`. See ADR-0019 §3.
    ThinkingDelta {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Partial reasoning text for this delta.
        chunk: String,
    },
    /// A thinking block has been sealed by the provider; carries the
    /// full accumulated text plus any provider-opaque payload (signature
    /// for Anthropic, `encrypted_content` + `item_id` for `OpenAI` Responses,
    /// `None` for OpenAI-compat). H06 calls
    /// `recorder.on_thinking_block_complete(...)`.
    ThinkingBlockCompleted {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Full accumulated reasoning text for the completed block.
        text: String,
        /// Provider-opaque round-trip payload (see ADR-0019 §1).
        provider_opaque: Option<serde_json::Value>,
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

/// Limits of the model a `ModelGateway` serves.
///
/// Sourced from the gateway implementation, not from strategy config.
/// Consumed by Compactor for adaptive thresholds and (future) H05 surface
/// sizing. See ADR-0008.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelLimits {
    /// The model id, with any `[<size>]` suffix preserved.
    pub model_id: String,
    /// Total context window in tokens (input + output combined).
    pub context_window_tokens: u64,
}

impl ModelLimits {
    /// Construct a `ModelLimits` value. Provided so that crates outside
    /// `cogito-protocol` can build instances despite the `#[non_exhaustive]`
    /// attribute (which forbids struct literal syntax outside the defining
    /// crate).
    #[must_use]
    pub fn new(model_id: impl Into<String>, context_window_tokens: u64) -> Self {
        Self {
            model_id: model_id.into(),
            context_window_tokens,
        }
    }
}

/// Boundary contract between Brain and external LLM providers.
///
/// Implementations live in `cogito-model::anthropic` and
/// `cogito-model::openai_compat`; consumers may add provider adapters of
/// their own. Brain never imports those crates — it holds an
/// `Arc<dyn ModelGateway>` injected by Runtime.
///
/// Cancellation: dropping the returned stream causes the adapter to abort
/// the underlying HTTP connection. Tools / hooks signal cancellation via
/// `ExecCtx.cancel`; adapters should listen on it (e.g. `select!` against
/// `ctx.cancel.cancelled()`) to short-circuit before the next chunk read.
#[async_trait::async_trait]
pub trait ModelGateway: Send + Sync {
    /// Open a streaming model call. The returned stream emits zero or more
    /// non-`MessageCompleted` events followed by exactly one
    /// `MessageCompleted` event, then ends.
    ///
    /// # Errors
    ///
    /// Returns `ModelError` if request construction or initial connect
    /// fails. Per-chunk errors arrive as `Err` items inside the stream.
    async fn stream(
        &self,
        input: ModelInput,
        ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError>;

    /// Stable identifier for telemetry and logging. Adapters return a
    /// fixed string, not a per-instance value.
    fn provider_id(&self) -> &'static str;

    /// Limits of the model this gateway serves. Used by Compactor for
    /// adaptive thresholds. Default impl returns a conservative `32_768`
    /// window; provider adapters SHOULD override.
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            model_id: self.provider_id().into(),
            context_window_tokens: 32_768,
        }
    }
}

/// Parse the conventional `[<size>]` suffix from a model id.
///
/// Suffix grammar: `\[(\d+)([kKmM])?\]$`; `k` = `1_000`, `m` = `1_000_000`,
/// no unit = literal value.
///
/// Returns `None` if no suffix or suffix is malformed.
///
/// # Panics
///
/// Never panics in practice. The internal static regex is a compile-time
/// constant that is provably valid; the `expect` call exists only to satisfy
/// `OnceLock::get_or_init`'s infallible-initializer requirement.
///
/// Examples:
/// - `"claude-opus-4-7[1m]"` -> `Some(1_000_000)`
/// - `"Llama-3.3-70B[32k]"` -> `Some(32_000)`
/// - `"gpt-4o[128000]"` -> `Some(128_000)`
/// - `"claude-opus-4-7"` -> `None`
#[must_use]
pub fn parse_context_window_suffix(model_id: &str) -> Option<u64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        #[allow(clippy::expect_used)]
        regex::Regex::new(r"\[(\d+)([kKmM])?\]$").expect("static regex compiles")
    });
    let caps = re.captures(model_id)?;
    let num: u64 = caps.get(1)?.as_str().parse().ok()?;
    let mult = match caps.get(2).map(|m| m.as_str().to_lowercase()).as_deref() {
        Some("k") => 1_000,
        Some("m") => 1_000_000,
        _ => 1,
    };
    num.checked_mul(mult)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented
)]
mod model_limits_tests {
    use super::*;

    struct DummyGateway;

    #[async_trait::async_trait]
    impl ModelGateway for DummyGateway {
        async fn stream(
            &self,
            _input: ModelInput,
            _ctx: crate::ExecCtx,
        ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
            unimplemented!("not used in this test")
        }

        fn provider_id(&self) -> &'static str {
            "dummy"
        }
    }

    #[test]
    fn default_model_limits_is_conservative() {
        let g = DummyGateway;
        let limits = g.model_limits();
        assert_eq!(limits.context_window_tokens, 32_768);
        assert_eq!(limits.model_id, "dummy");
    }
}

#[cfg(test)]
mod thinking_tests {
    use super::*;

    #[test]
    fn thinking_delta_roundtrips() -> serde_json::Result<()> {
        let evt = ModelEvent::ThinkingDelta {
            block_index: 0,
            chunk: "I should ".into(),
        };
        let json = serde_json::to_string(&evt)?;
        assert!(
            json.contains(r#""kind":"thinking_delta""#),
            "tag missing: {json}"
        );
        let back: ModelEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }

    #[test]
    fn thinking_block_completed_roundtrips() -> serde_json::Result<()> {
        let evt = ModelEvent::ThinkingBlockCompleted {
            block_index: 0,
            text: "I should grep.".into(),
            provider_opaque: Some(serde_json::json!({"signature":"abc"})),
        };
        let json = serde_json::to_string(&evt)?;
        let back: ModelEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }
}

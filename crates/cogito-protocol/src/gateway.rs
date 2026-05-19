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

//! Context Management protocol surface — traits, event-emitting types, config.
//!
//! See `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
//! and ADR-0008 for the full design. Implementations live in `cogito-context`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gateway::ModelError;
use crate::store::StoreError;

/// Failure mode for any of the four Context-Management traits.
///
/// Per ADR-0008, H11 records the error into
/// `ContextDecisionRecorded.errors` and continues the turn (degrade, not block).
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum ContextError {
    /// Compactor's summarization model call failed.
    #[error("summarization model call failed: {0}")]
    SummarizationModelFailed(#[from] ModelError),

    /// An invariant was violated (range bounds, turn alignment,
    /// duplicate compaction for turn).
    #[error("invariant violated: {0}")]
    InvariantViolated(String),

    /// Operation was aborted (cancel-token fired).
    #[error("operation aborted")]
    Aborted,

    /// Underlying conversation store rejected the write.
    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

/// What a `ContextCompacted` event substitutes in for the covered seq range.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionReplacement {
    /// Drop the covered range entirely (truncate-style; no replacement message).
    Drop,
    /// Replace with a summary user-role message wrapped in
    /// `<conversation_summary>...</conversation_summary>`.
    Summary {
        /// The summary text. Plain UTF-8.
        text: String,
        /// Provider model id that produced the summary (e.g. `"claude-haiku-4-5"`).
        model: String,
    },
}

/// How `ToolFilterOverrider` modifies `strategy.allowed_tools` for one turn.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolFilterOverrideMode {
    /// Use `strategy.allowed_tools` unchanged (no-op).
    Inherit,
    /// Intersect `strategy.allowed_tools` with the listed tools.
    Intersect {
        /// Tool names to keep.
        tools: Vec<String>,
    },
    /// Replace `strategy.allowed_tools` entirely (used by Plugin / Subagent).
    Replace {
        /// Tool names that become the full surface.
        tools: Vec<String>,
    },
}

/// Per-trait error capture for `ContextDecisionRecorded`. Each field is
/// the serialized display of a `ContextError`, or `None` if the trait ran cleanly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextDecisionErrors {
    /// Compactor failure (degrade path); `None` if it succeeded or returned vec![].
    pub compactor: Option<String>,
    /// `SystemPromptInjector` failure (H11 wrote a fallback empty event).
    pub injector: Option<String>,
    /// `ToolFilterOverrider` failure (H11 wrote a fallback Inherit event).
    pub overrider: Option<String>,
}

//! Context Management protocol surface — traits, event-emitting types, config.
//!
//! See `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
//! and ADR-0008 for the full design. Implementations live in `cogito-context`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::content::ContentBlock;
use crate::event::ConversationEvent;
use crate::exec_ctx::ExecCtx;
use crate::gateway::ModelError;
use crate::gateway::{ModelGateway, Usage};
use crate::ids::{EventId, SessionId, TurnId};
use crate::store::{EventRecorder, StoreError};
use crate::strategy::HarnessStrategy;

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

/// What kind of compaction was applied. Embedded in `CompactionApplied`
/// so H11's `ContextDecisionRecorded` summary can describe it textually.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionKind {
    /// Drop the covered seq range with no replacement (sliding-window truncation).
    Truncate,
    /// Replace the covered range with a model-generated summary.
    Summarize,
    /// Elide large tool result bodies while keeping the call/result structure.
    ToolBodyElision,
}

/// Returned from `Compactor::maybe_compact` per `ContextCompacted` event written.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct CompactionApplied {
    /// The `EventId` of the `ContextCompacted` event that was persisted.
    pub event_id: EventId,
    /// Inclusive `(start_seq, end_seq)` range replaced by this compaction.
    pub replaced_seq_range: (u64, u64),
    /// The kind of compaction that was applied.
    pub kind: CompactionKind,
}

/// Input handed to `Compactor::maybe_compact` by H11.
pub struct CompactionInput<'a> {
    /// Session identifier for the current turn.
    pub session_id: SessionId,
    /// Turn identifier for the current turn.
    pub turn_id: TurnId,
    /// Full event history as loaded by H11 before the turn begins.
    pub history: &'a [ConversationEvent],
    /// Per-turn strategy knobs (system prompt, model params, tool filter).
    pub strategy: &'a HarnessStrategy,
    /// Token usage from the most recent model call, if available.
    /// `None` on the very first turn or when the previous turn had no model call.
    pub last_usage: Option<Usage>,
    /// Gateway reference used by summarization compactors to issue model calls.
    pub model_gateway: &'a dyn ModelGateway,
    /// Event recorder used to persist `ContextCompacted` events. Compactors
    /// call `recorder.append_payload(turn_id, EventPayload::ContextCompacted { .. })`
    /// for each compaction they perform.
    pub recorder: &'a mut dyn EventRecorder,
}

/// Decide whether (and how) to compact history for the upcoming turn.
///
/// Implementations MUST be idempotent on `turn_id`: if a `ContextCompacted`
/// event already exists in `history` for this turn, return its
/// `CompactionApplied` without doing further work.
///
/// Failures degrade — they do NOT propagate to H01 as fatal. H11 records
/// the error in `ContextDecisionRecorded.errors.compactor` and continues
/// the turn without compaction.
#[async_trait]
pub trait Compactor: Send + Sync {
    /// Inspect `input.history` and, if compaction is warranted, write one or
    /// more `ContextCompacted` events via `input.recorder` and return a
    /// `CompactionApplied` descriptor for each. Return an empty vec when no
    /// compaction is needed.
    async fn maybe_compact(
        &self,
        input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError>;

    /// Stable identifier for this compactor implementation, written into
    /// `ContextCompacted.produced_by` (e.g. `"truncate"`, `"summarize"`).
    fn id(&self) -> &'static str;
}

/// One message in a projected history. Roles match the wire-format roles
/// adapters serialize for Anthropic/OpenAI/etc.
///
/// `System` carries the assembled system prompt; `User` carries a plain text
/// user turn; `Assistant` carries the model's content blocks; `ToolResult`
/// carries tool output fed back to the model.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectedMessage {
    /// System prompt for the turn.
    System(String),
    /// User-role text message.
    User(String),
    /// Assistant-role message carrying content blocks (text, tool-use, thinking).
    Assistant(Vec<ContentBlock>),
    /// Tool result message fed back to the model after a tool call.
    ToolResult {
        /// Opaque call identifier matching the originating `ToolUse.call_id`.
        call_id: String,
        /// Content blocks comprising the tool result.
        result_blocks: Vec<ContentBlock>,
    },
}

/// Project events + strategy into the dialogue messages H04 sends to the model.
///
/// Pure: implementations MUST be deterministic synchronous functions —
/// no I/O, no event writes, no clock reads. The covered-set + replacement
/// algorithm is fully specified in ADR-0008 and must be honored verbatim.
pub trait HistoryProjector: Send + Sync {
    /// Build the message list for `current_turn`'s upcoming model call.
    fn project(
        &self,
        events: &[ConversationEvent],
        strategy: &HarnessStrategy,
        current_turn: TurnId,
    ) -> Vec<ProjectedMessage>;

    /// Stable identifier for tracing and logging (e.g. `"default"`, `"compaction-aware"`).
    fn id(&self) -> &'static str;
}

/// Input handed to `SystemPromptInjector::inject`.
pub struct InjectionInput<'a> {
    /// Session identifier for the current turn.
    pub session_id: SessionId,
    /// Turn identifier for the current turn.
    pub turn_id: TurnId,
    /// Per-turn strategy knobs (system prompt, model params, tool filter).
    pub strategy: &'a HarnessStrategy,
    /// Full event history as loaded by H11 before the turn begins.
    pub history: &'a [ConversationEvent],
    /// Per-invocation execution context handed to every component.
    pub exec_ctx: &'a ExecCtx,
    /// Event recorder used to persist `SystemPromptInjected` events.
    pub recorder: &'a mut dyn EventRecorder,
}

/// Produce per-turn additions to `strategy.system_prompt`.
///
/// MUST write a `SystemPromptInjected` event every turn (even when the
/// suffix is empty). MUST be idempotent on `turn_id` for resume safety:
/// if a `SystemPromptInjected` event already exists in history for this
/// turn, return its `EventId` without writing a new one.
#[async_trait]
pub trait SystemPromptInjector: Send + Sync {
    /// Compute this turn's suffix and persist a `SystemPromptInjected` event.
    ///
    /// Returns the `EventId` of the written (or already-existing) event.
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError>;

    /// Stable identifier for this injector implementation (e.g. `"noop"`, `"date-injector"`).
    fn id(&self) -> &'static str;
}

/// Input handed to `ToolFilterOverrider::override_filter`.
pub struct ToolFilterInput<'a> {
    /// Session identifier for the current turn.
    pub session_id: SessionId,
    /// Turn identifier for the current turn.
    pub turn_id: TurnId,
    /// Per-turn strategy knobs (system prompt, model params, tool filter).
    pub strategy: &'a HarnessStrategy,
    /// Full event history as loaded by H11 before the turn begins.
    pub history: &'a [ConversationEvent],
    /// Per-invocation execution context handed to every component.
    pub exec_ctx: &'a ExecCtx,
    /// Event recorder used to persist `ToolFilterOverridden` events.
    pub recorder: &'a mut dyn EventRecorder,
}

/// Decide per-turn tool-filter override on top of `strategy.allowed_tools`.
///
/// MUST write a `ToolFilterOverridden` event every turn (`Inherit` counts
/// as ran). MUST be idempotent on `turn_id` for resume safety: if a
/// `ToolFilterOverridden` event already exists in history for this turn,
/// return its `EventId` without writing a new one.
#[async_trait]
pub trait ToolFilterOverrider: Send + Sync {
    /// Compute this turn's mode and persist a `ToolFilterOverridden` event.
    ///
    /// Returns the `EventId` of the written (or already-existing) event.
    async fn override_filter(&self, input: ToolFilterInput<'_>) -> Result<EventId, ContextError>;

    /// Stable identifier for this overrider implementation (e.g. `"noop"`, `"plugin-overrider"`).
    fn id(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// Per-trait configuration container; lives in `HarnessStrategy.context`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextConfig {
    /// Configuration for the `Compactor` trait implementation.
    pub compactor: CompactorConfig,
    /// Configuration for the `HistoryProjector` trait implementation.
    pub history_projector: HistoryProjectorConfig,
    /// Configuration for the `SystemPromptInjector` trait implementation.
    pub system_prompt_injector: SystemPromptInjectorConfig,
    /// Configuration for the `ToolFilterOverrider` trait implementation.
    pub tool_filter_overrider: ToolFilterOverriderConfig,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            compactor: CompactorConfig::None,
            history_projector: HistoryProjectorConfig::Standard,
            system_prompt_injector: SystemPromptInjectorConfig::None,
            tool_filter_overrider: ToolFilterOverriderConfig::None,
        }
    }
}

/// Selects which `Compactor` implementation H11 instantiates.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactorConfig {
    /// No compaction; H11 passes history to the model unchanged.
    None,
    /// Sliding-window truncation compactor.
    Truncate(TruncateConfig),
}

/// Per-trait config for `TruncateCompactor`. v0.1 ships only this Compactor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TruncateConfig {
    /// Adaptive (ratio of `model_limits().context_window_tokens`) or absolute.
    #[serde(default)]
    pub max_tokens: TokenThreshold,
    /// Preserve the first user message (turn 1)?
    #[serde(default = "default_true")]
    pub keep_first_user: bool,
    /// Always preserve this many most-recent completed turns.
    #[serde(default = "default_keep_recent")]
    pub keep_recent_turns: u32,
}

const fn default_true() -> bool {
    true
}

const fn default_keep_recent() -> u32 {
    5
}

/// Token budget threshold — either adaptive (relative to the model's context
/// window) or an absolute hard limit.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenThreshold {
    /// Adaptive: `ratio * context_window_tokens - safety_headroom`.
    Ratio {
        /// Fraction of the model's total context window to target (0.0–1.0).
        of_context_window: f32,
        /// Tokens reserved for model response and safety margin.
        safety_headroom: u64,
    },
    /// Hard absolute (ignores `model_limits`).
    Absolute(u64),
}

impl Default for TokenThreshold {
    fn default() -> Self {
        Self::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8192,
        }
    }
}

/// Selects which `HistoryProjector` implementation H11 instantiates.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryProjectorConfig {
    /// Standard compaction-aware projector (the only v0.1 implementation).
    Standard,
}

/// Selects which `SystemPromptInjector` implementation H11 instantiates.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemPromptInjectorConfig {
    /// No-op injector; `strategy.system_prompt` is used unchanged.
    None,
}

/// Selects which `ToolFilterOverrider` implementation H11 instantiates.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolFilterOverriderConfig {
    /// No-op overrider; `strategy.allowed_tools` is used unchanged.
    None,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
mod config_tests {
    use super::*;

    #[test]
    fn default_config_is_all_none() {
        let c = ContextConfig::default();
        assert!(matches!(c.compactor, CompactorConfig::None));
        assert!(matches!(
            c.history_projector,
            HistoryProjectorConfig::Standard
        ));
        assert!(matches!(
            c.system_prompt_injector,
            SystemPromptInjectorConfig::None
        ));
        assert!(matches!(
            c.tool_filter_overrider,
            ToolFilterOverriderConfig::None
        ));
    }

    #[test]
    fn truncate_config_toml_roundtrip() {
        let toml_input = r#"
[compactor]
kind = "truncate"
keep_first_user = true
keep_recent_turns = 5

[compactor.max_tokens]
kind = "ratio"
of_context_window = 0.75
safety_headroom = 8192

[history_projector]
kind = "standard"

[system_prompt_injector]
kind = "none"

[tool_filter_overrider]
kind = "none"
"#;
        let parsed: ContextConfig = toml::from_str(toml_input).expect("parses");
        let CompactorConfig::Truncate(t) = &parsed.compactor else {
            panic!("expected truncate");
        };
        assert!(t.keep_first_user);
        assert_eq!(t.keep_recent_turns, 5);
        let TokenThreshold::Ratio {
            of_context_window,
            safety_headroom,
        } = t.max_tokens
        else {
            panic!("expected ratio");
        };
        assert!((of_context_window - 0.75).abs() < 1e-6);
        assert_eq!(safety_headroom, 8192);
    }
}

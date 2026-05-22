//! Context Management protocol surface — traits, event-emitting types, config.
//!
//! See `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
//! and ADR-0008 for the full design. Implementations live in `cogito-context`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::content::ContentBlock;
use crate::event::ConversationEvent;
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

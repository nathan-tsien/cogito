//! H02 Step Recorder.
//!
//! Owns the live mapping from H01 / H06 events into the two streams:
//!
//! - **Persisted**: `ConversationEvent` written to `ConversationStore`.
//! - **Live broadcast**: `StreamEvent` sent to subscribers.
//!
//! See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §6 and ADR-0006 §7 for the dual-stream rationale. Text-block batching:
//! per Codex / Claude Code precedent, text deltas are NOT persisted
//! individually. They are accumulated until the wire-protocol
//! `content_block_stop` (text block) boundary, then written as one
//! `AssistantMessageAppended` event.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::{
    CompactionReplacement, ContextDecisionErrors, TokenEstimates, ToolFilterOverrideMode,
};
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::ids::{EventId, MessageId, SessionId, TurnId};
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::store::{ConversationStore, EventRecorder, StoreError};
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolResult;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use futures::StreamExt as _;
use tokio::sync::broadcast;

/// H02 Step Recorder. Persists [`ConversationEvent`]s to the
/// [`ConversationStore`] and fans out a parallel live [`StreamEvent`]
/// broadcast for subscribers (TUI, observability, consumer hooks).
///
/// Text deltas are buffered until [`StepRecorder::on_text_block_complete`]
/// to honor the H02 batching contract; every other event is persisted
/// immediately on its corresponding method.
///
/// The recorder keeps a local mirror of all appended events in
/// `history_cache`. This cache is used by `record_context_compacted` to
/// validate the §5.5 invariants without issuing an additional store read.
/// The cache is built from the `seq_start` position onward; events that
/// existed before the recorder was constructed are loaded on first access
/// via `load_prior_history`.
pub struct StepRecorder {
    store: Arc<dyn ConversationStore>,
    events_tx: broadcast::Sender<StreamEvent>,
    session_id: SessionId,
    seq_counter: u64,
    current_text_block: Option<TextBlockBuf>,
    current_thinking_block: Option<ThinkingBlockBuf>,
    /// Identity of the in-flight assistant message (one model call). Minted
    /// at `record_model_call_started` and stamped on the message's live and
    /// persisted events until the next model call replaces it. `None` before
    /// the first model call of a turn. See ADR-0041.
    current_message_id: Option<MessageId>,
    /// Mirror of all events persisted through this recorder instance.
    /// Used for invariant checking in `record_context_compacted`.
    history_cache: Vec<ConversationEvent>,
}

/// In-flight text block accumulator. Filled by `on_text_delta` and drained
/// by `on_text_block_complete` into a single `AssistantMessageAppended`.
struct TextBlockBuf {
    turn_id: TurnId,
    text: String,
    /// Message identity captured when the block opened, so the flushed
    /// `AssistantMessageAppended` carries the same id the live deltas did.
    message_id: Option<MessageId>,
}

/// In-flight thinking block accumulator. Filled by `on_thinking_delta`
/// and drained by `on_thinking_block_complete` into a single
/// `ThinkingBlockRecorded` event. `provider_opaque` is supplied at
/// flush time because adapters pre-aggregate signature/encrypted blobs
/// and only know the final payload after the wire `content_block_stop`.
struct ThinkingBlockBuf {
    turn_id: TurnId,
    text: String,
    /// See `TextBlockBuf::message_id`.
    message_id: Option<MessageId>,
}

impl StepRecorder {
    /// Create a recorder bound to `session_id`. `seq_start` is the seq
    /// number to assign to the next appended event; pass `0` for a fresh
    /// session, or `latest_seq + 1` when resuming.
    ///
    /// When `seq_start > 0` (resume path), prior events are NOT pre-loaded
    /// into `history_cache`. They are fetched lazily on the first call to
    /// `record_context_compacted` via `load_prior_history`.
    pub fn new(
        store: Arc<dyn ConversationStore>,
        events_tx: broadcast::Sender<StreamEvent>,
        session_id: SessionId,
        seq_start: u64,
    ) -> Self {
        Self {
            store,
            events_tx,
            session_id,
            seq_counter: seq_start,
            current_text_block: None,
            current_thinking_block: None,
            current_message_id: None,
            history_cache: Vec::new(),
        }
    }

    /// Crate-internal access to the live `StreamEvent` broadcast sender so
    /// the H06 demuxer can emit side-channel events (e.g.
    /// `SkillActivationRequested`) without going through `record_*`.
    pub(crate) fn events_tx(&self) -> &broadcast::Sender<StreamEvent> {
        &self.events_tx
    }

    /// Crate-internal iterator over the in-memory mirror of events appended
    /// through this recorder instance.
    ///
    /// Used by the session loop to recover the `call_id` associated with a
    /// just-paused job (see `runtime::session_loop::lookup_call_id_in_recorder`)
    /// without re-reading the persisted store. Only events appended through
    /// this instance are visible; resume-path callers that need prior events
    /// must consult the store directly (cf. `ensure_prior_history_loaded`).
    pub(crate) fn history_cache_iter(&self) -> impl DoubleEndedIterator<Item = &ConversationEvent> {
        self.history_cache.iter()
    }

    /// Record the session-open event. Called once per session, before any
    /// turn starts. Does not emit a [`StreamEvent`] — session-level state
    /// is observable via the persisted log only.
    pub async fn record_session_started(
        &mut self,
        meta: SessionMeta,
    ) -> Result<EventId, StoreError> {
        self.append(None, EventPayload::SessionStarted { meta })
            .await
    }

    /// Record the start of a new turn and broadcast a live
    /// [`StreamEvent::TurnStarted`].
    ///
    /// `activate_skills` carries the user-requested skill names (from a
    /// `TurnTrigger::SkillActivation`); pass an empty vec for plain
    /// `UserText`-triggered turns. The list is independent from
    /// sigil-based activations, which are re-derived from previous-turn
    /// text by H06.
    pub async fn record_turn_started(
        &mut self,
        turn_id: TurnId,
        user_input: Vec<ContentBlock>,
        activate_skills: Vec<String>,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnStarted {
            turn_id: Some(turn_id),
            subagent_call_id: None,
        });
        self.append(
            Some(turn_id),
            EventPayload::TurnStarted {
                user_input,
                activate_skills,
            },
        )
        .await
    }

    /// Buffer a streaming text chunk and broadcast it live as
    /// [`StreamEvent::TextDelta`]. Does NOT persist — call
    /// [`StepRecorder::on_text_block_complete`] when the wire protocol
    /// signals the block is finished to flush the buffer as a single
    /// `AssistantMessageAppended` event.
    pub fn on_text_delta(&mut self, turn_id: TurnId, chunk: String) {
        let message_id = self.current_message_id;
        let buf = self.current_text_block.get_or_insert_with(|| TextBlockBuf {
            turn_id,
            text: String::new(),
            message_id,
        });
        buf.text.push_str(&chunk);
        // Broadcast after the buffer push so the buffer is the source of
        // truth even if the channel has no live subscribers.
        let _ = self.events_tx.send(StreamEvent::TextDelta {
            chunk,
            turn_id: Some(turn_id),
            subagent_call_id: None,
            message_id,
        });
    }

    /// Persist the accumulated text block, if any. No-op when no
    /// `on_text_delta` calls have arrived since the last flush.
    ///
    /// Returns `Ok(None)` when there was no buffered text to flush (no-op
    /// path), or `Ok(Some(event_id))` when a `AssistantMessageAppended`
    /// event was persisted.
    pub async fn on_text_block_complete(&mut self) -> Result<Option<EventId>, StoreError> {
        let Some(buf) = self.current_text_block.take() else {
            return Ok(None);
        };
        let event_id = self
            .append(
                Some(buf.turn_id),
                EventPayload::AssistantMessageAppended {
                    text: buf.text,
                    message_id: buf.message_id,
                },
            )
            .await?;
        Ok(Some(event_id))
    }

    /// Buffer a streaming reasoning chunk and broadcast it live as
    /// [`StreamEvent::ThinkingDelta`]. Does NOT persist — call
    /// [`StepRecorder::on_thinking_block_complete`] when the wire
    /// protocol signals the block is finished.
    pub fn on_thinking_delta(&mut self, turn_id: TurnId, chunk: String) {
        let message_id = self.current_message_id;
        let buf = self
            .current_thinking_block
            .get_or_insert_with(|| ThinkingBlockBuf {
                turn_id,
                text: String::new(),
                message_id,
            });
        buf.text.push_str(&chunk);
        let _ = self.events_tx.send(StreamEvent::ThinkingDelta {
            chunk,
            turn_id: Some(turn_id),
            message_id,
        });
    }

    /// Persist the accumulated thinking block as one
    /// `ThinkingBlockRecorded` event. `provider_opaque` is taken from
    /// the gateway's `ThinkingBlockCompleted` event (signature for
    /// Anthropic, `encrypted_content` for `OpenAI` Responses, None for
    /// OpenAI-compat). No-op when no `on_thinking_delta` calls have
    /// arrived since the last flush.
    pub async fn on_thinking_block_complete(
        &mut self,
        provider_opaque: Option<serde_json::Value>,
    ) -> Result<Option<EventId>, StoreError> {
        let Some(buf) = self.current_thinking_block.take() else {
            return Ok(None);
        };
        let event_id = self
            .append(
                Some(buf.turn_id),
                EventPayload::ThinkingBlockRecorded {
                    text: buf.text,
                    provider_opaque,
                    message_id: buf.message_id,
                },
            )
            .await?;
        Ok(Some(event_id))
    }

    /// Record a `tool_use` content block and broadcast
    /// [`StreamEvent::ToolDispatchStarted`].
    pub async fn record_tool_use(
        &mut self,
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) -> Result<EventId, StoreError> {
        let message_id = self.current_message_id;
        let _ = self.events_tx.send(StreamEvent::ToolDispatchStarted {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            args: args.clone(),
            turn_id: Some(turn_id),
            message_id,
        });
        self.append(
            Some(turn_id),
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
                message_id,
            },
        )
        .await
    }

    /// Record a `tool_result` and broadcast
    /// [`StreamEvent::ToolDispatchEnded`] with the success flag derived
    /// from the [`ToolResult`] variant.
    pub async fn record_tool_result(
        &mut self,
        turn_id: TurnId,
        call_id: String,
        result: ToolResult,
    ) -> Result<EventId, StoreError> {
        let (ok, error_message) = match &result {
            ToolResult::Output(_) => (true, None),
            ToolResult::Error { message, .. } => (false, Some(message.clone())),
            // `ToolResult` is `#[non_exhaustive]`: treat unknown variants
            // as failures with no human-readable message.
            _ => (false, None),
        };
        let _ = self.events_tx.send(StreamEvent::ToolDispatchEnded {
            call_id: call_id.clone(),
            ok,
            error_message,
            turn_id: Some(turn_id),
            message_id: self.current_message_id,
        });
        self.append(
            Some(turn_id),
            EventPayload::ToolResultRecorded { call_id, result },
        )
        .await
    }

    /// Record that the turn paused on an async job and broadcast
    /// [`StreamEvent::TurnPaused`].
    pub async fn record_turn_paused(
        &mut self,
        turn_id: TurnId,
        job_id: JobId,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnPaused {
            turn_id: Some(turn_id),
        });
        self.append(Some(turn_id), EventPayload::TurnPaused { job_id })
            .await
    }

    /// Record that H08 submitted an async job for a tool call.
    ///
    /// Persists a [`EventPayload::JobSubmitted`] event but does NOT
    /// broadcast a [`StreamEvent`] — the broadcast happens later, at
    /// the subsequent [`StepRecorder::record_turn_paused`] call, so
    /// that subscribers observe the canonical pause boundary.
    pub async fn record_job_submitted(
        &mut self,
        turn_id: TurnId,
        call_id: String,
        job_id: JobId,
        tool_name: String,
    ) -> Result<EventId, StoreError> {
        self.append(
            Some(turn_id),
            EventPayload::JobSubmitted {
                call_id,
                job_id,
                tool_name,
            },
        )
        .await
    }

    /// Record that a previously-awaited job completed and broadcast
    /// [`StreamEvent::TurnResumed`].
    ///
    /// The persisted [`EventPayload::JobCompletedRecorded`] carries a
    /// [`JobOutcome`], so this method takes that type directly rather
    /// than the wire-level `JobCompletionEvent` envelope (the envelope's
    /// `job_id` is already a parameter here).
    pub async fn record_job_completed(
        &mut self,
        turn_id: TurnId,
        job_id: JobId,
        outcome: JobOutcome,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnResumed {
            turn_id: Some(turn_id),
        });
        self.append(
            Some(turn_id),
            EventPayload::JobCompletedRecorded { job_id, outcome },
        )
        .await
    }

    /// Record successful turn completion and broadcast
    /// [`StreamEvent::TurnCompleted`].
    ///
    /// `stop_reason` is the turn's terminal stop reason (the final model call's
    /// `stop_reason`); it rides on the broadcast so a live subscriber can flag a
    /// `MaxTokens`-truncated turn without scanning `ModelCallCompleted`
    /// (ADR-0040). It is deliberately *not* persisted onto
    /// `EventPayload::TurnCompleted` — the same value already sits in the
    /// adjacent `ModelCallCompleted` event.
    pub async fn record_turn_completed(
        &mut self,
        turn_id: TurnId,
        outcome: TurnOutcome,
        stop_reason: cogito_protocol::gateway::StopReason,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnCompleted {
            stop_reason: Some(stop_reason),
            turn_id: Some(turn_id),
            subagent_call_id: None,
        });
        self.append(Some(turn_id), EventPayload::TurnCompleted { outcome })
            .await
    }

    /// Record the `Init → ContextManaged` transition entry point.
    pub async fn record_context_manage_entered(
        &mut self,
        turn_id: TurnId,
    ) -> Result<EventId, StoreError> {
        self.append(Some(turn_id), EventPayload::ContextManageEntered {})
            .await
    }

    /// Record the `ContextManaged → PromptBuilt` transition entry point.
    pub async fn record_context_manage_completed(
        &mut self,
        turn_id: TurnId,
    ) -> Result<EventId, StoreError> {
        self.append(Some(turn_id), EventPayload::ContextManageCompleted {})
            .await
    }

    /// Record a `SystemPromptInjected` event for `turn_id`.
    ///
    /// Idempotent per turn: if an event already exists in the log for this
    /// `turn_id`, the existing [`EventId`] is returned without writing a
    /// second event. H11 calls this exactly once per turn; the idempotency
    /// guard protects against resume replays that re-enter `ContextManaged`.
    pub async fn record_system_prompt_injected(
        &mut self,
        turn_id: TurnId,
        suffix: String,
        contributors: Vec<String>,
        produced_by: impl Into<String>,
    ) -> Result<EventId, StoreError> {
        self.ensure_prior_history_loaded().await?;
        if let Some(existing_id) = self.history_cache.iter().find_map(|ev| match &ev.payload {
            EventPayload::SystemPromptInjected { turn_id: t, .. } if *t == turn_id => {
                Some(ev.event_id)
            }
            _ => None,
        }) {
            return Ok(existing_id);
        }
        self.append(
            Some(turn_id),
            EventPayload::SystemPromptInjected {
                turn_id,
                suffix,
                contributors,
                produced_by: produced_by.into(),
            },
        )
        .await
    }

    /// Record a `ToolFilterOverridden` event for `turn_id`.
    ///
    /// Idempotent per turn: if an event already exists in the log for this
    /// `turn_id`, the existing [`EventId`] is returned without writing a
    /// second event. H11 calls this exactly once per turn; the idempotency
    /// guard protects against resume replays that re-enter `ContextManaged`.
    pub async fn record_tool_filter_overridden(
        &mut self,
        turn_id: TurnId,
        mode: ToolFilterOverrideMode,
        contributors: Vec<String>,
        produced_by: impl Into<String>,
    ) -> Result<EventId, StoreError> {
        self.ensure_prior_history_loaded().await?;
        if let Some(existing_id) = self.history_cache.iter().find_map(|ev| match &ev.payload {
            EventPayload::ToolFilterOverridden { turn_id: t, .. } if *t == turn_id => {
                Some(ev.event_id)
            }
            _ => None,
        }) {
            return Ok(existing_id);
        }
        self.append(
            Some(turn_id),
            EventPayload::ToolFilterOverridden {
                turn_id,
                mode,
                contributors,
                produced_by: produced_by.into(),
            },
        )
        .await
    }

    /// Record the H11 `ContextDecisionRecorded` summary for this turn.
    ///
    /// Not idempotent — H11 is responsible for calling this exactly once per
    /// `ContextManaged` state entry. The event carries cross-references to the
    /// `SystemPromptInjected` and `ToolFilterOverridden` events written earlier
    /// in the same turn, so H11 must supply those [`EventId`]s.
    pub async fn record_context_decision(
        &mut self,
        turn_id: TurnId,
        compactions: Vec<EventId>,
        system_prompt_event: EventId,
        tool_filter_event: EventId,
        errors: ContextDecisionErrors,
    ) -> Result<EventId, StoreError> {
        self.append(
            Some(turn_id),
            EventPayload::ContextDecisionRecorded {
                turn_id,
                compactions,
                system_prompt_event,
                tool_filter_event,
                errors,
            },
        )
        .await
    }

    /// Record prompt composition metadata. Called after H04/H05 produce the
    /// `ModelInput` but before the gateway stream opens.
    pub async fn record_prompt_composed(
        &mut self,
        turn_id: TurnId,
        model: String,
        surface_size: u32,
    ) -> Result<EventId, StoreError> {
        self.append(
            Some(turn_id),
            EventPayload::PromptComposed {
                model,
                surface_size,
            },
        )
        .await
    }

    /// Record the start of a model gateway call. Called at the
    /// `PromptBuilt → ModelCalling` transition boundary.
    ///
    /// This is the assistant-message boundary (ADR-0041): a fresh `MessageId`
    /// is minted and broadcast as [`StreamEvent::AssistantMessageStarted`], and
    /// held as `current_message_id` so every event composing this model call's
    /// output (text/thinking deltas, tool dispatches, and their persisted
    /// counterparts) carries the same id. It is replaced on the next model
    /// call, so a multi-call (tool-loop) turn yields one message per call.
    pub async fn record_model_call_started(
        &mut self,
        turn_id: TurnId,
        model: String,
    ) -> Result<EventId, StoreError> {
        let message_id = MessageId::new();
        self.current_message_id = Some(message_id);
        let _ = self.events_tx.send(StreamEvent::AssistantMessageStarted {
            message_id,
            turn_id: Some(turn_id),
            subagent_call_id: None,
        });
        self.append(Some(turn_id), EventPayload::ModelCallStarted { model })
            .await
    }

    /// Record the sealing event for a model call. Called by H06 demux loop
    /// when `ModelEvent::MessageCompleted` is observed. Must complete before
    /// `demux` returns the sealed `ModelOutput` so that H03 can distinguish
    /// "model call done" from "model call in flight" without re-issuing the
    /// gateway request.
    pub async fn record_model_call_completed(
        &mut self,
        turn_id: TurnId,
        stop_reason: cogito_protocol::gateway::StopReason,
        usage: cogito_protocol::gateway::Usage,
    ) -> Result<EventId, StoreError> {
        self.append(
            Some(turn_id),
            EventPayload::ModelCallCompleted { stop_reason, usage },
        )
        .await
    }

    /// Record that an H09 hook rejected a lifecycle point.
    ///
    /// This event is purely an additive audit log entry (ADR-0007). It is
    /// persisted before the subsequent `TurnFailed` event so the log ordering
    /// reflects causality. No `StreamEvent` is broadcast — subscribers learn
    /// about the rejection through the `TurnFailed` broadcast that follows.
    pub async fn record_hook_rejected(
        &mut self,
        turn_id: TurnId,
        hook_name: String,
        point: cogito_protocol::hook::HookLifecyclePoint,
        reason: String,
    ) -> Result<EventId, StoreError> {
        self.append(
            Some(turn_id),
            EventPayload::HookRejected {
                hook_name,
                point,
                reason,
            },
        )
        .await
    }

    /// Record turn failure and broadcast [`StreamEvent::TurnFailed`] with
    /// a human-readable rendering of the reason.
    ///
    /// [`TurnFailureReason`] does not implement [`std::fmt::Display`] in
    /// v0.1 (only [`std::fmt::Debug`]); the subscriber-facing string is
    /// produced via `Debug` formatting until a dedicated user-facing
    /// renderer lands.
    pub async fn record_turn_failed(
        &mut self,
        turn_id: TurnId,
        reason: TurnFailureReason,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnFailed {
            reason: format!("{reason:?}"),
            turn_id: Some(turn_id),
            subagent_call_id: None,
        });
        self.append(Some(turn_id), EventPayload::TurnFailed { reason })
            .await
    }

    /// Record a compaction event with §5.5 invariant enforcement.
    ///
    /// Enforced invariants (returns `StoreError::InvariantViolated` on failure):
    ///
    /// 1. `replaced_seq_range.1 < next_seq` — the range must lie entirely in
    ///    the past; the new event's own seq is not included in its covered range.
    /// 2. `replaced_seq_range.0 <= replaced_seq_range.1` — the range is
    ///    well-formed (non-empty, non-inverted).
    /// 3. `replaced_seq_range.0` must be the seq of a `TurnStarted` event —
    ///    compaction always starts at a turn boundary.
    /// 4. The event immediately after `replaced_seq_range.1` (if any) must be
    ///    a `TurnStarted` or `ContextManageEntered` — compaction covers exactly
    ///    the tail of one complete turn.
    /// 5. No `ContextCompacted` event for the same `turn_id` may already exist
    ///    in the log — the compactor is responsible for idempotency before
    ///    calling this method.
    pub async fn record_context_compacted(
        &mut self,
        turn_id: TurnId,
        replaced_seq_range: (u64, u64),
        produced_by: impl Into<String>,
        replacement: CompactionReplacement,
        estimates: TokenEstimates,
    ) -> Result<EventId, StoreError> {
        // Ensure prior events (from resume path) are in the cache.
        self.ensure_prior_history_loaded().await?;
        let next_seq = self.seq_counter;

        // Invariant 1: range must lie entirely in the past.
        if replaced_seq_range.1 >= next_seq {
            return Err(StoreError::InvariantViolated(format!(
                "ContextCompacted.replaced_seq_range.1 = {} must be < next_seq = {}",
                replaced_seq_range.1, next_seq,
            )));
        }

        // Invariant 2: range is well-formed.
        if replaced_seq_range.0 > replaced_seq_range.1 {
            return Err(StoreError::InvariantViolated(format!(
                "ContextCompacted.replaced_seq_range is malformed: ({}, {})",
                replaced_seq_range.0, replaced_seq_range.1,
            )));
        }

        // Invariant 3: range.0 is a TurnStarted seq.
        let start_is_turn_boundary = self.history_cache.iter().any(|ev| {
            ev.seq == replaced_seq_range.0 && matches!(ev.payload, EventPayload::TurnStarted { .. })
        });
        if !start_is_turn_boundary {
            return Err(StoreError::InvariantViolated(format!(
                "replaced_seq_range.0 = {} is not the seq of a TurnStarted event",
                replaced_seq_range.0,
            )));
        }

        // Invariant 4: range.1 is the last event of its turn (the next event, if
        // any, must be TurnStarted or ContextManageEntered).
        if let Some(next_event) = self
            .history_cache
            .iter()
            .find(|ev| ev.seq > replaced_seq_range.1)
        {
            let is_valid_boundary = matches!(
                next_event.payload,
                EventPayload::TurnStarted { .. } | EventPayload::ContextManageEntered { .. }
            );
            if !is_valid_boundary {
                return Err(StoreError::InvariantViolated(format!(
                    "replaced_seq_range.1 = {} is not the last event of its turn \
                     (seq {} follows it with category {:?})",
                    replaced_seq_range.1,
                    next_event.seq,
                    next_event.payload.category(),
                )));
            }
        }

        // Invariant 5: at most one ContextCompacted per turn_id.
        let already_compacted = self.history_cache.iter().any(|ev| {
            matches!(
                &ev.payload,
                EventPayload::ContextCompacted { turn_id: t, .. } if *t == turn_id
            )
        });
        if already_compacted {
            return Err(StoreError::InvariantViolated(format!(
                "ContextCompacted already exists for turn_id {turn_id:?}",
            )));
        }

        self.append(
            Some(turn_id),
            EventPayload::ContextCompacted {
                turn_id,
                replaced_seq_range,
                produced_by: produced_by.into(),
                replacement,
                token_estimate_before: estimates.before,
                token_estimate_after: estimates.after,
            },
        )
        .await
    }

    /// Ensure `history_cache` contains all events that predate this recorder
    /// instance (resume path).
    ///
    /// On the fresh-session path (`seq_start == 0`) the cache is always
    /// complete because every `append` call adds to it. On the resume path
    /// (`seq_start > 0`) events from prior turns live only in the store. This
    /// method is called lazily by `record_context_compacted` and is a no-op
    /// after the first successful load.
    ///
    /// `ConversationStore::replay(sid, from_seq)` returns events where
    /// `seq > from_seq`. Seq-0 is always `SessionStarted`, never `TurnStarted`,
    /// so starting the stream at `from_seq = 0` (which yields seq >= 1) is safe
    /// for the turn-boundary invariants checked in `record_context_compacted`.
    async fn ensure_prior_history_loaded(&mut self) -> Result<(), StoreError> {
        // Count how many seqs should be in the cache at this point. The cache
        // holds all events appended through this instance (from seq_start onward).
        // If seq_start > 0, prior events (seq 0 .. seq_start-1) are missing.
        let seq_start = self.seq_counter - self.history_cache.len() as u64;
        if seq_start == 0 {
            // Fresh session: nothing predates this recorder instance.
            return Ok(());
        }

        // Load all events with seq < seq_start from the store. We use
        // `replay(sid, 0)` which yields seq > 0 (seq-0 is SessionStarted and
        // is not a TurnStarted boundary, so it is safe to skip).
        let mut prior: Vec<ConversationEvent> = Vec::new();
        let mut stream = self.store.replay(self.session_id, 0);
        while let Some(result) = stream.next().await {
            let event = result?;
            if event.seq < seq_start {
                prior.push(event);
            }
        }
        prior.sort_unstable_by_key(|e| e.seq);

        // Prepend prior events in front of the locally-appended ones.
        let mut combined = prior;
        combined.append(&mut self.history_cache);
        self.history_cache = combined;
        Ok(())
    }

    /// Build the envelope, persist via the store, and advance the
    /// session-local sequence counter. Returns the [`EventId`] minted for
    /// this event so callers can carry it forward (e.g. into `TurnState::Failed`).
    async fn append(
        &mut self,
        turn_id: Option<TurnId>,
        payload: EventPayload,
    ) -> Result<EventId, StoreError> {
        let event_id = EventId::new();
        let event = ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id,
            session_id: self.session_id,
            turn_id,
            seq: self.seq_counter,
            ts: Utc::now(),
            payload,
        };
        self.store.append(&event).await?;
        self.seq_counter = self.seq_counter.saturating_add(1);
        self.history_cache.push(event);
        Ok(event_id)
    }
}

/// Bridge `EventRecorder` onto `StepRecorder` so that H11 trait
/// implementations (`Compactor`, `SystemPromptInjector`, `ToolFilterOverrider`)
/// can call the recorder via the protocol trait without a concrete dependency
/// on `cogito-core`.
///
/// The convenience methods (`record_system_prompt_injected`,
/// `record_tool_filter_overridden`) are overridden to delegate to the
/// checked concrete methods that enforce idempotency invariants.
#[async_trait]
impl EventRecorder for StepRecorder {
    async fn append_payload(
        &mut self,
        turn_id: TurnId,
        payload: EventPayload,
    ) -> Result<(EventId, u64), StoreError> {
        // The concrete `append` method does not return the seq. We track it
        // from the counter before the call.
        let seq = self.seq_counter;
        let event_id = self.append(Some(turn_id), payload).await?;
        Ok((event_id, seq))
    }

    /// Override with the idempotency-checked version from `StepRecorder`.
    async fn record_system_prompt_injected(
        &mut self,
        turn_id: TurnId,
        suffix: String,
        contributors: Vec<String>,
        produced_by: &str,
    ) -> Result<EventId, StoreError> {
        StepRecorder::record_system_prompt_injected(
            self,
            turn_id,
            suffix,
            contributors,
            produced_by,
        )
        .await
    }

    /// Override with the idempotency-checked version from `StepRecorder`.
    async fn record_tool_filter_overridden(
        &mut self,
        turn_id: TurnId,
        mode: ToolFilterOverrideMode,
        contributors: Vec<String>,
        produced_by: &str,
    ) -> Result<EventId, StoreError> {
        StepRecorder::record_tool_filter_overridden(self, turn_id, mode, contributors, produced_by)
            .await
    }

    /// Override with the invariant-enforcing version from `StepRecorder`.
    async fn record_context_compacted(
        &mut self,
        turn_id: TurnId,
        replaced_seq_range: (u64, u64),
        produced_by: &str,
        replacement: cogito_protocol::context::CompactionReplacement,
        estimates: cogito_protocol::context::TokenEstimates,
    ) -> Result<EventId, StoreError> {
        StepRecorder::record_context_compacted(
            self,
            turn_id,
            replaced_seq_range,
            produced_by,
            replacement,
            estimates,
        )
        .await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use cogito_store::JsonlStore;

    fn fresh_store_in(dir: &std::path::Path) -> Arc<dyn ConversationStore> {
        Arc::new(JsonlStore::new(dir.to_path_buf()))
    }

    #[tokio::test]
    async fn text_block_lifecycle_persists_one_event() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let turn = TurnId::new();
        rec.on_text_delta(turn, "hello ".into());
        rec.on_text_delta(turn, "world".into());
        // No store write yet — deltas are buffered until the block boundary.
        assert_eq!(store.latest_seq(sid).await?, None);

        rec.on_text_block_complete().await?;
        // Exactly one event persisted at seq 0.
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        Ok(())
    }

    #[tokio::test]
    async fn text_block_lifecycle_combines_full_text() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);
        let turn = TurnId::new();
        rec.on_text_delta(turn, "foo".into());
        rec.on_text_delta(turn, "bar".into());
        rec.on_text_block_complete().await?;

        // Single event landed at seq 0 with combined text "foobar".
        assert_eq!(store.latest_seq(sid).await?, Some(0));

        // Read the JSONL file directly to assert the combined wire shape.
        let mut entries = tokio::fs::read_dir(tmp.path()).await?;
        let mut session_files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            session_files.push(entry.path());
        }
        assert_eq!(
            session_files.len(),
            1,
            "expected exactly one session file, got {session_files:?}"
        );
        let text = tokio::fs::read_to_string(&session_files[0]).await?;
        assert!(
            text.contains(r#""text":"foobar""#),
            "expected combined text in file contents, got: {text}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn text_block_complete_without_deltas_is_noop() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);
        rec.on_text_block_complete().await?;
        assert_eq!(store.latest_seq(sid).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn record_methods_return_event_id() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let event_id_1 = rec
            .record_session_started(SessionMeta {
                cogito_version: "0.1.0".into(),
                ..Default::default()
            })
            .await?;
        let turn_id = TurnId::new();
        let event_id_2 = rec
            .record_turn_started(
                turn_id,
                vec![ContentBlock::Text { text: "go".into() }],
                vec![],
            )
            .await?;

        assert_ne!(event_id_1, event_id_2, "EventIds must be unique");
        Ok(())
    }

    #[tokio::test]
    async fn message_bearing_broadcasts_carry_turn_id() -> Result<(), Box<dyn std::error::Error>> {
        // ADR-0041: a live subscriber must be able to key TurnStarted /
        // TextDelta / TurnCompleted to the same turn_id the persisted log
        // carries, so it can fold streamed deltas into the right message.
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, mut rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let turn_id = TurnId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        rec.record_turn_started(
            turn_id,
            vec![ContentBlock::Text { text: "go".into() }],
            vec![],
        )
        .await?;
        rec.on_text_delta(turn_id, "hello".into());
        rec.record_turn_completed(
            turn_id,
            TurnOutcome::Completed,
            cogito_protocol::gateway::StopReason::EndTurn,
        )
        .await?;

        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::TurnStarted { turn_id: tid, .. } => assert_eq!(tid, Some(turn_id)),
            other => panic!("expected TurnStarted, got {other:?}"),
        }
        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::TextDelta {
                chunk,
                turn_id: tid,
                ..
            } => {
                assert_eq!(chunk, "hello");
                assert_eq!(tid, Some(turn_id));
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::TurnCompleted { turn_id: tid, .. } => assert_eq!(tid, Some(turn_id)),
            other => panic!("expected TurnCompleted, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn assistant_message_id_minted_and_stamped_live_and_persisted()
    -> Result<(), Box<dyn std::error::Error>> {
        // ADR-0041: record_model_call_started mints a MessageId, broadcasts it
        // on AssistantMessageStarted, and stamps the same id on the message's
        // live deltas and its persisted AssistantMessageAppended — so a live
        // subscriber and a history reader key the message identically.
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, mut rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let turn_id = TurnId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        rec.record_model_call_started(turn_id, "m".into()).await?;
        rec.on_text_delta(turn_id, "hi".into());
        let appended = rec.on_text_block_complete().await?;
        assert!(appended.is_some(), "expected an AssistantMessageAppended");

        // The message-open broadcast carries the minted id.
        #[allow(clippy::panic)]
        let message_id = match rx.try_recv()? {
            StreamEvent::AssistantMessageStarted {
                message_id,
                turn_id: tid,
                ..
            } => {
                assert_eq!(tid, Some(turn_id));
                message_id
            }
            other => panic!("expected AssistantMessageStarted, got {other:?}"),
        };
        // The text delta self-attributes to the same message.
        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::TextDelta {
                chunk,
                message_id: mid,
                ..
            } => {
                assert_eq!(chunk, "hi");
                assert_eq!(mid, Some(message_id), "delta must carry the message id");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        // The persisted AssistantMessageAppended carries the same id, so a
        // history projection keys the message identically to the live stream.
        let session_file = std::fs::read_dir(tmp.path())?
            .next()
            .ok_or("no session file")?
            .map_err(|e| format!("{e}"))?
            .path();
        let log = tokio::fs::read_to_string(session_file).await?;
        let needle = format!(r#""message_id":{}"#, serde_json::to_string(&message_id)?);
        assert!(
            log.contains("assistant_message_appended") && log.contains(&needle),
            "persisted AssistantMessageAppended must carry message_id {needle}: {log}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn thinking_block_flush_persists_thinking_block_recorded()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let (tx, mut rx) = broadcast::channel(64);
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let mut recorder = StepRecorder::new(Arc::clone(&store), tx, session_id, 0);

        recorder
            .record_session_started(SessionMeta {
                cogito_version: "0.1.0".into(),
                ..Default::default()
            })
            .await?;

        recorder.on_thinking_delta(turn_id, "I should ".into());
        recorder.on_thinking_delta(turn_id, "grep.".into());
        let id = recorder
            .on_thinking_block_complete(Some(serde_json::json!({"signature":"abc"})))
            .await?;
        assert!(id.is_some(), "expected an EventId from flush");

        // Two ThinkingDelta StreamEvents broadcast then nothing more, each
        // carrying the originating turn id for live/persisted correlation.
        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::ThinkingDelta {
                chunk,
                turn_id: tid,
                ..
            } => {
                assert_eq!(chunk, "I should ");
                assert_eq!(tid, Some(turn_id));
            }
            other => panic!("unexpected stream event: {other:?}"),
        }
        #[allow(clippy::panic)]
        match rx.try_recv()? {
            StreamEvent::ThinkingDelta {
                chunk,
                turn_id: tid,
                ..
            } => {
                assert_eq!(chunk, "grep.");
                assert_eq!(tid, Some(turn_id));
            }
            other => panic!("unexpected stream event: {other:?}"),
        }

        // Persisted shape: read the JSONL file and confirm the payload type.
        let session_file = std::fs::read_dir(tmp.path())?
            .next()
            .ok_or("no session file")?
            .map_err(|e| format!("{e}"))?
            .path();
        let text = tokio::fs::read_to_string(session_file).await?;
        assert!(
            text.contains("thinking_block_recorded"),
            "expected thinking_block_recorded line, got: {text}"
        );
        assert!(
            text.contains(r#""provider_opaque":{"signature":"abc"}"#),
            "expected provider_opaque payload preserved, got: {text}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn thinking_block_complete_with_no_buffered_deltas_is_noop()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let (tx, _rx) = broadcast::channel(64);
        let mut recorder = StepRecorder::new(Arc::clone(&store), tx, SessionId::new(), 0);
        let id = recorder.on_thinking_block_complete(None).await?;
        assert!(id.is_none(), "no buffered deltas → no event written");
        Ok(())
    }

    #[tokio::test]
    async fn seq_counter_is_monotonic() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        rec.record_session_started(SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        })
        .await?;
        let turn = TurnId::new();
        rec.record_turn_started(turn, vec![ContentBlock::Text { text: "hi".into() }], vec![])
            .await?;
        rec.record_turn_completed(
            turn,
            TurnOutcome::Completed,
            cogito_protocol::gateway::StopReason::EndTurn,
        )
        .await?;

        assert_eq!(store.latest_seq(sid).await?, Some(2));
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // record_context_compacted tests
    // ---------------------------------------------------------------------------

    /// Seed a minimal two-turn history:
    ///   seq 0  `SessionStarted`
    ///   seq 1  `TurnStarted`  (`turn_a`)
    ///   seq 2  `AssistantMessageAppended`
    ///   seq 3  `TurnCompleted`
    ///   seq 4  `TurnStarted`  (`turn_b`)
    ///   seq 5  `AssistantMessageAppended`
    ///   seq 6  `TurnCompleted`
    ///
    /// Returns `(recorder, turn_a_id, turn_b_id)`.
    async fn seed_two_turns(
        store: &Arc<dyn ConversationStore>,
        sid: SessionId,
    ) -> Result<(StepRecorder, TurnId, TurnId), Box<dyn std::error::Error>> {
        let (tx, _rx) = broadcast::channel(64);
        let mut rec = StepRecorder::new(Arc::clone(store), tx, sid, 0);

        rec.record_session_started(SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        })
        .await?;

        let turn_a = TurnId::new();
        rec.record_turn_started(
            turn_a,
            vec![ContentBlock::Text { text: "q1".into() }],
            vec![],
        )
        .await?;
        rec.on_text_delta(turn_a, "answer one".into());
        rec.on_text_block_complete().await?;
        rec.record_turn_completed(
            turn_a,
            TurnOutcome::Completed,
            cogito_protocol::gateway::StopReason::EndTurn,
        )
        .await?;

        let turn_b = TurnId::new();
        rec.record_turn_started(
            turn_b,
            vec![ContentBlock::Text { text: "q2".into() }],
            vec![],
        )
        .await?;
        rec.on_text_delta(turn_b, "answer two".into());
        rec.on_text_block_complete().await?;
        rec.record_turn_completed(
            turn_b,
            TurnOutcome::Completed,
            cogito_protocol::gateway::StopReason::EndTurn,
        )
        .await?;

        // Seqs: 0=SessionStarted 1=TurnStarted(a) 2=AssistantMsg 3=TurnCompleted
        //       4=TurnStarted(b) 5=AssistantMsg 6=TurnCompleted
        assert_eq!(store.latest_seq(sid).await?, Some(6));
        Ok((rec, turn_a, turn_b))
    }

    #[tokio::test]
    async fn record_context_compacted_writes_event() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let sid = SessionId::new();
        let (mut rec, turn_a, _turn_b) = seed_two_turns(&store, sid).await?;

        // Compact turn_a (seqs 1-3): range.0 = 1 (TurnStarted), range.1 = 3
        // (TurnCompleted). The next event is seq 4 (TurnStarted of turn_b) which
        // satisfies the boundary invariant.
        let event_id = rec
            .record_context_compacted(
                turn_a,
                (1, 3),
                "truncate",
                cogito_protocol::context::CompactionReplacement::Drop,
                cogito_protocol::context::TokenEstimates {
                    before: Some(1000),
                    after: Some(100),
                },
            )
            .await?;

        // Event id is non-zero and event landed at seq 7.
        let _ = event_id; // EventId is opaque; just assert no error above.
        assert_eq!(store.latest_seq(sid).await?, Some(7));
        Ok(())
    }

    #[tokio::test]
    async fn record_context_compacted_rejects_self_referential_range()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let sid = SessionId::new();
        let (mut rec, turn_a, _turn_b) = seed_two_turns(&store, sid).await?;

        // next_seq = 7; passing range.1 = 7 (equal to next_seq) should fail.
        let err = rec
            .record_context_compacted(
                turn_a,
                (1, 7),
                "truncate",
                cogito_protocol::context::CompactionReplacement::Drop,
                cogito_protocol::context::TokenEstimates::default(),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(
                err,
                cogito_protocol::store::StoreError::InvariantViolated(_)
            ),
            "expected InvariantViolated, got {err:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn record_context_compacted_rejects_non_turn_boundary_start()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let sid = SessionId::new();
        let (mut rec, _turn_a, turn_b) = seed_two_turns(&store, sid).await?;

        // Seq 2 is AssistantMessageAppended, not TurnStarted — must be rejected.
        let err = rec
            .record_context_compacted(
                turn_b,
                (2, 3),
                "truncate",
                cogito_protocol::context::CompactionReplacement::Drop,
                cogito_protocol::context::TokenEstimates::default(),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(
                err,
                cogito_protocol::store::StoreError::InvariantViolated(_)
            ),
            "expected InvariantViolated for non-TurnStarted range.0, got {err:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn record_context_compacted_rejects_duplicate_for_turn()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let sid = SessionId::new();
        let (mut rec, turn_a, _turn_b) = seed_two_turns(&store, sid).await?;

        // First compaction succeeds.
        rec.record_context_compacted(
            turn_a,
            (1, 3),
            "truncate",
            cogito_protocol::context::CompactionReplacement::Drop,
            cogito_protocol::context::TokenEstimates::default(),
        )
        .await?;

        // Second compaction for the same turn_id must be rejected.
        let err = rec
            .record_context_compacted(
                turn_a,
                (1, 3),
                "truncate",
                cogito_protocol::context::CompactionReplacement::Drop,
                cogito_protocol::context::TokenEstimates::default(),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(
                err,
                cogito_protocol::store::StoreError::InvariantViolated(_)
            ),
            "expected InvariantViolated for duplicate turn compaction, got {err:?}"
        );
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // record_system_prompt_injected tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn record_system_prompt_injected_first_call_writes()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let turn = TurnId::new();
        let id = rec
            .record_system_prompt_injected(
                turn,
                "Today is 2026-05-23.".into(),
                vec!["date".into()],
                "none",
            )
            .await?;
        // The event should have been persisted at seq 0.
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        let _ = id;
        Ok(())
    }

    #[tokio::test]
    async fn record_system_prompt_injected_idempotent_on_turn()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let turn = TurnId::new();
        let first = rec
            .record_system_prompt_injected(turn, "a".into(), vec![], "none")
            .await?;
        let second = rec
            .record_system_prompt_injected(turn, "b".into(), vec![], "none")
            .await?;

        assert_eq!(
            first, second,
            "must return same EventId, must not double-write"
        );
        // Only one event written.
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // record_tool_filter_overridden tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn record_tool_filter_overridden_idempotent_on_turn()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let turn = TurnId::new();
        let first = rec
            .record_tool_filter_overridden(turn, ToolFilterOverrideMode::Inherit, vec![], "none")
            .await?;
        let second = rec
            .record_tool_filter_overridden(
                turn,
                ToolFilterOverrideMode::Intersect {
                    tools: vec!["read_file".into()],
                },
                vec![],
                "none",
            )
            .await?;

        assert_eq!(
            first, second,
            "must return same EventId, must not double-write"
        );
        // Only one event written.
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // record_context_decision tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn record_context_decision_writes_summary() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(Arc::clone(&store), tx, sid, 0);

        let turn = TurnId::new();
        let sys_id = rec
            .record_system_prompt_injected(turn, String::new(), vec![], "none")
            .await?;
        let filter_id = rec
            .record_tool_filter_overridden(turn, ToolFilterOverrideMode::Inherit, vec![], "none")
            .await?;
        let decision_id = rec
            .record_context_decision(
                turn,
                vec![],
                sys_id,
                filter_id,
                ContextDecisionErrors::default(),
            )
            .await?;

        // Three events at seqs 0, 1, 2.
        assert_eq!(store.latest_seq(sid).await?, Some(2));
        let _ = decision_id;
        Ok(())
    }
}

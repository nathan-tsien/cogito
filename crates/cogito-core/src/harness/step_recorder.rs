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

use chrono::Utc;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::store::{ConversationStore, StoreError};
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolResult;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use tokio::sync::broadcast;

/// H02 Step Recorder. Persists [`ConversationEvent`]s to the
/// [`ConversationStore`] and fans out a parallel live [`StreamEvent`]
/// broadcast for subscribers (TUI, observability, consumer hooks).
///
/// Text deltas are buffered until [`StepRecorder::on_text_block_complete`]
/// to honor the H02 batching contract; every other event is persisted
/// immediately on its corresponding method.
pub struct StepRecorder {
    store: Arc<dyn ConversationStore>,
    events_tx: broadcast::Sender<StreamEvent>,
    session_id: SessionId,
    seq_counter: u64,
    current_text_block: Option<TextBlockBuf>,
}

/// In-flight text block accumulator. Filled by `on_text_delta` and drained
/// by `on_text_block_complete` into a single `AssistantMessageAppended`.
struct TextBlockBuf {
    turn_id: TurnId,
    text: String,
}

impl StepRecorder {
    /// Create a recorder bound to `session_id`. `seq_start` is the seq
    /// number to assign to the next appended event; pass `0` for a fresh
    /// session, or `latest_seq + 1` when resuming.
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
        }
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
    pub async fn record_turn_started(
        &mut self,
        turn_id: TurnId,
        user_input: Vec<ContentBlock>,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnStarted);
        self.append(Some(turn_id), EventPayload::TurnStarted { user_input })
            .await
    }

    /// Buffer a streaming text chunk and broadcast it live as
    /// [`StreamEvent::TextDelta`]. Does NOT persist — call
    /// [`StepRecorder::on_text_block_complete`] when the wire protocol
    /// signals the block is finished to flush the buffer as a single
    /// `AssistantMessageAppended` event.
    pub fn on_text_delta(&mut self, turn_id: TurnId, chunk: String) {
        let buf = self.current_text_block.get_or_insert_with(|| TextBlockBuf {
            turn_id,
            text: String::new(),
        });
        buf.text.push_str(&chunk);
        // Broadcast after the buffer push so the buffer is the source of
        // truth even if the channel has no live subscribers.
        let _ = self.events_tx.send(StreamEvent::TextDelta { chunk });
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
                EventPayload::AssistantMessageAppended { text: buf.text },
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
        let _ = self.events_tx.send(StreamEvent::ToolDispatchStarted {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
        });
        self.append(
            Some(turn_id),
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
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
        let ok = matches!(result, ToolResult::Output(_));
        let _ = self.events_tx.send(StreamEvent::ToolDispatchEnded {
            call_id: call_id.clone(),
            ok,
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
        let _ = self.events_tx.send(StreamEvent::TurnPaused);
        self.append(Some(turn_id), EventPayload::TurnPaused { job_id })
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
        let _ = self.events_tx.send(StreamEvent::TurnResumed);
        self.append(
            Some(turn_id),
            EventPayload::JobCompletedRecorded { job_id, outcome },
        )
        .await
    }

    /// Record successful turn completion and broadcast
    /// [`StreamEvent::TurnCompleted`].
    pub async fn record_turn_completed(
        &mut self,
        turn_id: TurnId,
        outcome: TurnOutcome,
    ) -> Result<EventId, StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnCompleted);
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
    pub async fn record_model_call_started(
        &mut self,
        turn_id: TurnId,
        model: String,
    ) -> Result<EventId, StoreError> {
        self.append(Some(turn_id), EventPayload::ModelCallStarted { model })
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
        });
        self.append(Some(turn_id), EventPayload::TurnFailed { reason })
            .await
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
        Ok(event_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use cogito_store_jsonl::JsonlStore;

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
            .record_turn_started(turn_id, vec![ContentBlock::Text { text: "go".into() }])
            .await?;

        assert_ne!(event_id_1, event_id_2, "EventIds must be unique");
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
        rec.record_turn_started(turn, vec![ContentBlock::Text { text: "hi".into() }])
            .await?;
        rec.record_turn_completed(turn, TurnOutcome::Completed)
            .await?;

        assert_eq!(store.latest_seq(sid).await?, Some(2));
        Ok(())
    }
}

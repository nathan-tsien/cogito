//! `ConversationStore` — Brain-facing persistence trait.
//!
//! See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §3 for the full method semantics. See ADR-0007 for why cross-session /
//! cross-tenant query methods do **not** belong on this trait.

use async_trait::async_trait;
use futures::stream::BoxStream;
use thiserror::Error;

use crate::event::ConversationEvent;
use crate::ids::SessionId;

/// Persistent backend for a session's `ConversationEvent` stream.
///
/// Implementations live in separate crates (`cogito-store-jsonl`,
/// `cogito-store-postgres` v0.4). The Runtime holds **one**
/// `Arc<dyn ConversationStore>` shared by all per-session loop tasks;
/// every method takes the session identifier explicitly.
///
/// Durability semantics are backend-defined; see each backend's crate
/// docs.
#[async_trait]
pub trait ConversationStore: Send + Sync + 'static {
    /// Append a single event. Backends MUST honor `event.seq` and MUST
    /// NOT reorder events. On `Err`, the backend's per-session state is
    /// considered tainted: callers SHOULD `close(session_id)` before
    /// further appends.
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError>;

    /// Flush backend-internal buffers for `session_id`. No-op for backends
    /// without buffering. JSONL flushes its `tokio::fs::File`.
    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Release per-session resources (file handles, connection slot).
    /// After `close`, subsequent `append` for the same session is valid
    /// and re-acquires resources.
    async fn close(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Largest `seq` ever appended for `session_id`, or `None` if no
    /// events exist. Used by Sprint 3's H03 Resume Coordinator.
    async fn latest_seq(&self, session_id: SessionId) -> Result<Option<u64>, StoreError>;

    /// Stream events where `event.seq > from_seq`, in strict ascending
    /// `seq` order. Use `from_seq = 0` to read from the second event
    /// onward; use the result of `latest_seq` (i.e. the last persisted
    /// seq) to read net-new events after a resume — passing `from_seq = N`
    /// returns events with `seq > N`.
    fn replay(
        &self,
        session_id: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>>;
}

/// Errors returned by `ConversationStore` methods.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The requested session has no recorded events.
    #[error("session not found: {session_id}")]
    SessionNotFound {
        /// Identifier of the missing session.
        session_id: SessionId,
    },

    /// Underlying I/O failure.
    #[error("backend io error: {source}")]
    Io {
        /// The wrapped I/O error.
        #[from]
        source: std::io::Error,
    },

    /// JSON serialization or parsing failure.
    #[error("serde error: {source}")]
    Serde {
        /// The wrapped serde error.
        #[from]
        source: serde_json::Error,
    },

    /// Schema version of the persisted event is higher than this build
    /// understands. Reader cannot safely process the event.
    #[error("schema version {found} not supported; this build understands <= {supported}")]
    UnsupportedSchemaVersion {
        /// The version found on disk.
        found: u32,
        /// The maximum version this build supports.
        supported: u32,
    },

    /// Backend-specific error with a human-readable message.
    #[error("backend error: {message}")]
    Backend {
        /// Human-readable detail.
        message: String,
    },

    /// A write was rejected because a domain invariant would be violated.
    ///
    /// Used by `StepRecorder::record_context_compacted` to enforce the
    /// §5.5 boundary invariants (seq range alignment, uniqueness per turn).
    #[error("invariant violated: {0}")]
    InvariantViolated(String),
}

/// Write-side abstraction used by Context-Management trait implementations
/// (H11 Compactor, `SystemPromptInjector`, `ToolFilterOverrider`) to persist
/// events without taking a direct reference to `cogito_core::StepRecorder`.
///
/// Implementations bridge this trait to `StepRecorder::append` (or an
/// equivalent test double). The interface is intentionally minimal: callers
/// supply a fully-constructed `EventPayload`; the recorder assigns `seq`,
/// `ts`, `event_id`, and the session/turn envelope.
///
/// `EventRecorder` is dyn-safe and `Send + Sync` so it can be injected as
/// a `&mut dyn EventRecorder` across await points.
#[async_trait]
pub trait EventRecorder: Send {
    /// Persist one event payload for the given turn. Returns the `EventId`
    /// assigned to the newly-written event and its monotonic `seq` number.
    async fn append_payload(
        &mut self,
        turn_id: crate::ids::TurnId,
        payload: crate::event::EventPayload,
    ) -> Result<(crate::ids::EventId, u64), StoreError>;

    /// Persist a `SystemPromptInjected` event for `turn_id`.
    ///
    /// Default implementation builds the payload and delegates to
    /// [`EventRecorder::append_payload`]. Overriding implementations
    /// (e.g. `StepRecorder`) may add idempotency checks.
    async fn record_system_prompt_injected(
        &mut self,
        turn_id: crate::ids::TurnId,
        suffix: String,
        contributors: Vec<String>,
        produced_by: &str,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                turn_id,
                crate::event::EventPayload::SystemPromptInjected {
                    turn_id,
                    suffix,
                    contributors,
                    produced_by: produced_by.to_owned(),
                },
            )
            .await?;
        Ok(id)
    }

    /// Persist a `ToolFilterOverridden` event for `turn_id`.
    ///
    /// Default implementation builds the payload and delegates to
    /// [`EventRecorder::append_payload`]. Overriding implementations
    /// (e.g. `StepRecorder`) may add idempotency checks.
    async fn record_tool_filter_overridden(
        &mut self,
        turn_id: crate::ids::TurnId,
        mode: crate::context::ToolFilterOverrideMode,
        contributors: Vec<String>,
        produced_by: &str,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                turn_id,
                crate::event::EventPayload::ToolFilterOverridden {
                    turn_id,
                    mode,
                    contributors,
                    produced_by: produced_by.to_owned(),
                },
            )
            .await?;
        Ok(id)
    }

    /// Persist a `JobSubmitted` event for the given turn.
    ///
    /// Default implementation builds the payload and delegates to
    /// [`EventRecorder::append_payload`]. Overriding implementations
    /// (e.g. `StepRecorder`) may add side effects such as broadcasting
    /// a live [`crate::stream::StreamEvent`].
    async fn record_job_submitted(
        &mut self,
        turn_id: crate::ids::TurnId,
        call_id: String,
        job_id: crate::job::JobId,
        tool_name: String,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                turn_id,
                crate::event::EventPayload::JobSubmitted {
                    call_id,
                    job_id,
                    tool_name,
                },
            )
            .await?;
        Ok(id)
    }

    /// Persist a `SkillActivated` event. Default impl routes through
    /// `append_payload`; backends needing tighter integration may override.
    async fn record_skill_activated(
        &mut self,
        turn_id: crate::ids::TurnId,
        skill_name: String,
        source: crate::skill::SkillSource,
        channel: crate::skill::SkillActivationChannel,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                turn_id,
                crate::event::EventPayload::SkillActivated {
                    skill_name,
                    source,
                    channel,
                },
            )
            .await?;
        Ok(id)
    }

    /// Persist a `ContextCompacted` event for `turn_id`.
    ///
    /// Default implementation builds the payload and delegates to
    /// [`EventRecorder::append_payload`]. Overriding implementations
    /// (e.g. `StepRecorder`) enforce §5.5 boundary invariants.
    async fn record_context_compacted(
        &mut self,
        turn_id: crate::ids::TurnId,
        replaced_seq_range: (u64, u64),
        produced_by: &str,
        replacement: crate::context::CompactionReplacement,
        estimates: crate::context::TokenEstimates,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                turn_id,
                crate::event::EventPayload::ContextCompacted {
                    turn_id,
                    replaced_seq_range,
                    produced_by: produced_by.to_owned(),
                    replacement,
                    token_estimate_before: estimates.before,
                    token_estimate_after: estimates.after,
                },
            )
            .await?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check that `ConversationStore` is dyn-safe.
    #[test]
    fn trait_is_dyn_safe() {
        fn assert_dyn_safe(_: &dyn ConversationStore) {}
        // No instance needed; this only checks the trait constructs
        // a valid `dyn` type. The body executes only if called.
        let _ = assert_dyn_safe;
    }

    #[test]
    fn store_error_displays_session_not_found() {
        let sid = SessionId::new();
        let err = StoreError::SessionNotFound { session_id: sid };
        let text = err.to_string();
        assert!(text.starts_with("session not found:"));
    }
}

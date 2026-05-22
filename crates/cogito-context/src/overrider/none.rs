//! `NoneOverrider` -- no-op `ToolFilterOverrider` that inherits the strategy filter.

use async_trait::async_trait;
use cogito_protocol::context::{
    ContextError, ToolFilterInput, ToolFilterOverrideMode, ToolFilterOverrider,
};
use cogito_protocol::ids::EventId;
use cogito_protocol::store::EventRecorder;

/// `ToolFilterOverrider` that always emits `ToolFilterOverrideMode::Inherit`.
///
/// Writes a `ToolFilterOverridden` event with `mode = Inherit` and
/// `produced_by = "none"` every turn. The event recorder's default impl
/// provides idempotency when backed by `StepRecorder`.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneOverrider;

#[async_trait]
impl ToolFilterOverrider for NoneOverrider {
    async fn override_filter(&self, input: ToolFilterInput<'_>) -> Result<EventId, ContextError> {
        let event_id = EventRecorder::record_tool_filter_overridden(
            input.recorder,
            input.turn_id,
            ToolFilterOverrideMode::Inherit,
            vec![],
            "none",
        )
        .await?;
        Ok(event_id)
    }

    fn id(&self) -> &'static str {
        "none"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::event::EventPayload;
    use cogito_protocol::exec_ctx::ExecCtx;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;
    use cogito_test_fixtures::context::InMemoryRecorder;

    #[tokio::test]
    async fn none_overrider_writes_inherit_tool_filter_overridden() {
        let mut recorder = InMemoryRecorder::default();
        let strategy = HarnessStrategy::default_with_model("test");
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
        let input = ToolFilterInput {
            session_id,
            turn_id,
            strategy: &strategy,
            history: &[],
            exec_ctx: &exec_ctx,
            recorder: &mut recorder,
        };
        let event_id = NoneOverrider.override_filter(input).await.unwrap();
        assert_eq!(recorder.events.len(), 1, "must write exactly one event");
        let (_, payload) = &recorder.events[0];
        match payload {
            EventPayload::ToolFilterOverridden {
                turn_id: t,
                mode,
                contributors,
                produced_by,
            } => {
                assert_eq!(*t, turn_id);
                assert!(
                    matches!(mode, ToolFilterOverrideMode::Inherit),
                    "mode must be Inherit for NoneOverrider"
                );
                assert!(contributors.is_empty());
                assert_eq!(produced_by, "none");
            }
            other => panic!("expected ToolFilterOverridden, got {other:?}"),
        }
        let _ = event_id;
    }
}

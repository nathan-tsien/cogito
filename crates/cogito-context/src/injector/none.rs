//! `NoneInjector` -- no-op `SystemPromptInjector` that writes an empty suffix.

use async_trait::async_trait;
use cogito_protocol::context::{ContextError, InjectionInput, SystemPromptInjector};
use cogito_protocol::ids::EventId;
use cogito_protocol::store::EventRecorder;

/// `SystemPromptInjector` that always injects an empty suffix.
///
/// Writes a `SystemPromptInjected` event with `suffix = ""` and
/// `produced_by = "none"` every turn. The event recorder's default impl
/// provides idempotency when backed by `StepRecorder`.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneInjector;

#[async_trait]
impl SystemPromptInjector for NoneInjector {
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError> {
        let event_id = EventRecorder::record_system_prompt_injected(
            input.recorder,
            input.turn_id,
            String::new(),
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
    async fn none_injector_writes_empty_system_prompt_injected() {
        let mut recorder = InMemoryRecorder::default();
        let strategy = HarnessStrategy::default_with_model("test");
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
        let input = InjectionInput {
            session_id,
            turn_id,
            strategy: &strategy,
            history: &[],
            exec_ctx: &exec_ctx,
            recorder: &mut recorder,
        };
        let event_id = NoneInjector.inject(input).await.unwrap();
        assert_eq!(recorder.events.len(), 1, "must write exactly one event");
        let (_, payload) = &recorder.events[0];
        match payload {
            EventPayload::SystemPromptInjected {
                turn_id: t,
                suffix,
                contributors,
                produced_by,
            } => {
                assert_eq!(*t, turn_id);
                assert!(suffix.is_empty(), "suffix must be empty for NoneInjector");
                assert!(contributors.is_empty());
                assert_eq!(produced_by, "none");
            }
            other => panic!("expected SystemPromptInjected, got {other:?}"),
        }
        let _ = event_id;
    }
}

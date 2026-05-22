//! `NoneCompactor` -- no-op `Compactor` that never compacts.

use async_trait::async_trait;
use cogito_protocol::context::{CompactionApplied, CompactionInput, Compactor, ContextError};

/// `Compactor` that always returns `Ok(vec![])`.
///
/// Use this when the harness strategy requires no compaction (the default).
/// H11 drives the compaction pipeline with this implementation when
/// `CompactorConfig::None` is selected.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneCompactor;

#[async_trait]
impl Compactor for NoneCompactor {
    async fn maybe_compact(
        &self,
        _input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError> {
        Ok(vec![])
    }

    fn id(&self) -> &'static str {
        "none"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;
    use cogito_test_fixtures::context::{DummyGateway, InMemoryRecorder};

    #[tokio::test]
    async fn none_compactor_returns_empty_vec() {
        let mut recorder = InMemoryRecorder::default();
        let strategy = HarnessStrategy::default_with_model("test");
        let gateway = DummyGateway;
        let input = CompactionInput {
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            history: &[],
            strategy: &strategy,
            last_usage: None,
            model_gateway: &gateway,
            recorder: &mut recorder,
        };
        let result = NoneCompactor.maybe_compact(input).await.unwrap();
        assert!(result.is_empty(), "NoneCompactor must return empty vec");
        // NoneCompactor writes no events.
        assert!(
            recorder.events.is_empty(),
            "NoneCompactor must not write any events"
        );
    }
}

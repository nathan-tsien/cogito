//! Chaos: a hook that panics in `pre_prompt` produces a `HookRejected`
//! turn outcome, never an uncaught panic that crashes the FSM task.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::NoOpMetricsRecorder;
use cogito_protocol::gateway::{ModelEvent, ModelInput, StopReason, Usage};
use cogito_protocol::hook::{HookDecision, HookHandler};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use cogito_store_jsonl::JsonlStore;
use cogito_tools::provider::BuiltinToolProvider;
use tokio::sync::{Mutex, broadcast};

struct PanicInPrePrompt;
impl HookHandler for PanicInPrePrompt {
    fn name(&self) -> &'static str {
        "panicky"
    }
    fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
        panic!("intentional panic in test")
    }
}

#[tokio::test]
async fn hook_panic_in_pre_prompt_yields_turn_failed() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = tempfile::tempdir()?;
    let store: Arc<dyn cogito_protocol::store::ConversationStore> =
        Arc::new(JsonlStore::new(tmp_dir.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let (tx, _rx) = broadcast::channel(64);
    let recorder = Arc::new(Mutex::new(StepRecorder::new(
        Arc::clone(&store),
        tx,
        session_id,
        0,
    )));

    let mock: Arc<MockModelGateway> = Arc::new(MockModelGateway::new());
    // The model is never called — pre_prompt panics first. Push an
    // empty reply to satisfy the gateway contract just in case.
    mock.push_reply(vec![ModelEvent::MessageCompleted {
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    }]);

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> =
        Arc::new(BuiltinToolProvider::builder().build());

    let hooks = Arc::new(CompositeHookPipeline::with_handlers(vec![
        Arc::new(PanicInPrePrompt) as Arc<dyn HookHandler>,
    ]));

    let deps = TurnDeps {
        step: Arc::clone(&recorder),
        store: Arc::clone(&store),
        model: Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>,
        tools,
        hooks,
        metrics: Arc::new(NoOpMetricsRecorder),
        context_pipeline: Arc::new(cogito_context::build_pipeline(
            &cogito_protocol::context::ContextConfig::default(),
        )),
    };

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy: HarnessStrategy::default_with_model("mock"),
        consecutive_tool_errors: 0,
    };

    let outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;

    // TurnOutcome::Failed carries both `reason` and `recorded_event_id`.
    // We match on `reason` only and ignore the event id.
    match outcome {
        TurnOutcome::Failed {
            reason: TurnFailureReason::HookRejected { hook_name, message },
            ..
        } => {
            assert_eq!(hook_name, "panicky");
            assert!(
                message.contains("panicky") && message.contains("intentional panic in test"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected Failed(HookRejected), got {other:?}"),
    }
    Ok(())
}

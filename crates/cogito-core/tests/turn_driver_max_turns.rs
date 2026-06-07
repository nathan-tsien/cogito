//! E2E integration test: the per-turn iteration budget (ADR-0038).
//!
//! A model that keeps emitting a `tool_use` block never terminates on its
//! own. With `strategy.max_turns = 2`, the FSM must stop after two model
//! calls and return `TurnOutcome::Failed { MaxTurnsExceeded { turns: 2 } }`
//! rather than issuing a third model call.

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_jobs::LocalJobManager;
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::NoOpMetricsRecorder;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use cogito_store::JsonlStore;
use cogito_tools::ReadFile;
use cogito_tools::provider::BuiltinToolProvider;
use tokio::sync::{Mutex, broadcast, mpsc};

/// One scripted model reply that asks to call `read_file` and ends the
/// message with `stop_reason = ToolUse`, so the FSM dispatches the tool and
/// loops back for another model call.
fn tool_call_reply(call_id: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: call_id.into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: call_id.into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({ "path": "/nonexistent" }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 3,
            },
        },
    ]
}

#[tokio::test]
async fn turn_fails_when_max_turns_budget_is_exhausted() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store: Arc<dyn cogito_protocol::store::ConversationStore> =
        Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();

    let (tx, _rx) = broadcast::channel(64);
    let recorder = Arc::new(Mutex::new(StepRecorder::new(
        Arc::clone(&store),
        tx,
        session_id,
        0,
    )));

    // The model always asks for another tool call, so only the budget can
    // stop it. Push more replies than the budget allows so exhaustion of the
    // mock is never the reason the turn ends.
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(tool_call_reply("c1"));
    mock.push_reply(tool_call_reply("c2"));
    mock.push_reply(tool_call_reply("c3"));
    mock.push_reply(tool_call_reply("c4"));

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let (job_completion_tx, _job_completion_rx) = mpsc::channel(32);
    let deps = TurnDeps {
        step: Arc::clone(&recorder),
        store: Arc::clone(&store),
        model: mock,
        tools,
        hooks: Arc::new(CompositeHookPipeline::default()),
        metrics: Arc::new(NoOpMetricsRecorder),
        context_pipeline: Arc::new(cogito_context::build_pipeline(
            &cogito_protocol::context::ContextConfig::default(),
        )),
        skills: None,
        job_mgr: LocalJobManager::new(),
        job_completion_tx,
    };

    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.max_turns = 2;

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy,
        consecutive_tool_errors: 0,
        model_calls: 0,
    };

    let outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;
    assert!(
        matches!(
            outcome,
            TurnOutcome::Failed {
                reason: TurnFailureReason::MaxTurnsExceeded { turns: 2 },
                ..
            }
        ),
        "expected Failed(MaxTurnsExceeded {{ turns: 2 }}), got {outcome:?}"
    );
    Ok(())
}

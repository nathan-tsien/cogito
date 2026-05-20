//! E2E integration test: text-only turn reaches `Completed`.
//!
//! The mock model emits a single text block and then `MessageCompleted`
//! with `stop_reason = EndTurn`. The FSM must traverse
//! `Init` -> `ContextManaged` -> `PromptBuilt` -> `ModelCalling` -> `ModelCompleted`
//! -> `Completed` and return `TurnOutcome::Completed`.

use std::sync::Arc;

use cogito_core::harness::hooks::HookPipeline;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::TurnOutcome;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::ReadFile;
use cogito_tools::provider::BuiltinToolProvider;
use tokio::sync::{Mutex, broadcast};

#[tokio::test]
async fn text_only_turn_reaches_completed() -> Result<(), Box<dyn std::error::Error>> {
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

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "Hello!".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "Hello!".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 3,
            },
        },
    ]);

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let deps = TurnDeps {
        step: Arc::clone(&recorder),
        store: Arc::clone(&store),
        model: mock,
        tools,
        hooks: HookPipeline::new(),
    };

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy: HarnessStrategy::default_with_model("mock"),
        consecutive_tool_errors: 0,
    };

    let outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "expected Completed, got {outcome:?}"
    );
    Ok(())
}

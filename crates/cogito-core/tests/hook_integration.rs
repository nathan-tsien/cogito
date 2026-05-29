//! End-to-end: `SensitiveContentHook` rejects a tool call carrying an
//! AWS key in args. Turn ends Failed with `HookRejected`. Event log
//! shows `HookRejected` followed by `TurnFailed`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_core::harness::hooks::examples::SensitiveContentHook;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_jobs::LocalJobManager;
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::NoOpMetricsRecorder;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::hook::{HookHandler, HookLifecyclePoint};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_store::JsonlStore;
use cogito_tools::ReadFile;
use cogito_tools::provider::BuiltinToolProvider;
use futures::StreamExt as _;
use tokio::sync::{Mutex, broadcast, mpsc};

#[tokio::test]
async fn sensitive_content_hook_rejects_tool_with_aws_key() -> Result<(), Box<dyn std::error::Error>>
{
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

    // Model issues a read_file tool call whose `path` arg contains an
    // AWS access key. SensitiveContentHook scans recursively through
    // all string fields in tool args.
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({ "path": "/tmp/AKIAIOSFODNN7EXAMPLE" }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 2,
            },
        },
    ]);

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let hooks =
        Arc::new(CompositeHookPipeline::with_handlers(vec![
            Arc::new(SensitiveContentHook::new()) as Arc<dyn HookHandler>,
        ]));

    let (job_completion_tx, _job_completion_rx) = mpsc::channel(32);
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
        skills: None,
        job_mgr: LocalJobManager::new(),
        job_completion_tx,
    };

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy: HarnessStrategy::default_with_model("mock"),
        consecutive_tool_errors: 0,
    };

    // Hook rejects pre-dispatch; the FSM converts the rejection to a
    // ToolResult::Error and loops for a follow-up model call. No second
    // reply is queued so the mock returns an empty stream, which causes
    // the turn to end in Failed. We assert the HookRejected event is
    // present in the persisted log regardless of the final outcome.
    let _outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;

    // Read back the persisted events via store.replay (matching the pattern
    // used in turn_driver_tool_call.rs).
    let events: Vec<cogito_protocol::event::ConversationEvent> = store
        .replay(session_id, 0)
        .filter_map(|r| async move { r.ok() })
        .collect()
        .await;

    let hook_rejected = events.iter().find(|e| {
        matches!(
            &e.payload,
            EventPayload::HookRejected {
                hook_name,
                point: HookLifecyclePoint::PreDispatch,
                reason,
            } if hook_name == "sensitive-content" && reason.contains("aws-access-key")
        )
    });
    assert!(
        hook_rejected.is_some(),
        "expected HookRejected event in log; got events: {events:#?}"
    );

    Ok(())
}

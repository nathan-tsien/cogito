//! E2E integration test: tool-call round-trip reaches `Completed`.
//!
//! First model call emits one `ToolUse` block (`read_file`). The FSM dispatches
//! it, then re-enters `Init` for the second model call which returns a text
//! block and `stop_reason = EndTurn`. The FSM must return
//! `TurnOutcome::Completed` and the mock model must have zero scripts left.

use std::sync::Arc;

use cogito_core::harness::hooks::HookPipeline;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::TurnOutcome;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::ReadFile;
use cogito_tools::provider::BuiltinToolProvider;
use futures::StreamExt as _;
use tokio::sync::{Mutex, broadcast};

#[tokio::test]
async fn tool_call_completes_via_second_model_call() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_file = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp_file.path(), "answer 42")?;

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

    // First call: model issues a read_file tool call.
    let path_str = tmp_file
        .path()
        .to_str()
        .ok_or("non-utf8 temp path")?
        .to_owned();
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
            args: serde_json::json!({ "path": path_str }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 2,
            },
        },
    ]);

    // Second call: model responds with text after seeing the tool result.
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "done".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "done".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
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
        model: Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>,
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
    assert_eq!(
        mock.remaining(),
        0,
        "mock model should have no scripts left"
    );
    Ok(())
}

/// When the model emits a tool call whose args fail JSON Schema validation
/// (H07), the error must be persisted as a `ToolResultRecorded` event
/// before the second model call.  Without this, providers that enforce
/// strict `tool_call_id` pairing (e.g. `SenseNova`) return 400.
#[tokio::test]
async fn invalid_tool_args_persist_error_result() -> Result<(), Box<dyn std::error::Error>> {
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

    // First call: model issues read_file with MISSING required `path` arg.
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c_bad".into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c_bad".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({}), // missing required `path`
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        },
    ]);

    // Second call: model retries with a text response (no more tool calls).
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "sorry, need a path".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "sorry, need a path".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
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
        model: Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>,
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
        "expected Completed (model retried after validation error), got {outcome:?}"
    );
    assert_eq!(mock.remaining(), 0, "both mock scripts must be consumed");

    // Verify the event log contains a ToolResultRecorded for the bad call.
    let events: Vec<cogito_protocol::event::ConversationEvent> = store
        .replay(session_id, 0)
        .filter_map(|r| async move { r.ok() })
        .collect()
        .await;
    let has_tool_result = events.iter().any(|e| {
        matches!(&e.payload, EventPayload::ToolResultRecorded { call_id, .. } if call_id == "c_bad")
    });
    assert!(
        has_tool_result,
        "ToolResultRecorded for c_bad must be in event log"
    );
    Ok(())
}

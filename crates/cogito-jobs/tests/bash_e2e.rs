//! End-to-end: a model turn emits one `bash` `tool_use`; the turn runs the
//! command through the real [`DirectExecutor`] and completes. Mirrors
//! `run_tests_happy_path.rs` but with a synchronous `echo` command.
//!
//! Shape:
//! 1. Build a `Runtime` wired with `LocalJobManager`, `MockModelGateway`,
//!    and a single-tool [`BashTool`] backed by the real `DirectExecutor`.
//! 2. Submit a user turn. Turn 1's mock reply emits one `tool_use(bash)`
//!    block; turn 2's reply emits a final text and `end_turn`.
//! 3. Wait for `TurnCompleted` on the broadcast (synchronous `echo`, so the
//!    30 s ceiling is generous).
//!
//! Assertion: a `ToolResultRecorded` event is persisted with
//! `result = ToolResult::Output([{...}])` whose `stdout` contains `e2e`,
//! proving the command ran through `DirectExecutor` and the payload made
//! it back through the async loop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{BashConfig, BashTool, LocalJobManager};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::command::CommandExecutor;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolProvider, ToolResult};
use cogito_sandbox::{DirectConfig, DirectExecutor};
use cogito_store::JsonlStore;
use futures::StreamExt as _;

/// Terminal outcome of the broadcast-stream wait loop. Kept at module
/// scope to avoid `clippy::items_after_statements` inside the test body.
#[derive(Debug)]
enum Outcome {
    /// `StreamEvent::TurnCompleted` arrived. Happy path.
    Completed,
    /// `StreamEvent::TurnFailed` arrived before completion.
    Failed,
    /// The broadcast sender closed before any terminal event.
    StreamClosed,
    /// Wall-clock budget elapsed before any terminal event.
    Timeout,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bash_echo_completes_through_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let executor: Arc<dyn CommandExecutor> = Arc::new(DirectExecutor::new(DirectConfig::default()));
    let bash: Arc<dyn ToolProvider> = Arc::new(BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        BashConfig::default(),
    ));

    let mock = Arc::new(MockModelGateway::new());
    mock.script_tool_then_text("bash", serde_json::json!({ "command": "echo e2e" }), "done");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn ModelGateway>)
        .tools(bash)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle.submit_user_text("run echo").await?;

    // 30 s ceiling: synchronous `echo` is near-instant; the budget only
    // guards against a stuck async-job loop. Distinguish the four terminal
    // cases (see `Outcome`) so a failure mode reports specifically rather
    // than collapsing into one boolean.
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return Outcome::Completed,
                Ok(StreamEvent::TurnFailed { .. }) => return Outcome::Failed,
                Ok(_) => {}
                Err(_) => return Outcome::StreamClosed,
            }
        }
    })
    .await
    .unwrap_or(Outcome::Timeout);
    assert!(
        matches!(outcome, Outcome::Completed),
        "expected TurnCompleted within 30s, got {outcome:?} — the bash command \
         either failed, the broadcast stream closed early, or the async-job loop \
         did not drive the resumed turn"
    );

    handle.shutdown(Duration::from_secs(10)).await?;

    // Both scripted replies must have been consumed: turn 1 produced the
    // `bash` tool call, turn 2 produced the final text.
    assert_eq!(
        mock.remaining(),
        0,
        "expected both scripted model replies to be consumed; {} remain",
        mock.remaining()
    );

    // Replay the JSONL log and find the `ToolResultRecorded` event that
    // carries the command output back to the model.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };
    let tool_result = log
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("ToolResultRecorded missing from persisted log");

    match tool_result {
        ToolResult::Output(blocks) => {
            assert!(
                !blocks.is_empty(),
                "BashTool must emit a non-empty Output payload"
            );
            let first = &blocks[0];
            let stdout = first
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            assert!(
                stdout.contains("e2e"),
                "expected `echo e2e` output in captured stdout; full payload: {first:?}"
            );
        }
        other => panic!("expected ToolResult::Output, got {other:?}"),
    }

    Ok(())
}

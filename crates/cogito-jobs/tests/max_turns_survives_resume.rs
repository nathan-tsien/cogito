//! ADR-0038: the iteration budget must survive async pause/resume.
//!
//! Each model call here asks for the async `sleep` tool, so every inner-loop
//! iteration pauses the turn (`TurnPaused`) and resumes via the job-completion
//! path — re-spawning the `TurnDriver` task and losing the in-memory model-call
//! counter. If the counter is not re-derived from the event log on resume, the
//! budget resets every pause and the turn never stops. With `max_turns = 2`,
//! the turn must end `Failed(MaxTurnsExceeded { turns: 2 })` after exactly two
//! model calls, proving the count is reconstructed across the pause boundary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{LocalJobManager, SleepTool};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::{ModelEvent, ModelGateway, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_protocol::turn::TurnFailureReason;
use cogito_store::JsonlStore;
use futures::StreamExt as _;

/// One scripted reply that calls the async `sleep` tool and ends the message
/// with `stop_reason = ToolUse`, so the turn pauses on the job and loops back
/// for another model call when the sleep completes.
fn sleep_reply(call_id: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: call_id.into(),
            tool_name: "sleep".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: call_id.into(),
            tool_name: "sleep".into(),
            args: serde_json::json!({ "duration_ms": 10 }),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_turns_budget_survives_async_pause_resume() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let sleep_tool: Arc<dyn ToolProvider> = Arc::new(SleepTool::new(
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>
    ));

    // The model always asks for another sleep — only the budget can stop it.
    // Push more replies than the budget allows so mock exhaustion is never the
    // reason the turn ends within the budget window.
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(sleep_reply("c1"));
    mock.push_reply(sleep_reply("c2"));
    mock.push_reply(sleep_reply("c3"));
    mock.push_reply(sleep_reply("c4"));

    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.max_turns = 2;

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn ModelGateway>)
        .tools(sleep_tool)
        .strategy(strategy)
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle.submit_user_text("please loop").await?;

    // Wait for a terminal turn event. The budget should produce TurnFailed
    // after two model calls; a hang or a TurnCompleted would both be bugs.
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnFailed { .. }) => return Some("failed"),
                Ok(StreamEvent::TurnCompleted { .. }) => return Some("completed"),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .unwrap_or(None);
    assert_eq!(
        terminal,
        Some("failed"),
        "expected the turn to end in TurnFailed (budget hit), got {terminal:?}"
    );

    handle.shutdown(Duration::from_secs(5)).await?;

    // The budget allows exactly two model calls before failing, so two of the
    // four scripted replies must remain unconsumed.
    assert_eq!(
        mock.remaining(),
        2,
        "expected exactly two model calls before the budget stopped the turn; \
         {} replies remain",
        mock.remaining()
    );

    // The persisted log must carry a TurnFailed with MaxTurnsExceeded { turns: 2 }.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };
    let reason = log
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::TurnFailed { reason } => Some(reason.clone()),
            _ => None,
        })
        .expect("TurnFailed missing from persisted log");
    assert_eq!(
        reason,
        TurnFailureReason::MaxTurnsExceeded { turns: 2 },
        "budget must be re-derived across resume; got {reason:?}"
    );

    Ok(())
}

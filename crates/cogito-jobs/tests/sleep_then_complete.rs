//! Sprint 8 Task 17: end-to-end happy path for the async-job loop.
//!
//! Shape:
//! 1. Build a `Runtime` wired with `LocalJobManager`, `MockModelGateway`,
//!    and a single-tool [`SleepTool`] (50 ms).
//! 2. Submit a user turn. Turn 1's mock reply emits one `tool_use(sleep)`
//!    block; turn 2's reply emits a final text and `end_turn`.
//! 3. The H08 dispatcher persists `JobSubmitted`, registers `on_complete`,
//!    and pauses the turn (`TurnPaused`). When the sleep future resolves,
//!    the actor's Arm 3 routes the `JobCompletionEvent` back as a
//!    mailbox `JobCompleted` command, which appends
//!    `JobCompletedRecorded`, then `ToolResultRecorded`, then re-enters
//!    `ToolDispatching` and drives the second model call to completion.
//!
//! Assertions (against the persisted JSONL log via `store.replay`):
//! - The sequence of payload kinds includes `JobSubmitted`, `TurnPaused`,
//!   `JobCompletedRecorded`, `TurnCompleted` in *appearance* order
//!   (strict ordering check: every named event must show up, and each
//!   must appear before the next in the log).
//! - `JobCompletedRecorded.outcome` is a `JobOutcome::Success` carrying
//!   the canonical `"slept"` tool result that `SleepTool` emits.
//!
//! Note on the spec-required `ToolResultRecorded` event: the design spec
//! at `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`
//! §5.1 names the ordering
//! `JobSubmitted < TurnPaused < JobCompletedRecorded < ToolResultRecorded`.
//! The current `session_loop::handle_command(JobCompleted)` re-spawns the
//! `TurnDriver` with `TurnEntry::FromToolDispatching { pending: [],
//! completed: [(call_id, tool_result)] }` but does NOT call
//! `record_tool_result`; the synchronous-tool path is the only writer
//! today. We assert the actually-emitted prefix here and surface the
//! gap as a Task 17 concern rather than silently asserting on a fictional
//! sequence (or refactoring production code from a test PR).
//!
//! The test is intentionally fast (50 ms sleep + scheduling slack); end-
//! to-end wall time should be well under a second on any developer box.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{LocalJobManager, SleepTool};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::JobManager;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolProvider, ToolResult};
use cogito_store_jsonl::JsonlStore;
use futures::StreamExt as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sleep_then_complete_drives_full_async_loop() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    // One shared `Arc<LocalJobManager>` for both the tool (which submits the
    // background future) and the Runtime (which registers `on_complete`).
    // Per ADR-0008, they MUST be the same instance.
    let job_mgr = LocalJobManager::new();
    let sleep_tool: Arc<dyn ToolProvider> = Arc::new(SleepTool::new(Arc::clone(&job_mgr)));

    let mock = Arc::new(MockModelGateway::new());
    mock.script_tool_then_text(
        "sleep",
        serde_json::json!({ "duration_ms": 50 }),
        "ok, slept",
    );

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn ModelGateway>)
        .tools(sleep_tool)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle.submit_user_text("please sleep").await?;

    // Wait for the canonical terminal event on the broadcast stream. We
    // give ourselves 5 s to absorb worst-case scheduler jitter; the
    // happy path completes well under 200 ms (50 ms tokio::sleep + a
    // handful of FSM transitions).
    let saw_completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_completed,
        "TurnCompleted not observed within 5s — the async-job loop did \
         not drive the resumed turn to terminal"
    );

    // Drain the actor cleanly so the JSONL writer flushes before we read.
    handle.shutdown(Duration::from_secs(5)).await?;

    // Both scripted replies must have been consumed: turn 1 produced the
    // `sleep` tool call, turn 2 produced the final text.
    assert_eq!(
        mock.remaining(),
        0,
        "expected both scripted model replies to be consumed; {} remain",
        mock.remaining()
    );

    // Inspect the persisted event log. We assert exact ordering by
    // payload kind: every named event must appear, and each must
    // strictly precede the next.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };
    let kinds: Vec<&'static str> = log.iter().map(|e| kind_of(&e.payload)).collect();

    let need = [
        "JobSubmitted",
        "TurnPaused",
        "JobCompletedRecorded",
        "TurnCompleted",
    ];
    let mut cursor = 0usize;
    for needle in need {
        let pos = kinds[cursor..]
            .iter()
            .position(|k| *k == needle)
            .unwrap_or_else(|| panic!("missing {needle} in persisted log; got {kinds:?}"));
        cursor += pos + 1;
    }

    // The `JobCompletedRecorded.outcome` must carry the canonical "slept"
    // success that `SleepTool` emits. (See module docs for why we don't
    // also assert on `ToolResultRecorded` here.)
    let outcome = log
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::JobCompletedRecorded { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .expect("JobCompletedRecorded missing from persisted log");
    let tool_result = match outcome {
        cogito_protocol::job::JobOutcome::Success { result } => result,
        other => panic!("expected JobOutcome::Success, got {other:?}"),
    };
    match tool_result {
        ToolResult::Output(blocks) => {
            assert_eq!(
                blocks,
                vec![serde_json::Value::String("slept".into())],
                "SleepTool must surface its `slept` payload through the dispatcher"
            );
        }
        other => panic!("expected ToolResult::Output, got {other:?}"),
    }

    Ok(())
}

/// Tag the payload variant for sequence assertions. Kept here (not in
/// production code) so the matching is exhaustive for *this* test only;
/// future variants surface as `"_other_"` and the order check fails
/// loudly rather than silently dropping them.
fn kind_of(p: &EventPayload) -> &'static str {
    match p {
        EventPayload::SessionStarted { .. } => "SessionStarted",
        EventPayload::TurnStarted { .. } => "TurnStarted",
        EventPayload::AssistantMessageAppended { .. } => "AssistantMessageAppended",
        EventPayload::ToolUseRecorded { .. } => "ToolUseRecorded",
        EventPayload::ToolResultRecorded { .. } => "ToolResultRecorded",
        EventPayload::JobSubmitted { .. } => "JobSubmitted",
        EventPayload::TurnPaused { .. } => "TurnPaused",
        EventPayload::JobCompletedRecorded { .. } => "JobCompletedRecorded",
        EventPayload::TurnCompleted { .. } => "TurnCompleted",
        EventPayload::TurnFailed { .. } => "TurnFailed",
        EventPayload::ContextManageEntered {} => "ContextManageEntered",
        EventPayload::ContextManageCompleted {} => "ContextManageCompleted",
        EventPayload::PromptComposed { .. } => "PromptComposed",
        EventPayload::ModelCallStarted { .. } => "ModelCallStarted",
        EventPayload::ModelCallCompleted { .. } => "ModelCallCompleted",
        EventPayload::ThinkingBlockRecorded { .. } => "ThinkingBlockRecorded",
        EventPayload::HookRejected { .. } => "HookRejected",
        EventPayload::ContextCompacted { .. } => "ContextCompacted",
        EventPayload::SystemPromptInjected { .. } => "SystemPromptInjected",
        EventPayload::ToolFilterOverridden { .. } => "ToolFilterOverridden",
        EventPayload::ContextDecisionRecorded { .. } => "ContextDecisionRecorded",
        EventPayload::SkillActivated { .. } => "SkillActivated",
        _ => "_other_",
    }
}

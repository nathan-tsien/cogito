//! Sprint 8 Task 17: cancellation of a paused async job.
//!
//! Shape:
//! 1. Wire `Runtime` + `LocalJobManager` + a `SleepTool(60_000 ms)` so
//!    the job will park indefinitely.
//! 2. Submit a user turn, wait for `TurnPaused` to ensure the actor is
//!    in `InFlight::PausedOnJob`.
//! 3. Call `handle.cancel_turn()`. The handle:
//!    - fires the (idle) per-turn cancel token;
//!    - sends `InternalCancel` for an ack;
//!    - snapshots `in_flight`, sees `PausedOnJob`, and sends
//!      `CancelJob { job_id }`.
//! 4. The actor calls `job_mgr.cancel(job_id)`. `LocalJobManager` aborts
//!    the future and fires the registered `on_complete` sink with
//!    `JobOutcome::Cancelled`. Arm 3 routes that back as
//!    `SessionCommand::JobCompleted`; `handle_command` records
//!    `JobCompletedRecorded { outcome: Cancelled }` and re-enters the
//!    turn at `ToolDispatching` with `ToolResult::Error { kind:
//!    Cancelled }`. The mock model's turn-2 script returns a clean
//!    `end_turn`, so the FSM reaches `TurnCompleted`.
//!
//! Assertions:
//! - A terminal turn event (`TurnCompleted` or `TurnFailed`) arrives on
//!   the broadcast stream within ~2 s of the cancel (Task 17 budget is
//!   ~1 s; we keep a 2 s ceiling for scheduler jitter on CI).
//! - The persisted log contains `JobCompletedRecorded { outcome:
//!   Cancelled }` followed by `ToolResultRecorded { result: Error{kind:
//!   Cancelled} }` (the §5.1 ordering invariant; the
//!   `outcome_to_tool_result` translation runs in
//!   `session_loop::handle_command(JobCompleted)`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{LocalJobManager, SleepTool};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, JobOutcome, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_store::JsonlStore;
use futures::StreamExt as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cancel_while_paused_unwinds_to_tool_error_cancelled()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let sleep_tool: Arc<dyn ToolProvider> = Arc::new(SleepTool::new(
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>
    ));

    // Turn 1: tool_use(sleep, 60 s). Turn 2: clean text + end_turn so the
    // resumed (cancelled) turn has somewhere to land instead of failing.
    let mock = Arc::new(MockModelGateway::new());
    mock.script_tool_then_text(
        "sleep",
        serde_json::json!({ "duration_ms": 60_000 }),
        "cancelled, ok",
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

    handle.submit_user_text("sleep forever").await?;

    // Wait for the turn to enter the `PausedOnJob` state. The actor
    // emits `StreamEvent::TurnPaused` on the broadcast right before
    // parking, so observing it here means the next `cancel_turn` will
    // hit the CancelJob arm rather than the running-driver arm.
    let saw_paused = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnPaused { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_paused,
        "TurnPaused not observed within 2s — the async-job pause path may be broken"
    );

    let cancel_start = std::time::Instant::now();
    handle.cancel_turn().await?;

    // The cancel must unwind via:
    //   JobManager::cancel -> Cancelled sink -> Arm 3 -> JobCompleted
    //   mailbox -> respawn TurnDriver with ToolResult::Error{Cancelled}
    //   -> second model call -> end_turn -> TurnCompleted.
    // The Task 17 budget is ~1 s; we keep a 2 s ceiling for scheduler
    // jitter but the typical wall time is under 100 ms.
    let saw_terminal = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. } | StreamEvent::TurnFailed { .. }) => {
                    return true;
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    let elapsed = cancel_start.elapsed();
    assert!(
        saw_terminal,
        "terminal turn event not observed within 2s after cancel_turn (elapsed: {elapsed:?})"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "cancel-to-terminal took {elapsed:?} — Task 17 expects ~1s"
    );

    handle.shutdown(Duration::from_secs(5)).await?;

    // The persisted log must surface the cancellation as both
    // `JobCompletedRecorded { outcome: Cancelled }` and a downstream
    // `ToolResultRecorded { result: Error{kind: Cancelled} }`.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };

    let jcr_idx = log
        .iter()
        .position(|e| match &e.payload {
            EventPayload::JobCompletedRecorded { outcome, .. } => {
                matches!(outcome, JobOutcome::Cancelled)
            }
            _ => false,
        })
        .expect("JobCompletedRecorded { outcome: Cancelled } missing from persisted log");

    let trr_idx = log
        .iter()
        .position(|e| {
            matches!(
                &e.payload,
                EventPayload::ToolResultRecorded {
                    result: cogito_protocol::tool::ToolResult::Error {
                        kind: cogito_protocol::tool::ToolErrorKind::Cancelled,
                        ..
                    },
                    ..
                }
            )
        })
        .expect("ToolResultRecorded { Error{kind: Cancelled} } missing from persisted log");

    assert!(
        jcr_idx < trr_idx,
        "spec §5.1 violation: JobCompletedRecorded ({jcr_idx}) must precede ToolResultRecorded ({trr_idx})"
    );

    // Both scripted model replies must have been consumed. Turn 1
    // produced the `sleep` tool call (script #1); the cancelled async
    // job resumed the turn with a `ToolResult::Error { kind: Cancelled }`
    // visible to the model, which then consumed script #2 (clean
    // end_turn). Empty `remaining()` proves the cancel propagated
    // end-to-end through `outcome_to_tool_result` and back into the
    // next model call.
    assert_eq!(
        mock.remaining(),
        0,
        "expected both scripted model replies to be consumed (turn 1 + \
         cancel-resume), {} remain",
        mock.remaining()
    );

    Ok(())
}

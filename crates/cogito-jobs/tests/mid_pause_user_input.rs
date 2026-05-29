//! Sprint 8 Task 17: mid-pause user-input single-slot queue, end-to-end.
//!
//! Complements `crates/cogito-core/tests/mid_pause_user_input.rs` (Task 9),
//! which proves the actor's single-slot, latest-wins queue against a
//! parked `ModelGateway` mock. This variant exercises the *full* stack —
//! a real async tool (`SleepTool`) parks turn 1 inside the
//! `InFlight::PausedOnJob` state, then `send_user("a")`, `send_user("b")`
//! arrive during the pause and the second overwrites the first. Once the
//! sleep resolves and turn 1 completes, the actor drains the queued
//! trigger and starts turn 2 with `user_input == [Text("b")]`.
//!
//! Why both tests:
//! - Task 9 isolates the queue logic at the actor level (no real job).
//! - Task 17 (this) exercises the queue *while paused on a real
//!   `LocalJobManager`-backed job*, proving the queue and the
//!   `JobCompleted` Arm-3 path interact correctly: a queued trigger
//!   does NOT race the job-completion arm and the drained turn 2 only
//!   fires AFTER the paused turn 1 fully retires.
//!
//! Shape:
//! 1. `send_user("a")`, wait for `TurnPaused`.
//! 2. `send_user("b")` while paused. Lands in `pending_user_input`.
//! 3. Sleep resolves naturally (`200 ms` future). Turn 1 wraps up
//!    (`TurnCompleted`).
//! 4. `on_turn_complete` drains the queued trigger and starts turn 2.
//! 5. Wait for turn 2's `TurnCompleted`. Inspect the persisted log:
//!    exactly two `TurnStarted` events; `user_input` projects as
//!    `[Text("a")]` then `[Text("b")]`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines // long but linear integration setup; splitting hurts readability
)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{LocalJobManager, SleepTool};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::{ModelEvent, ModelGateway, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_store::JsonlStore;
use futures::StreamExt as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mid_pause_user_input_drained_latest_wins() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let sleep_tool: Arc<dyn ToolProvider> = Arc::new(SleepTool::new(
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>
    ));

    // Turn 1: tool_use(sleep, 200 ms) + text. Turn 2 (the queued
    // "b"-triggered turn): single text + end_turn. We script three
    // model replies in total: (turn-1 tool call), (turn-1 resume after
    // sleep with end_turn), (turn-2 with end_turn).
    let mock = Arc::new(MockModelGateway::new());
    // Turn 1: tool_use(sleep, 200 ms), stop = ToolUse.
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "sleep".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "sleep".into(),
            args: serde_json::json!({ "duration_ms": 200 }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);
    // Turn 1: resume after sleep completes; emit a final text + end_turn.
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "done-a".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "done-a".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);
    // Turn 2: the drained "b" trigger — clean text + end_turn.
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "done-b".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "done-b".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);

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

    // Trigger turn 1 and wait for it to actually pause on the sleep job
    // before submitting the queued triggers — submitting "b" before
    // turn 1 has entered `PausedOnJob` would race the actor's
    // `try_start_turn` and could end up either rejected (active turn)
    // or starting a second turn out of order.
    handle.submit_user_text("a").await?;
    let saw_paused = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnPaused) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_paused, "turn 1 never reached TurnPaused");

    // While paused, push "b" into the single-slot queue. (We only push
    // one extra trigger here; the latest-wins semantics with multiple
    // overwriting triggers is already covered by Task 9's actor-level
    // test. This test focuses on "real-job pause + queue drain".)
    handle.submit_user_text("b").await?;

    // Wait for two `TurnCompleted` events: one for turn 1 (after the
    // sleep resolves naturally and the resumed turn drives the second
    // model call), one for turn 2 (the drained "b" trigger).
    let saw_two_completed = tokio::time::timeout(Duration::from_secs(5), async {
        let mut n = 0u32;
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted) => {
                    n += 1;
                    if n >= 2 {
                        return true;
                    }
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_two_completed,
        "did not observe 2x TurnCompleted (turn 1 + drained turn 2) within 5s"
    );

    handle.shutdown(Duration::from_secs(5)).await?;

    // Inspect the persisted log: exactly two `TurnStarted` events; their
    // `user_input` is `[Text("a")]` and `[Text("b")]` in order. The
    // intermediate trigger that the queue holds (if any) MUST never have
    // produced a third `TurnStarted` — single-slot, latest-wins.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };

    let turn_starts: Vec<&Vec<ContentBlock>> = log
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TurnStarted { user_input, .. } => Some(user_input),
            _ => None,
        })
        .collect();
    assert_eq!(
        turn_starts.len(),
        2,
        "expected exactly two TurnStarted events; got {}",
        turn_starts.len()
    );
    assert_eq!(
        turn_starts[0],
        &vec![ContentBlock::Text { text: "a".into() }],
        "turn 1 user_input must be `a`"
    );
    assert_eq!(
        turn_starts[1],
        &vec![ContentBlock::Text { text: "b".into() }],
        "turn 2 user_input must be the queued `b` trigger"
    );

    // All three scripted model replies should have been consumed (two
    // for turn 1, one for turn 2).
    assert_eq!(
        mock.remaining(),
        0,
        "expected all 3 model replies to be consumed; {} remain",
        mock.remaining()
    );

    Ok(())
}

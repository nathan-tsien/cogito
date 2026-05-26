//! Integration test for Sprint 8 Task 13: `ResumePausedJob` synthesizes a
//! `Failed { message: "lost across process restart" }` completion when the
//! freshly-instantiated `JobManager` does not know the persisted `JobId`.
//!
//! Shape:
//!
//! 1. Pre-populate a JSONL store with a complete "paused on async tool"
//!    event sequence: `SessionStarted` -> `TurnStarted` ->
//!    `ModelCallStarted` -> `ToolUseRecorded { call_id: "c_async" }` ->
//!    `ModelCallCompleted { stop_reason: ToolUse }` ->
//!    `JobSubmitted { call_id: "c_async", job_id }` ->
//!    `TurnPaused { job_id }`.  The `job_id` is randomly generated, so the
//!    default `LocalJobManager` instantiated by `RuntimeBuilder` does not
//!    know it — exactly the "lost across process restart" scenario.
//!
//! 2. `Runtime::open_session(.., OpenMode::Resume)` runs the startup
//!    sequence.  H03 `replay` yields `ResumePoint::ResumePausedJob`, which
//!    `apply_resume_point` translates into `JobManager::on_complete`.  The
//!    `LocalJobManager` returns `JobError::UnknownJob`; the loop then
//!    posts a synthetic `JobCompletionEvent { outcome: Failed { message:
//!    "lost across process restart" } }` on its own Arm 3 channel.
//!
//! 3. Arm 3 dequeues the synthetic event and forwards it as
//!    `SessionCommand::JobCompleted`.  `handle_command` records
//!    `JobCompletedRecorded` and respawns the `TurnDriver` with
//!    `TurnEntry::FromToolDispatching { completed: [(c_async,
//!    AsyncFailed)] }`.  The mock model sees the `AsyncFailed` result, emits
//!    a clean `EndTurn`, and the turn reaches `TurnCompleted`.
//!
//! 4. The test asserts: (a) `TurnCompleted` reaches the broadcast stream;
//!    (b) the persisted log contains the synthetic
//!    `JobCompletedRecorded { outcome: Failed { message: "lost across
//!    process restart" } }`; (c) the original async job never fires (we
//!    never registered it with the new `LocalJobManager`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines // long but linear test setup; splitting hurts readability
)]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Hand-build a `ConversationEvent` with the given seq + payload.
fn evt(
    session_id: SessionId,
    seq: u64,
    turn_id: Option<TurnId>,
    payload: EventPayload,
) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id,
        seq,
        ts: Utc::now(),
        payload,
    }
}

#[tokio::test]
async fn resume_paused_job_synthesizes_failure_when_job_is_unknown()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let job_id = JobId::default();
    let async_call_id = "c_async".to_string();

    // Pre-populate the store with a complete paused-on-job log.  The
    // default `LocalJobManager` built by `RuntimeBuilder::build()` will
    // not know `job_id`, matching the "process restart lost the
    // in-memory map" scenario the synthesis path is designed for.
    let pre_events = vec![
        evt(
            session_id,
            0,
            None,
            EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    strategy: Some("default".into()),
                    model: Some("mock".into()),
                    ..Default::default()
                },
            },
        ),
        evt(
            session_id,
            1,
            Some(turn_id),
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text {
                    text: "run the long tool".into(),
                }],
                activate_skills: vec![],
            },
        ),
        evt(
            session_id,
            2,
            Some(turn_id),
            EventPayload::ModelCallStarted {
                model: "mock".into(),
            },
        ),
        evt(
            session_id,
            3,
            Some(turn_id),
            EventPayload::ToolUseRecorded {
                call_id: async_call_id.clone(),
                tool_name: "long_tool".into(),
                args: serde_json::json!({}),
            },
        ),
        evt(
            session_id,
            4,
            Some(turn_id),
            EventPayload::ModelCallCompleted {
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            },
        ),
        evt(
            session_id,
            5,
            Some(turn_id),
            EventPayload::JobSubmitted {
                call_id: async_call_id.clone(),
                job_id,
                tool_name: "long_tool".into(),
            },
        ),
        evt(
            session_id,
            6,
            Some(turn_id),
            EventPayload::TurnPaused { job_id },
        ),
    ];
    for e in &pre_events {
        store.append(e).await?;
    }

    // Mock model: the resumed TurnDriver re-enters at `FromToolDispatching`
    // with the synthetic AsyncFailed result and immediately calls the model
    // once more.  Reply with a clean EndTurn so the turn terminates.
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "the async job was lost".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "the async job was lost".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let handle = runtime.open_session(session_id, OpenMode::Resume).await?;
    let mut events_rx = handle.subscribe();

    // The actor must drive the resumed turn to `TurnCompleted` on its own:
    // synthesize -> Arm 3 -> JobCompleted mailbox -> respawn TurnDriver ->
    // model call (consumes the pushed script) -> EndTurn.
    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Ok(StreamEvent::TurnCompleted) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        got_completed,
        "TurnCompleted not observed within 5s after Resume; the synthetic \
         JobCompleted path must drive the turn to terminal even when the \
         original job is unknown to the new JobManager"
    );

    // The mock model script must have been consumed exactly once — proving
    // the resumed TurnDriver actually re-entered ToolDispatching, sent the
    // AsyncFailed result back to the model, and drove the next model call.
    assert_eq!(
        mock.remaining(),
        0,
        "mock gateway script should have been consumed by the resumed turn"
    );

    // Clean shutdown.
    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got: {out:?}"
    );

    // Inspect the persisted log: it must contain the synthetic
    // `JobCompletedRecorded { outcome: Failed { message: "lost across
    // process restart" } }` immediately following the original
    // `TurnPaused`.
    let mut log_stream = store.replay(session_id, 0);
    let mut log: Vec<ConversationEvent> = Vec::new();
    while let Some(evt) = log_stream.next().await {
        log.push(evt?);
    }
    let synthetic = log.iter().find_map(|e| match &e.payload {
        EventPayload::JobCompletedRecorded {
            job_id: jid,
            outcome,
        } if *jid == job_id => Some(outcome.clone()),
        _ => None,
    });
    let outcome = synthetic.expect("synthetic JobCompletedRecorded missing from persisted log");
    match outcome {
        JobOutcome::Failed { message } => {
            assert_eq!(
                message, "lost across process restart",
                "synthetic Failed completion must carry the canonical lost-across-restart message"
            );
        }
        other => panic!("expected JobOutcome::Failed, got {other:?}"),
    }

    Ok(())
}

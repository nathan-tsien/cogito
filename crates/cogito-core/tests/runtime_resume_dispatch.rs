//! P4.4 — `run_session` startup sequence + `apply_resume_point`.
//!
//! End-to-end tests covering the resume-on-startup contract:
//!
//! 1. `resume_from_model_completed_fast_paths_to_turn_completed` — the
//!    session loop must call H03 `replay()` on startup and dispatch the
//!    resulting `ResumeFromModelCompleted` into the FSM, which fast-paths
//!    to `TurnCompleted` without re-calling the model.
//!
//! 2. `resume_with_completed_session_idles_then_serves_new_input` — when the
//!    log ends in `TurnCompleted`/`TurnFailed`, replay yields `FreshTurn` and
//!    the actor idles until the next user Input.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines // long but linear test setup; splitting hurts readability
)]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::{ConversationEvent, EventPayload, SCHEMA_VERSION, SessionMeta};
use cogito_store_jsonl::JsonlStore;
use cogito_test_fixtures::canonical_sample_session;
use cogito_tools::{BuiltinToolProvider, ReadFile};

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
async fn resume_from_model_completed_fast_paths_to_turn_completed()
-> Result<(), Box<dyn std::error::Error>> {
    // Pre-populate the store with an in-flight turn whose model call has
    // completed but no TurnCompleted was written (simulating an actor crash
    // after writing ModelCallCompleted). H03 replay yields
    // ResumeFromModelCompleted; with P4.4 the actor must spawn TurnDriver
    // via TurnEntry::FromModelCompleted, which fast-paths to TurnCompleted
    // WITHOUT re-calling the model.
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();

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
                user_input: vec![cogito_protocol::ContentBlock::Text { text: "hi".into() }],
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
            EventPayload::AssistantMessageAppended { text: "ack".into() },
        ),
        evt(
            session_id,
            4,
            Some(turn_id),
            EventPayload::ModelCallCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            },
        ),
    ];
    for e in &pre_events {
        store.append(e).await?;
    }

    // Mock model: no scripts queued. If the actor incorrectly re-calls the
    // model during resume, the gateway will return a "no scripts" error and
    // the test will fail.
    let mock = Arc::new(MockModelGateway::new());

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

    // Open Resume and subscribe BEFORE the actor task starts driving the
    // resumed turn. open_session is async but the actor task is spawned;
    // subscribing immediately after open_session returns should still catch
    // the broadcast.
    let handle = runtime.open_session(session_id, OpenMode::Resume).await?;
    let mut events_rx = handle.subscribe();

    // Wait for TurnCompleted on the broadcast stream. We did NOT send any
    // Input — the FSM should drive the resumed turn to completion on its own.
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
        "TurnCompleted not observed within 5s after Resume \
         (ResumeFromModelCompleted should fast-path to Completed)"
    );

    // The mock model must NOT have been called during the resume fast-path.
    assert_eq!(
        mock.remaining(),
        0,
        "mock gateway started with 0 scripts; remaining count should stay 0"
    );

    // Clean shutdown.
    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got: {out:?}"
    );

    Ok(())
}

#[tokio::test]
async fn resume_with_completed_session_idles_then_serves_new_input()
-> Result<(), Box<dyn std::error::Error>> {
    // Pre-populate the store with just SessionStarted (seq=0). H03 replay
    // returns FreshTurn; the actor should idle until we send a fresh Input,
    // then drive a normal turn to completion.
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let events = canonical_sample_session();
    let session_id = events[0].session_id;
    store.append(&events[0]).await?;

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "ack".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "ack".into(),
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

    handle.submit_user_text("hello after resume").await?;

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
        "TurnCompleted not observed within 5s after resume"
    );

    // The mock script must have been consumed by the fresh turn — otherwise
    // TurnCompleted fired through some unintended path (e.g. the resumed
    // FreshTurn idle didn't actually idle and the turn closed without a model
    // call).
    assert_eq!(
        mock.remaining(),
        0,
        "model should have been called exactly once for the fresh Input"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got: {out:?}"
    );

    Ok(())
}

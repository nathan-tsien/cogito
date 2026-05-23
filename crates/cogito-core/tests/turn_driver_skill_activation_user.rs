#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Validates that `TurnTrigger::SkillActivation` projects to `TurnStarted`
//! with the right `activate_skills` + `user_input` shape. Drives one turn
//! through a mock model that just echoes -- assertion is on the recorded
//! `TurnStarted` event.
//!
//! Mirrors the scaffolding from `runtime_submit.rs` (mock model, JSONL
//! store, in-mem broadcast subscription), but pushes a
//! `TurnTrigger::SkillActivation` instead of `UserText` so the projection
//! branch added in Sprint 7 Task 16 is exercised end-to-end.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::turn_trigger::TurnTrigger;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Build a `Runtime` with a one-shot mock model that emits a minimal
/// `TextDelta` + `MessageCompleted` reply so the turn can reach
/// `TurnCompleted` without external dependencies.
fn build_runtime(store: &Arc<JsonlStore>) -> Result<Arc<Runtime>, Box<dyn std::error::Error>> {
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
    Ok(Runtime::builder()
        .store(Arc::clone(store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?)
}

/// Read back the persisted log and return the first `TurnStarted` payload.
async fn first_turn_started(
    store: &Arc<JsonlStore>,
    session_id: SessionId,
) -> Result<EventPayload, Box<dyn std::error::Error>> {
    let mut stream = store.replay(session_id, 0);
    while let Some(evt) = stream.next().await {
        let evt = evt?;
        if matches!(&evt.payload, EventPayload::TurnStarted { .. }) {
            return Ok(evt.payload);
        }
    }
    Err("no TurnStarted event in persisted log".into())
}

#[tokio::test]
async fn skill_activation_with_text_projects_correctly() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(&store)?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle
        .submit(TurnTrigger::SkillActivation {
            names: vec!["foo".into()],
            user_text: Some("hi".into()),
        })
        .await?;

    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
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
    assert!(got_completed, "TurnCompleted event not received within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    match first_turn_started(&store, session_id).await? {
        EventPayload::TurnStarted {
            user_input,
            activate_skills,
        } => {
            assert_eq!(
                activate_skills,
                vec!["foo".to_string()],
                "activate_skills must carry the names from SkillActivation"
            );
            assert_eq!(
                user_input,
                vec![ContentBlock::Text { text: "hi".into() }],
                "non-empty user_text projects to a single Text block"
            );
        }
        other => return Err(format!("expected TurnStarted, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn skill_activation_no_text_projects_to_empty_user_input()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(&store)?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle
        .submit(TurnTrigger::SkillActivation {
            names: vec!["foo".into()],
            user_text: None,
        })
        .await?;

    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
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
    assert!(got_completed, "TurnCompleted event not received within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    match first_turn_started(&store, session_id).await? {
        EventPayload::TurnStarted {
            user_input,
            activate_skills,
        } => {
            assert_eq!(activate_skills, vec!["foo".to_string()]);
            assert!(
                user_input.is_empty(),
                "missing user_text must yield empty user_input"
            );
        }
        other => return Err(format!("expected TurnStarted, got {other:?}").into()),
    }
    Ok(())
}

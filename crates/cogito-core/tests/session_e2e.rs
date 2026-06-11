//! End-to-end session tests: open -> send -> complete/fail -> shutdown.
//!
//! Wires a `JsonlStore` + `MockModelGateway` + `BuiltinToolProvider` through
//! `Runtime::builder()`, opens a session, sends one user message, waits for
//! the turn to complete via the broadcast stream, and shuts down cleanly.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

#[tokio::test]
async fn open_send_complete_shutdown() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

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
        .store(store)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    // Subscribe before sending so we don't miss events.
    let mut events = handle.subscribe();

    handle.submit_user_text("hello").await?;

    // Wait for TurnCompleted on the broadcast stream (deterministic, no sleep).
    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(got_completed, "TurnCompleted event not received within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "session did not shut down cleanly: {out:?}"
    );
    Ok(())
}

/// ISSUE#69 part 1 / ADR-0040: a turn whose final model call is cut off by
/// `max_tokens` still reaches `Completed`, but the broadcast `TurnCompleted`
/// carries `stop_reason: Some(StopReason::MaxTokens)` so a live subscriber can
/// distinguish truncation from a clean end-of-turn without scanning
/// `ModelCallCompleted`. A clean turn carries `Some(StopReason::EndTurn)`.
#[tokio::test]
async fn truncated_turn_flags_max_tokens_on_turn_completed()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    // A reply truncated mid-output: text, then MessageCompleted with MaxTokens.
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "partial".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "partial".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::MaxTokens,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 4096,
            },
        },
    ]);

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let runtime = Runtime::builder()
        .store(store)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    let mut events = handle.subscribe();
    handle.submit_user_text("write a long essay").await?;

    // Capture the terminal broadcast and inspect its stop_reason.
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(ev @ StreamEvent::TurnCompleted { .. }) => return Some(ev),
                Ok(StreamEvent::TurnFailed { .. }) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(None);

    // The turn must reach Completed (not Failed) AND flag the truncation.
    assert!(
        matches!(
            terminal,
            Some(StreamEvent::TurnCompleted {
                stop_reason: Some(StopReason::MaxTokens),
                ..
            })
        ),
        "truncated turn must reach Completed flagging MaxTokens; got {terminal:?}"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}

/// Regression test for the `double-turn-completed` bug (ISSUE#69 part 2).
///
/// Before the fix, `on_turn_complete` called `record_turn_completed` for
/// `TurnOutcome::Completed` even though the FSM transition (`model_completed::transit`)
/// had already written the `TurnCompleted` event. This produced two `TurnCompleted`
/// entries in the event log (and two broadcasts) for a single completed turn —
/// consumers saw the pairs in their `ConversationStore` and on the broadcast channel.
/// This test drives a turn to clean completion and asserts that exactly one
/// `TurnCompleted` event is written to the JSONL store. Mirrors
/// `failed_turn_emits_exactly_one_turn_failed_event`, which already held for the
/// `Failed` arm.
#[tokio::test]
async fn completed_turn_emits_exactly_one_turn_completed_event()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

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

    let store_clone = Arc::clone(&store);
    let runtime = Runtime::builder()
        .store(store_clone)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    let mut events = handle.subscribe();
    handle.submit_user_text("hello").await?;

    // Wait for TurnCompleted to arrive on the broadcast stream.
    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        got_completed,
        "TurnCompleted stream event not received within 5s"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "session did not shut down cleanly: {out:?}"
    );

    // Replay the event log and count TurnCompleted payloads. Before the fix,
    // this count was 2; after the fix it must be exactly 1.
    let turn_completed_count = store
        .replay(session_id, 0)
        .filter(|ev| {
            let is_completed = matches!(
                ev,
                Ok(e) if matches!(e.payload, EventPayload::TurnCompleted { .. })
            );
            futures::future::ready(is_completed)
        })
        .count()
        .await;

    assert_eq!(
        turn_completed_count, 1,
        "expected exactly 1 TurnCompleted event in the log, got {turn_completed_count} \
         (double-turn-completed regression, ISSUE#69 part 2)"
    );

    Ok(())
}

/// Regression test for the double-record bug fixed in Sprint 3 P2.5 follow-up.
///
/// Before the fix, actor.rs called `record_turn_failed` for `TurnOutcome::Failed`
/// even though the FSM transition had already written the event. This produced two
/// `TurnFailed` entries in the event log for a single failed turn. This test drives
/// a turn to failure (via a mock model error) and asserts that exactly one
/// `TurnFailed` event is written to the JSONL store.
#[tokio::test]
async fn failed_turn_emits_exactly_one_turn_failed_event() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    // Push a model error to trigger a TurnOutcome::Failed path via the FSM.
    mock.push_error("mock model error: simulated provider failure");

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let store_clone = Arc::clone(&store);
    let runtime = Runtime::builder()
        .store(store_clone)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    // Subscribe before sending so we don't miss the TurnFailed stream event.
    let mut events = handle.subscribe();
    handle.submit_user_text("trigger failure").await?;

    // Wait for TurnFailed to arrive on the broadcast stream.
    let got_failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnFailed { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(got_failed, "TurnFailed stream event not received within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "session did not shut down cleanly: {out:?}"
    );

    // Replay the event log and count TurnFailed payloads.  Before the fix,
    // this count was 2; after the fix it must be exactly 1.
    let turn_failed_count = store
        .replay(session_id, 0)
        .filter(|ev| {
            let is_failed = matches!(
                ev,
                Ok(e) if matches!(e.payload, EventPayload::TurnFailed { .. })
            );
            futures::future::ready(is_failed)
        })
        .count()
        .await;

    assert_eq!(
        turn_failed_count, 1,
        "expected exactly 1 TurnFailed event in the log, got {turn_failed_count} \
         (double-record regression from Sprint 3 P2.5)"
    );

    Ok(())
}

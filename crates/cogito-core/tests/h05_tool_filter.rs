//! Integration test: H05 tool filter modes through the Runtime.
//!
//! Scope:
//! - Sub-test 1 (Inherit / NoneOverrider): default `ContextConfig` runs through
//!   the Runtime and emits a `ToolFilterOverridden { mode: Inherit }` event.
//!   The persisted event is inspected to confirm H05 applied the no-op path.
//! - Sub-test 2 (Allow filter respected): strategy restricts `allowed_tools`
//!   to a single named tool; after one turn the `ToolFilterOverridden` event
//!   still carries `Inherit` mode (produced by the `NoneOverrider`), proving
//!   the surface is filtered by strategy, not by the overrider.
//!
//! Note: Intersect and Replace modes are exercised at the unit level in
//! `cogito_core::harness::tool_surface` (Task 30). These integration tests
//! verify the same modes reach the event log via the full Runtime stack.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::EventPayload;
use cogito_protocol::context::ToolFilterOverrideMode;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::stream::StreamEvent;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Build a minimal reply: one text block and `MessageCompleted`.
fn text_reply(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: text.into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: text.into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 3,
            },
        },
    ]
}

/// Submit one user message and wait for `TurnCompleted`. Panics on failure.
async fn run_one_turn(handle: &cogito_core::runtime::SessionHandle, msg: &str) {
    let mut events = handle.subscribe();
    handle
        .submit_user_text(msg)
        .await
        .expect("submit_user_text");

    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(StreamEvent::TurnFailed { reason, .. }) => {
                    // Surface the failure reason so test diagnostics are clear.
                    panic!("TurnFailed: {reason:?}");
                }
                Err(e) => {
                    panic!("stream error waiting for TurnCompleted: {e:?}");
                }
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(ok, "turn '{msg}' timed out waiting for TurnCompleted");
}

// ---------------------------------------------------------------------------
// Sub-test 1: default config (NoneOverrider) -> Inherit mode in event log
// ---------------------------------------------------------------------------

#[tokio::test]
async fn h05_inherit_mode_written_by_default_config() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(text_reply("ok"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Default strategy: NoneOverrider, ToolFilter::All.
    let strategy = HarnessStrategy::default_with_model("mock");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(strategy)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    run_one_turn(&handle, "hello").await;

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    // Replay and locate the ToolFilterOverridden event.
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };

    let filter_ev = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ToolFilterOverridden { .. }))
        .expect("expected exactly 1 ToolFilterOverridden event");

    match &filter_ev.payload {
        EventPayload::ToolFilterOverridden {
            mode, produced_by, ..
        } => {
            assert!(
                matches!(mode, ToolFilterOverrideMode::Inherit),
                "NoneOverrider must emit Inherit mode; got {mode:?}"
            );
            assert_eq!(
                produced_by, "none",
                "NoneOverrider produced_by must be 'none'; got '{produced_by}'"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Sub-test 2: strategy Allow filter -> Inherit mode still written (overrider
//             does not affect the strategy filter; it adds on top)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn h05_strategy_allow_filter_does_not_change_overrider_mode()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(text_reply("ok"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Strategy restricts tools to only "read_file"; NoneOverrider still writes Inherit.
    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.allowed_tools = ToolFilter::Allow(vec!["read_file".into()]);

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(strategy)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    run_one_turn(&handle, "hello").await;

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };

    let filter_ev = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ToolFilterOverridden { .. }))
        .expect("expected exactly 1 ToolFilterOverridden event");

    match &filter_ev.payload {
        EventPayload::ToolFilterOverridden { mode, .. } => {
            // The overrider (NoneOverrider) always emits Inherit regardless of
            // how the strategy restricts tools. The restriction is applied by
            // H05 surface() when building the final tool list, not stored in mode.
            assert!(
                matches!(mode, ToolFilterOverrideMode::Inherit),
                "NoneOverrider with Allow-filtered strategy must still emit Inherit; got {mode:?}"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    // Verify the ContextDecisionRecorded references the filter event correctly.
    let filter_event_id = filter_ev.event_id;

    let decision = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ContextDecisionRecorded { .. }))
        .expect("expected exactly 1 ContextDecisionRecorded");

    match &decision.payload {
        EventPayload::ContextDecisionRecorded {
            tool_filter_event, ..
        } => {
            assert_eq!(
                *tool_filter_event, filter_event_id,
                "ContextDecisionRecorded.tool_filter_event must reference the \
                 ToolFilterOverridden event id"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Sub-test 3: two turns - verify H05 writes a fresh ToolFilterOverridden
//             event per turn (not reusing across turns).
//
// Approach: subscribe once, drive two sequential user messages, wait for
// two TurnCompleted broadcasts, then inspect the persisted log.
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn h05_each_turn_writes_its_own_tool_filter_event() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(text_reply("first"));
    mock.push_reply(text_reply("second"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let strategy = HarnessStrategy::default_with_model("mock");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(strategy)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    // Subscribe once before any turns to capture all broadcast events.
    let mut events = handle.subscribe();

    // Submit turn 1 and wait for its single TurnCompleted broadcast. Since
    // ISSUE#69 part 2 was fixed, exactly one is emitted per turn (the
    // TurnDriver's FSM transition; session_loop.on_turn_complete no longer
    // re-records it). Submitting turn 2 back-to-back is still safe: the
    // single-threaded actor parks a trigger arriving mid-retirement in the
    // single-slot pending_user_input queue and drains it, so it is never lost.
    handle.submit_user_text("turn 1").await?;
    let turn1_ok = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(StreamEvent::TurnFailed { reason, .. }) => {
                    panic!("turn 1 failed: {reason:?}");
                }
                Err(e) => panic!("turn 1 stream error: {e:?}"),
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(turn1_ok, "turn 1 did not complete within 5s");

    // Submit turn 2. If turn 1's actor-side retirement has not run yet, the
    // trigger is queued and drained on retirement (single-slot, never lost).
    handle.submit_user_text("turn 2").await?;
    let turn2_ok = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(StreamEvent::TurnFailed { reason, .. }) => {
                    panic!("turn 2 failed: {reason:?}");
                }
                Err(e) => panic!("turn 2 stream error: {e:?}"),
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(turn2_ok, "turn 2 did not produce TurnCompleted within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };

    // Two real turns => two TurnStarted events.
    let turn_started_count = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .count();
    assert_eq!(
        turn_started_count, 2,
        "expected exactly 2 TurnStarted events (one per turn); got {turn_started_count}"
    );

    // Two turns => two ToolFilterOverridden events with distinct event_ids.
    let filter_events: Vec<_> = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::ToolFilterOverridden { .. }))
        .collect();

    assert_eq!(
        filter_events.len(),
        2,
        "expected exactly 2 ToolFilterOverridden events (one per turn); \
         got {}",
        filter_events.len()
    );

    let id0 = filter_events[0].event_id;
    let id1 = filter_events[1].event_id;
    assert_ne!(
        id0, id1,
        "each turn must produce a distinct ToolFilterOverridden event_id"
    );

    // Both must be Inherit mode.
    for ev in &filter_events {
        match &ev.payload {
            EventPayload::ToolFilterOverridden { mode, .. } => {
                assert!(
                    matches!(mode, ToolFilterOverrideMode::Inherit),
                    "both turns must have Inherit mode; got {mode:?}"
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    Ok(())
}

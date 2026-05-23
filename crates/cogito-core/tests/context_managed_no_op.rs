//! Integration test: default `ContextConfig` (all no-op) writes the expected
//! context-decision events per turn.
//!
//! One turn through the Runtime must produce these events in order:
//!   - `ContextManageEntered`    (1x)
//!   - `SystemPromptInjected`    (1x, suffix="", `produced_by="none`")
//!   - `ToolFilterOverridden`    (1x, mode=Inherit, `produced_by="none`")
//!   - `ContextDecisionRecorded` (1x, compactions=[], errors all None)
//!   - `ContextManageCompleted`  (1x)
//!   - `ContextCompacted`        (0x — no compactor configured)

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
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Build a minimal reply script: one text block + `MessageCompleted`.
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

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn no_op_context_pipeline_writes_four_decision_events()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(text_reply("ack"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Default HarnessStrategy has ContextConfig::default() (all no-op).
    let strategy = HarnessStrategy::default_with_model("mock");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(strategy)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle.submit_user_text("hello").await?;

    // Wait for TurnCompleted on the broadcast stream.
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

    // Replay and collect all events.
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };

    // -- Count each context-decision event type --

    let entered_count = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::ContextManageEntered {}))
        .count();
    assert_eq!(entered_count, 1, "expected exactly 1 ContextManageEntered");

    let completed_count = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::ContextManageCompleted {}))
        .count();
    assert_eq!(
        completed_count, 1,
        "expected exactly 1 ContextManageCompleted"
    );

    let compacted_count = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::ContextCompacted { .. }))
        .count();
    assert_eq!(
        compacted_count, 0,
        "expected 0 ContextCompacted with no-op config"
    );

    // -- Locate and inspect SystemPromptInjected --
    let injected = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::SystemPromptInjected { .. }))
        .expect("expected exactly 1 SystemPromptInjected");
    let injected_event_id = injected.event_id;
    match &injected.payload {
        EventPayload::SystemPromptInjected {
            suffix,
            produced_by,
            ..
        } => {
            assert_eq!(suffix, "", "no-op injector must emit empty suffix");
            assert_eq!(
                produced_by, "none",
                "no-op injector produced_by must be 'none'"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    // -- Locate and inspect ToolFilterOverridden --
    let filter_overridden = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ToolFilterOverridden { .. }))
        .expect("expected exactly 1 ToolFilterOverridden");
    let filter_event_id = filter_overridden.event_id;
    match &filter_overridden.payload {
        EventPayload::ToolFilterOverridden {
            mode, produced_by, ..
        } => {
            assert!(
                matches!(mode, ToolFilterOverrideMode::Inherit),
                "no-op overrider must emit Inherit mode, got {mode:?}"
            );
            assert_eq!(
                produced_by, "none",
                "no-op overrider produced_by must be 'none'"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    // -- Locate and inspect ContextDecisionRecorded --
    let decision = persisted
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ContextDecisionRecorded { .. }))
        .expect("expected exactly 1 ContextDecisionRecorded");
    match &decision.payload {
        EventPayload::ContextDecisionRecorded {
            compactions,
            system_prompt_event,
            tool_filter_event,
            errors,
            ..
        } => {
            assert!(
                compactions.is_empty(),
                "no-op: compactions must be empty, got {compactions:?}"
            );
            assert_eq!(
                *system_prompt_event, injected_event_id,
                "system_prompt_event must reference the SystemPromptInjected event id"
            );
            assert_eq!(
                *tool_filter_event, filter_event_id,
                "tool_filter_event must reference the ToolFilterOverridden event id"
            );
            assert!(
                errors.compactor.is_none(),
                "no errors expected for no-op compactor"
            );
            assert!(
                errors.injector.is_none(),
                "no errors expected for no-op injector"
            );
            assert!(
                errors.overrider.is_none(),
                "no errors expected for no-op overrider"
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    // -- Verify ordering: Entered -> SystemPromptInjected -> ToolFilterOverridden
    //    -> ContextDecisionRecorded -> Completed --
    let ctx_event_seqs: Vec<(u64, &str)> = persisted
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ContextManageEntered {} => Some((e.seq, "Entered")),
            EventPayload::SystemPromptInjected { .. } => Some((e.seq, "SystemPromptInjected")),
            EventPayload::ToolFilterOverridden { .. } => Some((e.seq, "ToolFilterOverridden")),
            EventPayload::ContextDecisionRecorded { .. } => {
                Some((e.seq, "ContextDecisionRecorded"))
            }
            EventPayload::ContextManageCompleted {} => Some((e.seq, "Completed")),
            _ => None,
        })
        .collect();

    assert_eq!(
        ctx_event_seqs.len(),
        5,
        "expected 5 context events in order"
    );

    // Check that seqs are strictly increasing and in the right label order.
    let expected_labels = [
        "Entered",
        "SystemPromptInjected",
        "ToolFilterOverridden",
        "ContextDecisionRecorded",
        "Completed",
    ];
    for (i, (seq, label)) in ctx_event_seqs.iter().enumerate() {
        assert_eq!(
            *label, expected_labels[i],
            "at position {i}: expected '{}', got '{label}' (seq={seq})",
            expected_labels[i]
        );
        if i > 0 {
            assert!(
                *seq > ctx_event_seqs[i - 1].0,
                "seq must be strictly increasing: {seq} <= {}",
                ctx_event_seqs[i - 1].0
            );
        }
    }

    Ok(())
}

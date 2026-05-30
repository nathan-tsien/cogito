//! ADR-0028: per-session provider injection via `open_session_with` /
//! `update_session`. Patterned on `tests/runtime_submit.rs` (real
//! `MockModelGateway` + `JsonlStore` + `BuiltinToolProvider`).

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, SessionHandle, SessionSpec, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn end_turn_reply() -> Vec<ModelEvent> {
    vec![
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
    ]
}

fn builtin_tools() -> Arc<dyn ToolProvider> {
    Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    )
}

async fn await_turn_completed(handle: &SessionHandle) -> bool {
    let mut events = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[tokio::test]
async fn open_session_with_uses_injected_providers() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let mut per_session_strategy = HarnessStrategy::default_with_model("mock");
    per_session_strategy.name = "tenant-acme".into();
    let spec = SessionSpec {
        tools: Some(builtin_tools()),
        strategy: Some(per_session_strategy),
        tenant_id: Some("acme".into()),
        ..Default::default()
    };

    let sid = SessionId::new();
    let handle = runtime.open_session_with(sid, OpenMode::New, spec).await?;
    handle.submit_user_text("hello").await?;

    assert!(await_turn_completed(&handle).await, "turn did not complete");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}

#[tokio::test]
async fn update_session_then_turn_completes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let sid = SessionId::new();
    let handle = runtime.open_session(sid, OpenMode::New).await?;

    // Swap the tool provider mid-session (no turn in flight yet).
    let spec = SessionSpec {
        tools: Some(builtin_tools()),
        ..Default::default()
    };
    handle.update_session(spec).await?;

    // The next turn must still complete with the swapped provider.
    handle.submit_user_text("hi").await?;
    assert!(
        await_turn_completed(&handle).await,
        "turn did not complete after update"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}

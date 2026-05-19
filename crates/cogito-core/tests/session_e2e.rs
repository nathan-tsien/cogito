//! End-to-end session test: open -> send -> complete -> shutdown.
//!
//! Wires a `JsonlStore` + `MockModelGateway` + `BuiltinToolProvider` through
//! `Runtime::builder()`, opens a session, sends one user message, waits for
//! the turn to complete via the broadcast stream, and shuts down cleanly.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};

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

    handle.send_user("hello").await?;

    // Wait for TurnCompleted on the broadcast stream (deterministic, no sleep).
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
    assert!(out.clean, "session did not shut down cleanly: {out:?}");
    Ok(())
}

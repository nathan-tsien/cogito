//! Integration test for `SessionHandle::submit(TurnTrigger)` -- ADR-0016.
//!
//! Mirrors `session_e2e::open_send_complete_shutdown` but exercises the
//! `submit` path with a `TurnTrigger::UserText` payload, then asserts
//! that the persisted `TurnStarted` event's `user_input` is exactly
//! `vec![ContentBlock::Text { text: "hello" }]`. This locks the v0.1
//! projection (ADR-0016 §4: "`TurnTrigger::UserText(text)` projects to
//! vec![`ContentBlock::Text` { text }]").

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

#[tokio::test]
async fn submit_user_text_projects_to_text_content_block() -> Result<(), Box<dyn std::error::Error>>
{
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

    // Pass a cloned Arc to the builder so the local `store` handle stays
    // alive for the read-back assertion below. The explicit cast to
    // `Arc<dyn ConversationStore>` is required because `RuntimeBuilder::store`
    // takes the trait object type and coercion does not cross `Arc::clone`.
    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // The canonical entry point for any new turn trigger source.
    handle.submit(TurnTrigger::UserText("hello".into())).await?;

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

    // Verify the projection: read the persisted log and confirm the
    // first TurnStarted event carries user_input = [Text("hello")].
    // `replay(session_id, 0)` yields events with `seq > 0`; SessionStarted
    // sits at seq 0 and is skipped, but TurnStarted (seq 1) is included.
    // The trait method is in scope via the `ConversationStore` import; the
    // call auto-derefs through `Arc<JsonlStore>`.
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };
    let turn_started = persisted
        .iter()
        .find(|e| matches!(&e.payload, EventPayload::TurnStarted { .. }))
        .ok_or("expected a TurnStarted event in the persisted log")?;
    match &turn_started.payload {
        EventPayload::TurnStarted { user_input, .. } => {
            assert_eq!(
                user_input,
                &vec![ContentBlock::Text {
                    text: "hello".into()
                }],
                "TurnTrigger::UserText projection must equal vec![Text(text)] per ADR-0016 §4"
            );
        }
        other => {
            return Err(format!("expected TurnStarted, got {other:?}").into());
        }
    }

    Ok(())
}

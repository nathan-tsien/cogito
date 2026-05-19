//! Verifies that `MockModelGateway` plays back scripts faithfully.

#![allow(clippy::unwrap_used)]

use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{
    ModelEvent, ModelGateway, ModelInput, ModelParams, StopReason, Usage,
};
use cogito_protocol::ids::{SessionId, TurnId};
use futures::StreamExt;

fn empty_input() -> ModelInput {
    ModelInput {
        system: String::new(),
        messages: vec![],
        tools: vec![],
        params: ModelParams {
            model: "mock".into(),
            max_tokens: 1,
            temperature: None,
            top_p: None,
            stop_sequences: vec![],
        },
    }
}

#[tokio::test]
async fn replays_scripted_events() -> Result<(), Box<dyn std::error::Error>> {
    let gateway = MockModelGateway::new();
    gateway.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "hi".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "hi".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let mut stream = gateway.stream(empty_input(), ctx).await?;
    let mut seen = 0;
    while let Some(evt) = stream.next().await {
        evt?;
        seen += 1;
    }
    assert_eq!(seen, 3);
    assert_eq!(gateway.remaining(), 0);
    Ok(())
}

#[tokio::test]
async fn returns_scripted_error() {
    let gateway = MockModelGateway::new();
    gateway.push_error("simulated outage");
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let res = gateway.stream(empty_input(), ctx).await;
    assert!(res.is_err());
}

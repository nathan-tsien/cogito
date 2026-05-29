//! Integration tests for H06 `stream_demux::demux`.

use std::sync::Arc;

use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::stream_demux::demux;
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{
    ModelEvent, ModelGateway, ModelInput, ModelParams, StopReason, Usage,
};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::store::ConversationStore;
use cogito_store::JsonlStore;
use tokio::sync::broadcast;

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

async fn make_recorder(
    tmp: &tempfile::TempDir,
) -> Result<
    (
        StepRecorder,
        broadcast::Receiver<cogito_protocol::stream::StreamEvent>,
    ),
    Box<dyn std::error::Error>,
> {
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (tx, rx) = broadcast::channel(64);
    let sid = SessionId::new();
    let mut recorder = StepRecorder::new(Arc::clone(&store), tx, sid, 0);
    recorder
        .record_session_started(SessionMeta::default())
        .await?;
    Ok((recorder, rx))
}

#[tokio::test]
async fn demux_text_only_yields_text_content() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let (mut recorder, _rx) = make_recorder(&tmp).await?;

    let mock = MockModelGateway::new();
    mock.push_reply(vec![
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
    let turn_id = TurnId::new();
    let stream = mock.stream(empty_input(), ctx).await?;
    let output = demux(stream, &mut recorder, turn_id, None).await?;

    assert_eq!(output.content.len(), 1);
    assert!(
        matches!(&output.content[0], ContentBlock::Text { text } if text == "hi"),
        "expected Text {{ text: \"hi\" }}, got {:?}",
        output.content[0]
    );
    assert_eq!(output.stop_reason, StopReason::EndTurn);
    Ok(())
}

#[tokio::test]
async fn demux_tool_use_captures_call_in_content() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let (mut recorder, _rx) = make_recorder(&tmp).await?;

    let mock = MockModelGateway::new();
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({ "path": "/tmp/x" }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        },
    ]);

    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let turn_id = TurnId::new();
    let stream = mock.stream(empty_input(), ctx).await?;
    let output = demux(stream, &mut recorder, turn_id, None).await?;

    assert_eq!(output.content.len(), 1);
    assert!(
        matches!(
            &output.content[0],
            ContentBlock::ToolUse { call_id, tool_name, .. }
                if call_id == "c1" && tool_name == "read_file"
        ),
        "expected ToolUse with call_id=c1 and tool_name=read_file, got {:?}",
        output.content[0]
    );
    assert_eq!(output.stop_reason, StopReason::ToolUse);
    Ok(())
}

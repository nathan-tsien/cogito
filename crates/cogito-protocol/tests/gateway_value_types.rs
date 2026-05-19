//! Serde round-trip + JSON shape tests for the small `gateway` value types.

use cogito_protocol::gateway::{ModelParams, StopReason, Usage};

#[test]
fn model_params_round_trip() -> serde_json::Result<()> {
    let mp = ModelParams {
        model: "claude-opus-4-7".into(),
        max_tokens: 4096,
        temperature: Some(0.7),
        top_p: None,
        stop_sequences: vec!["\n\nHuman:".into()],
    };
    let json = serde_json::to_string(&mp)?;
    let back: ModelParams = serde_json::from_str(&json)?;
    assert_eq!(mp, back);
    Ok(())
}

#[test]
fn stop_reason_snake_case_wire() -> serde_json::Result<()> {
    assert_eq!(serde_json::to_string(&StopReason::EndTurn)?, "\"end_turn\"");
    assert_eq!(serde_json::to_string(&StopReason::ToolUse)?, "\"tool_use\"");
    assert_eq!(
        serde_json::to_string(&StopReason::MaxTokens)?,
        "\"max_tokens\""
    );
    assert_eq!(
        serde_json::to_string(&StopReason::StopSequence)?,
        "\"stop_sequence\""
    );
    Ok(())
}

#[test]
fn usage_default_is_zero() {
    let u = Usage::default();
    assert_eq!(u.input_tokens, 0);
    assert_eq!(u.output_tokens, 0);
}

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor};

#[test]
fn message_user_wire() -> serde_json::Result<()> {
    let msg = Message::User {
        content: vec![ContentBlock::Text {
            text: "hello".into(),
        }],
    };
    let json = serde_json::to_value(&msg)?;
    assert_eq!(json["role"], "user");
    assert_eq!(json["content"][0]["type"], "text");
    let back: Message = serde_json::from_value(json)?;
    assert_eq!(msg, back);
    Ok(())
}

#[test]
fn message_assistant_with_tool_use_wire() -> serde_json::Result<()> {
    let msg = Message::Assistant {
        content: vec![
            ContentBlock::Text {
                text: "Let me check.".into(),
            },
            ContentBlock::ToolUse {
                call_id: "call_1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({ "path": "/etc/hosts" }),
            },
        ],
    };
    let back: Message = serde_json::from_str(&serde_json::to_string(&msg)?)?;
    assert_eq!(msg, back);
    Ok(())
}

#[test]
fn model_input_round_trip() -> serde_json::Result<()> {
    let mi = ModelInput {
        system: "You are helpful.".into(),
        messages: vec![Message::User {
            content: vec![ContentBlock::Text { text: "hi".into() }],
        }],
        tools: vec![ToolDescriptor {
            name: "read_file".into(),
            description: "Read a file.".into(),
            schema: serde_json::json!({ "type": "object" }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }],
        params: ModelParams {
            model: "test".into(),
            max_tokens: 256,
            temperature: None,
            top_p: None,
            stop_sequences: vec![],
        },
    };
    let json = serde_json::to_string(&mi)?;
    let back: ModelInput = serde_json::from_str(&json)?;
    let json_back = serde_json::to_string(&back)?;
    assert_eq!(json, json_back);
    Ok(())
}

use cogito_protocol::gateway::{ModelEvent, ModelOutput};

#[test]
fn model_event_text_delta_wire() -> serde_json::Result<()> {
    let evt = ModelEvent::TextDelta {
        block_index: 0,
        chunk: "hello".into(),
    };
    let json = serde_json::to_value(&evt)?;
    assert_eq!(json["kind"], "text_delta");
    assert_eq!(json["block_index"], 0);
    assert_eq!(json["chunk"], "hello");
    let back: ModelEvent = serde_json::from_value(json)?;
    assert_eq!(evt, back);
    Ok(())
}

#[test]
fn model_event_tool_use_completed_wire() -> serde_json::Result<()> {
    let evt = ModelEvent::ToolUseCompleted {
        block_index: 1,
        call_id: "call_abc".into(),
        name: "read_file".into(),
        args: serde_json::json!({ "path": "/tmp/x" }),
    };
    let back: ModelEvent = serde_json::from_str(&serde_json::to_string(&evt)?)?;
    assert_eq!(evt, back);
    Ok(())
}

#[test]
fn model_event_message_completed_carries_usage() -> serde_json::Result<()> {
    let evt = ModelEvent::MessageCompleted {
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
    };
    let back: ModelEvent = serde_json::from_str(&serde_json::to_string(&evt)?)?;
    assert_eq!(evt, back);
    Ok(())
}

#[test]
fn model_output_round_trip() -> serde_json::Result<()> {
    let mo = ModelOutput {
        content: vec![ContentBlock::Text {
            text: "hello".into(),
        }],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 3,
            output_tokens: 1,
        },
    };
    let back: ModelOutput = serde_json::from_str(&serde_json::to_string(&mo)?)?;
    assert_eq!(mo, back);
    Ok(())
}

use cogito_protocol::gateway::ModelError;

#[test]
fn model_error_display() {
    assert_eq!(
        ModelError::Network("connect refused".into()).to_string(),
        "network error: connect refused"
    );
    assert_eq!(
        ModelError::Provider {
            status: 500,
            message: "boom".into()
        }
        .to_string(),
        "provider error 500: boom"
    );
    assert_eq!(ModelError::Auth.to_string(), "auth failed");
    assert_eq!(
        ModelError::RateLimited {
            retry_after_secs: Some(30)
        }
        .to_string(),
        "rate limited (retry-after: Some(30))"
    );
    assert_eq!(ModelError::Cancelled.to_string(), "cancelled");
}

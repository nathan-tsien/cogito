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

//! Replay recorded OpenAI-compat SSE fixtures through the decoder and assert
//! the resulting `ModelEvent` sequence.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use cogito_model::sse::replay_openai_compat_into_model_events;
use cogito_protocol::gateway::{ModelEvent, StopReason};
use cogito_test_fixtures::sse_fixture;

#[test]
fn text_only_replay_yields_expected_sequence() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(sse_fixture("openai-compat-text-only.txt"))?;
    let events = replay_openai_compat_into_model_events(&bytes)?;

    let text_completed = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("TextBlockCompleted present");
    assert_eq!(text_completed, "Hello, world!");

    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
        "last event must be MessageCompleted(EndTurn), got {last:?}"
    );
    Ok(())
}

#[test]
fn tool_use_replay_seals_call_at_finish() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(sse_fixture("openai-compat-with-tool-use.txt"))?;
    let events = replay_openai_compat_into_model_events(&bytes)?;

    let (call_id, tool_name, args) = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::ToolUseCompleted {
                call_id,
                tool_name,
                args,
                ..
            } => Some((call_id.clone(), tool_name.clone(), args.clone())),
            _ => None,
        })
        .expect("ToolUseCompleted present");

    assert_eq!(call_id, "call_abc");
    assert_eq!(tool_name, "read_file");
    assert_eq!(args, serde_json::json!({ "path": "/tmp/x" }));

    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::ToolUse,
                ..
            }
        ),
        "last event must be MessageCompleted(ToolUse), got {last:?}"
    );
    Ok(())
}

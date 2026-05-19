//! Replay recorded Anthropic SSE fixtures through the decoder and assert
//! the resulting `ModelEvent` sequence.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use cogito_model::sse::replay_anthropic_into_model_events;
use cogito_protocol::gateway::{ModelEvent, StopReason};
use cogito_test_fixtures::sse_fixture;

#[test]
fn text_only_replay_yields_expected_sequence() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(sse_fixture("anthropic-text-only.txt"))?;
    let events = replay_anthropic_into_model_events(&bytes)?;
    // Expected sequence:
    //   TextDelta "Hello"
    //   TextDelta ", world!"
    //   TextBlockCompleted "Hello, world!"
    //   MessageCompleted (EndTurn)
    assert!(
        matches!(&events[0], ModelEvent::TextDelta { chunk, .. } if chunk == "Hello"),
        "expected TextDelta(Hello) at index 0, got {:?}",
        events[0]
    );
    assert!(
        matches!(&events[1], ModelEvent::TextDelta { chunk, .. } if chunk == ", world!"),
        "expected TextDelta(', world!') at index 1, got {:?}",
        events[1]
    );
    assert!(
        matches!(&events[2], ModelEvent::TextBlockCompleted { text, .. } if text == "Hello, world!"),
        "expected TextBlockCompleted at index 2, got {:?}",
        events[2]
    );
    let last = events.last().expect("non-empty");
    assert!(
        matches!(last, ModelEvent::MessageCompleted { stop_reason: StopReason::EndTurn, .. }),
        "expected MessageCompleted(EndTurn) at end, got {last:?}"
    );
    Ok(())
}

#[test]
fn tool_use_replay_yields_completed_event() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(sse_fixture("anthropic-with-tool-use.txt"))?;
    let events = replay_anthropic_into_model_events(&bytes)?;
    let (call_id, tool_name, args) = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::ToolUseCompleted { call_id, tool_name, args, .. } => {
                Some((call_id.clone(), tool_name.clone(), args.clone()))
            }
            _ => None,
        })
        .expect("ToolUseCompleted present");
    assert_eq!(call_id, "call_abc");
    assert_eq!(tool_name, "read_file");
    assert_eq!(args, serde_json::json!({ "path": "/tmp/x" }));
    let last = events.last().expect("non-empty");
    assert!(
        matches!(last, ModelEvent::MessageCompleted { stop_reason: StopReason::ToolUse, .. }),
        "expected MessageCompleted(ToolUse) at end, got {last:?}"
    );
    Ok(())
}

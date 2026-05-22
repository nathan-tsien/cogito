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
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
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
        "expected MessageCompleted(ToolUse) at end, got {last:?}"
    );
    Ok(())
}

#[test]
fn replays_thinking_block_with_signature() -> Result<(), Box<dyn std::error::Error>> {
    // Synthetic Anthropic Messages SSE: plain thinking block (two thinking_delta
    // chunks + one signature_delta), followed by message_delta + message_stop.
    let sse = b"event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-4-7\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"I should \"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"grep.\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_xyz\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let events = replay_anthropic_into_model_events(sse)?;
    let mut iter = events.iter();

    // First event: ThinkingDelta "I should "
    match iter.next().expect("event 0 present") {
        ModelEvent::ThinkingDelta {
            block_index: 0,
            chunk,
        } => {
            assert_eq!(chunk, "I should ", "first ThinkingDelta chunk mismatch");
        }
        other => panic!("expected ThinkingDelta(\"I should \"), got {other:?}"),
    }

    // Second event: ThinkingDelta "grep."
    match iter.next().expect("event 1 present") {
        ModelEvent::ThinkingDelta {
            block_index: 0,
            chunk,
        } => {
            assert_eq!(chunk, "grep.", "second ThinkingDelta chunk mismatch");
        }
        other => panic!("expected ThinkingDelta(\"grep.\"), got {other:?}"),
    }

    // Third event: ThinkingBlockCompleted with accumulated text and signature in provider_opaque.
    match iter.next().expect("event 2 present") {
        ModelEvent::ThinkingBlockCompleted {
            block_index: 0,
            text,
            provider_opaque,
        } => {
            assert_eq!(text, "I should grep.", "accumulated thinking text mismatch");
            assert_eq!(
                *provider_opaque,
                Some(serde_json::json!({"signature": "sig_xyz"})),
                "provider_opaque should carry signature"
            );
        }
        other => panic!("expected ThinkingBlockCompleted, got {other:?}"),
    }

    // Last event: MessageCompleted(EndTurn).
    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
        "expected MessageCompleted(EndTurn) at end, got {last:?}"
    );
    Ok(())
}

#[test]
fn replays_redacted_thinking_sealed_at_start() -> Result<(), Box<dyn std::error::Error>> {
    // Synthetic Anthropic Messages SSE: redacted_thinking block sealed at start
    // (no deltas), followed by message_delta + message_stop.
    let sse = b"event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-4-7\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"enc_blob\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":0}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let events = replay_anthropic_into_model_events(sse)?;

    // Exactly one ThinkingBlockCompleted event should be emitted for the
    // redacted block, sealed immediately at content_block_start.
    let completed = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted {
                block_index: 0,
                text,
                provider_opaque,
            } => Some((text.clone(), provider_opaque.clone())),
            _ => None,
        })
        .expect("ThinkingBlockCompleted must be present for redacted_thinking");
    assert_eq!(completed.0, "", "redacted thinking text must be empty");
    assert_eq!(
        completed.1,
        Some(serde_json::json!({"data": "enc_blob"})),
        "provider_opaque should carry enc_blob under data key"
    );

    // No ThinkingDelta events should appear for redacted blocks.
    let delta_count = events
        .iter()
        .filter(|e| matches!(e, ModelEvent::ThinkingDelta { .. }))
        .count();
    assert_eq!(
        delta_count, 0,
        "redacted thinking emits no ThinkingDelta events"
    );

    // Stream ends with MessageCompleted.
    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
        "expected MessageCompleted(EndTurn) at end, got {last:?}"
    );
    Ok(())
}

//! Wire-level integration tests for the `OpenAI` Responses SSE fixtures.
//!
//! Each fixture is a recorded SSE byte stream. We drive the decoder via
//! `replay_openai_responses_into_model_events` and assert the concrete
//! `ModelEvent` sequence — variant ordering and terminal stop reason —
//! rather than fixture string-matching, so semantic mapping regressions
//! surface here. The full async stream integration (network, cancel,
//! backpressure) is exercised by a manual live-API smoke test outside
//! CI; see ROADMAP §15.3.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use cogito_model::sse::replay_openai_responses_into_model_events;
use cogito_protocol::gateway::{ModelEvent, StopReason};

#[test]
fn text_completion_yields_text_block_and_end_turn() {
    let bytes = read_fixture("text_completion.sse");
    let events = replay_openai_responses_into_model_events(&bytes).expect("replay ok");

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ModelEvent::TextDelta { .. })),
        "expected at least one TextDelta, got {events:?}"
    );

    let text = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("TextBlockCompleted present");
    assert!(
        text.contains("Hello world"),
        "TextBlockCompleted text must include 'Hello world', got {text:?}"
    );

    let last = events.last().expect("non-empty");
    match last {
        ModelEvent::MessageCompleted { stop_reason, usage } => {
            assert_eq!(*stop_reason, StopReason::EndTurn);
            assert!(
                usage.input_tokens >= 1,
                "expected usage.input_tokens >= 1, got {}",
                usage.input_tokens
            );
        }
        other => panic!("last event must be MessageCompleted(EndTurn), got {other:?}"),
    }
}

#[test]
fn tool_call_yields_started_completed_and_end_turn() {
    let bytes = read_fixture("tool_call.sse");
    let events = replay_openai_responses_into_model_events(&bytes).expect("replay ok");

    // Locate the started + completed events for the tool call and assert
    // their relative ordering: started must precede completed.
    let started_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::ToolUseStarted { .. }))
        .expect("ToolUseStarted present");
    let completed_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::ToolUseCompleted { .. }))
        .expect("ToolUseCompleted present");
    assert!(
        started_pos < completed_pos,
        "ToolUseStarted must precede ToolUseCompleted (got {started_pos} vs {completed_pos})",
    );

    match &events[completed_pos] {
        ModelEvent::ToolUseCompleted {
            call_id,
            tool_name,
            args,
            ..
        } => {
            assert_eq!(call_id, "call_abc");
            assert_eq!(tool_name, "read_file");
            assert_eq!(args, &serde_json::json!({"path": "/etc/hosts"}));
        }
        other => panic!("expected ToolUseCompleted, got {other:?}"),
    }

    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
        "last event must be MessageCompleted(EndTurn), got {last:?}",
    );
}

#[test]
fn reasoning_summary_yields_thinking_then_text_then_end_turn() {
    let bytes = read_fixture("reasoning_summary.sse");
    let events = replay_openai_responses_into_model_events(&bytes).expect("replay ok");

    let thinking_delta_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::ThinkingDelta { .. }))
        .expect("ThinkingDelta present");
    let thinking_completed_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::ThinkingBlockCompleted { .. }))
        .expect("ThinkingBlockCompleted present");
    let text_delta_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::TextDelta { .. }))
        .expect("TextDelta present");
    let text_completed_pos = events
        .iter()
        .position(|e| matches!(e, ModelEvent::TextBlockCompleted { .. }))
        .expect("TextBlockCompleted present");

    assert!(thinking_delta_pos < thinking_completed_pos);
    assert!(thinking_completed_pos < text_delta_pos);
    assert!(text_delta_pos < text_completed_pos);

    match &events[thinking_completed_pos] {
        ModelEvent::ThinkingBlockCompleted { text, .. } => {
            assert!(
                text.contains("Considering the request"),
                "ThinkingBlockCompleted text must include 'Considering the request', got {text:?}",
            );
        }
        other => panic!("expected ThinkingBlockCompleted, got {other:?}"),
    }

    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ),
        "last event must be MessageCompleted(EndTurn), got {last:?}",
    );
}

#[test]
fn stop_reason_max_tokens_maps_to_max_tokens() {
    let bytes = read_fixture("stop_reason_max_tokens.sse");
    let events = replay_openai_responses_into_model_events(&bytes).expect("replay ok");

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ModelEvent::TextDelta { .. })),
        "expected at least one TextDelta, got {events:?}",
    );

    let last = events.last().expect("non-empty");
    assert!(
        matches!(
            last,
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::MaxTokens,
                ..
            }
        ),
        "last event must be MessageCompleted(MaxTokens), got {last:?}",
    );
}

fn read_fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("openai_responses")
        .join(name);
    fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

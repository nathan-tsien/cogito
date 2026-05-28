//! Wire-level integration tests for the `OpenAI` Responses SSE fixtures.
//!
//! Each fixture is a recorded SSE byte stream. We parse the `data:`
//! payload lines directly here and assert that the right event-type
//! discriminator appears in each fixture. The full async stream
//! integration is exercised by a manual live-API smoke test (not in
//! CI) — see ROADMAP §15.3.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

#[test]
fn text_completion_parses() {
    let raw = read_fixture("text_completion.sse");
    let events = parse_sse(&raw);
    assert!(
        events.iter().any(|e| e.contains("output_text.delta")),
        "text_completion fixture must include output_text.delta events: {events:?}"
    );
    assert!(
        events.iter().any(|e| e.contains("response.completed")),
        "text_completion fixture must include a response.completed event: {events:?}"
    );
}

#[test]
fn tool_call_parses() {
    let raw = read_fixture("tool_call.sse");
    let events = parse_sse(&raw);
    assert!(
        events
            .iter()
            .any(|e| e.contains("function_call_arguments.delta")),
        "tool_call fixture must include function_call_arguments.delta events: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e.contains("function_call_arguments.done")),
        "tool_call fixture must include a function_call_arguments.done event: {events:?}"
    );
}

#[test]
fn reasoning_summary_parses() {
    let raw = read_fixture("reasoning_summary.sse");
    let events = parse_sse(&raw);
    assert!(
        events
            .iter()
            .any(|e| e.contains("reasoning_summary_text.delta")),
        "reasoning_summary fixture must include reasoning_summary_text.delta: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e.contains("reasoning_summary_text.done")),
        "reasoning_summary fixture must include reasoning_summary_text.done: {events:?}"
    );
}

#[test]
fn stop_reason_max_tokens_parses() {
    let raw = read_fixture("stop_reason_max_tokens.sse");
    let events = parse_sse(&raw);
    assert!(
        events.last().unwrap().contains("max_output_tokens"),
        "incomplete_details.reason must be preserved through SSE encoding: {events:?}"
    );
}

fn read_fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("openai_responses")
        .join(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Minimal SSE event-data extractor: returns each `data:` payload line.
fn parse_sse(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|l| l.strip_prefix("data: ").map(str::to_string))
        .collect()
}

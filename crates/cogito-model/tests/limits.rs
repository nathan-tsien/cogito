//! Integration tests for `ModelGateway::model_limits` on `AnthropicGateway`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_model::anthropic::AnthropicGateway;
use cogito_protocol::gateway::ModelGateway;

fn make(model_id: &str) -> AnthropicGateway {
    AnthropicGateway::new_for_test("sk-fake", model_id)
}

#[test]
fn anthropic_opus_1m_suffix() {
    let g = make("claude-opus-4-7[1m]");
    assert_eq!(g.model_limits().context_window_tokens, 1_000_000);
    assert_eq!(g.model_limits().model_id, "claude-opus-4-7[1m]");
}

#[test]
fn anthropic_opus_default_200k() {
    let g = make("claude-opus-4-7");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_sonnet_default_200k() {
    let g = make("claude-sonnet-4-6");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_haiku_default_200k() {
    let g = make("claude-haiku-4-5");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_dated_haiku_default_200k() {
    let g = make("claude-haiku-4-5-20251001");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_unknown_model_falls_back_to_200k() {
    let g = make("claude-future-x-9");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_suffix_overrides_lookup() {
    let g = make("claude-sonnet-4-6[500k]");
    assert_eq!(g.model_limits().context_window_tokens, 500_000);
}

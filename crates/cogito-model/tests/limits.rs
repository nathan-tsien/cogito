//! Integration tests for `ModelGateway::model_limits` on `AnthropicGateway`
//! and `OpenAiCompatGateway`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_model::anthropic::AnthropicGateway;
use cogito_model::openai_compat::{OpenAiCompatConfig, OpenAiCompatGateway};
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

// --- OpenAiCompatGateway tests ---

fn oai(model_id: &str, declared: Option<u64>) -> OpenAiCompatGateway {
    let mut cfg = OpenAiCompatConfig::with_base_url("http://localhost:8000/v1");
    cfg.model = model_id.into();
    cfg.context_window_tokens = declared;
    OpenAiCompatGateway::new_for_test(cfg)
}

#[test]
fn oai_suffix_overrides_config() {
    let g = oai("Llama-3.3-70B[32k]", Some(8_000));
    assert_eq!(g.model_limits().context_window_tokens, 32_000);
}

#[test]
fn oai_config_used_when_no_suffix() {
    let g = oai("Llama-3.3-70B", Some(8_000));
    assert_eq!(g.model_limits().context_window_tokens, 8_000);
}

#[test]
fn oai_double_fallback_to_32768() {
    let g = oai("mystery-model", None);
    assert_eq!(g.model_limits().context_window_tokens, 32_768);
}

#[test]
fn oai_api_model_id_strips_suffix() {
    let g = oai("Llama-3.3-70B[32k]", None);
    assert_eq!(g.api_model_id(), "Llama-3.3-70B");
}

#[test]
fn oai_api_model_id_passes_through_when_no_suffix() {
    let g = oai("Llama-3.3-70B", None);
    assert_eq!(g.api_model_id(), "Llama-3.3-70B");
}

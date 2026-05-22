#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use cogito_protocol::gateway::parse_context_window_suffix;

#[test]
fn opus_with_1m_suffix() {
    assert_eq!(
        parse_context_window_suffix("claude-opus-4-7[1m]"),
        Some(1_000_000)
    );
}

#[test]
fn sonnet_with_200k_suffix() {
    assert_eq!(
        parse_context_window_suffix("claude-sonnet-4-6[200k]"),
        Some(200_000)
    );
}

#[test]
fn vllm_with_32k_suffix() {
    assert_eq!(
        parse_context_window_suffix("meta-llama/Llama-3.3-70B[32k]"),
        Some(32_000)
    );
}

#[test]
fn gpt_with_numeric_suffix() {
    assert_eq!(parse_context_window_suffix("gpt-4o[128000]"), Some(128_000));
}

#[test]
fn capital_k_and_m_accepted() {
    assert_eq!(parse_context_window_suffix("foo[1M]"), Some(1_000_000));
    assert_eq!(parse_context_window_suffix("bar[32K]"), Some(32_000));
}

#[test]
fn no_suffix_returns_none() {
    assert_eq!(parse_context_window_suffix("claude-opus-4-7"), None);
    assert_eq!(parse_context_window_suffix("gpt-4o"), None);
}

#[test]
fn malformed_suffix_returns_none() {
    assert_eq!(parse_context_window_suffix("model[abc]"), None);
    assert_eq!(parse_context_window_suffix("model[1g]"), None);
    assert_eq!(parse_context_window_suffix("model[]"), None);
    assert_eq!(parse_context_window_suffix("model[1m"), None);
}

#[test]
fn suffix_in_middle_ignored() {
    // suffix must be at end-of-string anchor
    assert_eq!(parse_context_window_suffix("model[1m]-v2"), None);
}

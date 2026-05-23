//! Tests for layered partial merge and `finalize` defaults / validation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use cogito_config::{RuntimeConfigPartial, RuntimeSectionPartial, merge_layers};
use cogito_model::ProviderConfig;

fn partial_with_model(model: &str) -> RuntimeConfigPartial {
    RuntimeConfigPartial {
        runtime: Some(RuntimeSectionPartial {
            default_model: Some(model.into()),
            ..Default::default()
        }),
        providers: None,
        mcp_servers: None,
        skills: None,
    }
}

fn anthropic_provider(name: &str) -> ProviderConfig {
    ProviderConfig::Anthropic {
        name: name.into(),
        api_key: "k".into(),
        base_url: "https://api.anthropic.com".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    }
}

#[test]
fn later_layer_overrides_earlier() {
    let merged = merge_layers(vec![
        partial_with_model("claude-sonnet-4-6"),
        partial_with_model("claude-opus-4-7"),
    ]);
    let rt = merged.runtime.unwrap();
    assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn later_some_does_not_overwrite_with_none() {
    let merged = merge_layers(vec![
        partial_with_model("claude-opus-4-7"),
        RuntimeConfigPartial::default(),
    ]);
    let rt = merged.runtime.unwrap();
    assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn providers_array_replaces_wholesale() {
    let layer_a = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
        mcp_servers: None,
        skills: None,
    };
    let layer_b = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("c")]),
        mcp_servers: None,
        skills: None,
    };
    let merged = merge_layers(vec![layer_a, layer_b]);
    assert_eq!(merged.providers.as_ref().unwrap().len(), 1);
    assert_eq!(merged.providers.as_ref().unwrap()[0].name(), "c");
}

#[test]
fn finalize_fills_defaults() {
    let partial = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("only")]),
        mcp_servers: None,
        skills: None,
    };
    let cfg = partial.finalize().expect("ok");
    assert_eq!(cfg.runtime.session_root, PathBuf::from("./sessions"));
    assert_eq!(cfg.runtime.strategies_dir, PathBuf::from("./strategies"));
    // Auto-select rule: one provider, no explicit default_provider.
    assert_eq!(cfg.runtime.default_provider.as_deref(), Some("only"));
    assert!(cfg.runtime.default_model.is_none());
}

#[test]
fn finalize_preserves_explicit_default_provider() {
    let partial = RuntimeConfigPartial {
        runtime: Some(RuntimeSectionPartial {
            default_provider: Some("a".into()),
            ..Default::default()
        }),
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
        mcp_servers: None,
        skills: None,
    };
    let cfg = partial.finalize().expect("ok");
    assert_eq!(cfg.runtime.default_provider.as_deref(), Some("a"));
}

#[test]
fn finalize_ambiguous_provider_errors() {
    let partial = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
        mcp_servers: None,
        skills: None,
    };
    let err = partial.finalize().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("multiple providers") || msg.contains("default_provider"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn finalize_empty_providers_yields_empty_runtime() {
    // Sprint 4.5 legacy bridge must run AFTER finalize when this happens.
    // finalize itself does NOT error on empty providers — it leaves them
    // empty so the caller (cogito-cli) can synthesize a default.
    let partial = RuntimeConfigPartial::default();
    let cfg = partial.finalize().expect("ok");
    assert!(cfg.providers.is_empty());
    assert!(cfg.runtime.default_provider.is_none());
}

#[test]
fn openai_compat_with_context_window_tokens() {
    // Verify that `context_window_tokens` is accepted in [[providers]] and
    // survives deserialization into ProviderConfig::OpenAiCompat.
    let toml_str = r#"
        [[providers]]
        kind = "openai-compat"
        name = "local"
        base_url = "http://localhost:8000/v1"
        context_window_tokens = 32768
    "#;
    let partial: RuntimeConfigPartial = toml::from_str(toml_str).expect("parse ok");
    let providers = partial.providers.expect("providers present");
    assert_eq!(providers.len(), 1);
    let ProviderConfig::OpenAiCompat {
        name,
        context_window_tokens,
        ..
    } = &providers[0]
    else {
        panic!("expected OpenAiCompat variant");
    };
    assert_eq!(name, "local");
    assert_eq!(*context_window_tokens, Some(32_768));
}

#[test]
fn openai_compat_without_context_window_tokens_defaults_to_none() {
    // Existing configs without the field must still parse correctly.
    let toml_str = r#"
        [[providers]]
        kind = "openai-compat"
        name = "local"
        base_url = "http://localhost:8000/v1"
    "#;
    let partial: RuntimeConfigPartial = toml::from_str(toml_str).expect("parse ok");
    let providers = partial.providers.expect("providers present");
    let ProviderConfig::OpenAiCompat {
        context_window_tokens,
        ..
    } = &providers[0]
    else {
        panic!("expected OpenAiCompat variant");
    };
    assert_eq!(*context_window_tokens, None);
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tests for `cogito_model::ProviderConfig` deserialization + factory.

use std::sync::Arc;

use cogito_model::{ProviderConfig, build_gateway};
use cogito_protocol::gateway::ModelGateway;

#[test]
fn anthropic_deserializes_with_defaults() {
    let toml_str = r#"
        name = "anthropic-prod"
        kind = "anthropic"
        api_key = "sk-test"
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::Anthropic {
            name,
            api_key,
            base_url,
            anthropic_version,
            timeout_secs,
        } => {
            assert_eq!(name, "anthropic-prod");
            assert_eq!(api_key, "sk-test");
            assert_eq!(base_url, "https://api.anthropic.com");
            assert_eq!(anthropic_version, "2023-06-01");
            assert!(timeout_secs.is_none());
        }
        ProviderConfig::OpenAiCompat { .. } => panic!("expected Anthropic variant"),
    }
}

#[test]
fn anthropic_deserializes_with_overrides() {
    let toml_str = r#"
        name = "anthropic-internal"
        kind = "anthropic"
        api_key = "key"
        base_url = "https://internal.api/anthropic/v1"
        anthropic_version = "2024-01-01"
        timeout_secs = 120
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::Anthropic {
            base_url,
            anthropic_version,
            timeout_secs,
            ..
        } => {
            assert_eq!(base_url, "https://internal.api/anthropic/v1");
            assert_eq!(anthropic_version, "2024-01-01");
            assert_eq!(timeout_secs, Some(120));
        }
        ProviderConfig::OpenAiCompat { .. } => panic!("expected Anthropic variant"),
    }
}

#[test]
fn openai_compat_deserializes() {
    let toml_str = r#"
        name = "vllm"
        kind = "openai-compat"
        base_url = "http://vllm:8000/v1"
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::OpenAiCompat {
            name,
            base_url,
            api_key,
            auth_header,
            auth_scheme,
            ..
        } => {
            assert_eq!(name, "vllm");
            assert_eq!(base_url, "http://vllm:8000/v1");
            assert!(api_key.is_none());
            assert_eq!(auth_header, "Authorization");
            assert_eq!(auth_scheme, "Bearer");
        }
        ProviderConfig::Anthropic { .. } => panic!("expected OpenAiCompat variant"),
    }
}

#[test]
fn unknown_kind_errors() {
    let toml_str = r#"
        name = "x"
        kind = "no-such-kind"
    "#;
    let err = toml::from_str::<ProviderConfig>(toml_str).unwrap_err();
    assert!(
        err.to_string().contains("no-such-kind") || err.to_string().contains("unknown variant")
    );
}

#[test]
fn unknown_field_errors() {
    let toml_str = r#"
        name = "x"
        kind = "anthropic"
        api_key = "k"
        bogus_field = "boom"
    "#;
    let err = toml::from_str::<ProviderConfig>(toml_str).unwrap_err();
    assert!(err.to_string().contains("bogus_field") || err.to_string().contains("unknown field"));
}

#[test]
fn build_anthropic_gateway() {
    let cfg = ProviderConfig::Anthropic {
        name: "x".into(),
        api_key: "sk".into(),
        base_url: "https://internal.api/anthropic/v1".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    };
    let gw: Arc<dyn ModelGateway> = build_gateway(cfg).expect("build");
    assert_eq!(gw.provider_id(), "anthropic");
}

#[test]
fn build_openai_compat_gateway() {
    let cfg = ProviderConfig::OpenAiCompat {
        name: "x".into(),
        api_key: Some("k".into()),
        base_url: "http://localhost:8000/v1".into(),
        auth_header: "Authorization".into(),
        auth_scheme: "Bearer".into(),
        timeout_secs: None,
    };
    let gw: Arc<dyn ModelGateway> = build_gateway(cfg).expect("build");
    assert_eq!(gw.provider_id(), "openai-compat");
}

#[test]
fn provider_config_name_accessor() {
    let cfg = ProviderConfig::Anthropic {
        name: "anthropic-prod".into(),
        api_key: "k".into(),
        base_url: "https://api.anthropic.com".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    };
    assert_eq!(cfg.name(), "anthropic-prod");
}

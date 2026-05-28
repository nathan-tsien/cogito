//! End-to-end tests for [`cogito_cli::chat::resolve_strategy`].
//!
//! These exercise the resolution table from the Sprint 9a spec §12.1:
//! synthesized default, registry hit, `--model` override, unknown
//! strategy, missing provider. Uses the in-memory
//! [`MapStrategyRegistry`] fixture instead of touching disk so the
//! tests stay deterministic and fast.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;

use cogito_cli::chat::{ChatArgs, ResolveError, resolve_strategy};
use cogito_config::{RuntimeConfig, RuntimeSection};
use cogito_model::ProviderConfig;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_test_fixtures::strategy::MapStrategyRegistry;

fn cfg_with_provider() -> RuntimeConfig {
    RuntimeConfig {
        runtime: RuntimeSection {
            session_root: PathBuf::from("./sessions"),
            default_provider: Some("anthropic-default".into()),
            default_model: Some("claude-opus-4-7".into()),
            default_strategy: None,
            strategies_dir: PathBuf::from(".cogito/strategies"),
        },
        providers: vec![ProviderConfig::Anthropic {
            name: "anthropic-default".into(),
            api_key: "k".into(),
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout_secs: None,
        }],
        strategies: HashMap::new(),
        mcp_servers: vec![],
        mcp_parse_failures: vec![],
        skills: None,
    }
}

fn args_with(strategy: Option<&str>, model: Option<&str>) -> ChatArgs {
    ChatArgs {
        strategy: strategy.map(String::from),
        model: model.map(String::from),
        ..ChatArgs::default()
    }
}

#[test]
fn synthesized_default_when_no_strategy() {
    let reg = MapStrategyRegistry::new();
    let cfg = cfg_with_provider();
    let args = args_with(None, None);
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.name, "default");
    assert_eq!(strategy.model_params.model, "claude-opus-4-7");
}

#[test]
fn registry_hit_when_strategy_provided() {
    let mut s = HarnessStrategy::default_with_model("");
    s.name = "coder".into();
    s.system_prompt = "be precise".into();
    s.model_params.model = "claude-sonnet-4-6".into();

    let reg = MapStrategyRegistry::new().with("coder", s);
    let cfg = cfg_with_provider();
    let args = args_with(Some("coder"), None);
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.name, "coder");
    assert_eq!(strategy.model_params.model, "claude-sonnet-4-6");
    assert_eq!(strategy.system_prompt, "be precise");
}

#[test]
fn model_flag_overrides_strategy_model() {
    let mut s = HarnessStrategy::default_with_model("");
    s.name = "coder".into();
    s.model_params.model = "claude-sonnet-4-6".into();
    let reg = MapStrategyRegistry::new().with("coder", s);

    let cfg = cfg_with_provider();
    let args = args_with(Some("coder"), Some("claude-opus-4-7"));
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.model_params.model, "claude-opus-4-7");
}

#[test]
fn unknown_strategy_returns_error_with_available() {
    let reg = MapStrategyRegistry::new();
    let cfg = cfg_with_provider();
    let args = args_with(Some("nope"), None);
    let err = resolve_strategy(&args, &cfg, &reg).unwrap_err();
    assert!(matches!(err, ResolveError::UnknownStrategy { ref name, .. } if name == "nope"));
}

#[test]
fn missing_provider_returns_error() {
    let mut cfg = cfg_with_provider();
    cfg.runtime.default_provider = None;
    cfg.providers.clear();
    let reg = MapStrategyRegistry::new();
    let args = args_with(None, Some("any"));
    let err = resolve_strategy(&args, &cfg, &reg).unwrap_err();
    assert!(matches!(err, ResolveError::MissingProvider));
}

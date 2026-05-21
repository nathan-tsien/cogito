//! Issue #1 sub-need 1: no cogito.toml, only `ANTHROPIC_API_KEY` or
//! `OPENAI_BASE_URL` set. Sprint 4.5 must reproduce Sprint 2 behaviour.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

const SCRUBBED_KEYS: [(&str, Option<&str>); 9] = [
    ("COGITO_CONFIG", None),
    ("ANTHROPIC_API_KEY", None),
    ("OPENAI_API_KEY", None),
    ("OPENAI_BASE_URL", None),
    ("COGITO_SESSION_ROOT", None),
    ("COGITO_DEFAULT_PROVIDER", None),
    ("COGITO_DEFAULT_MODEL", None),
    ("COGITO_STRATEGIES_DIR", None),
    ("XDG_CONFIG_HOME", None),
];

#[tokio::test]
async fn legacy_anthropic_bridge() {
    let _g = ENV_LOCK.lock().await;
    let mut vars: Vec<(&str, Option<&str>)> = SCRUBBED_KEYS.to_vec();
    vars[1] = ("ANTHROPIC_API_KEY", Some("sk-test-bridge"));
    vars[8] = ("XDG_CONFIG_HOME", Some("/nonexistent/path"));

    temp_env::async_with_vars(vars, async {
        let inputs = ChatConfigInputs {
            model: Some("claude-opus-4-7".into()),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        assert!(
            cfg.providers.is_empty(),
            "no cogito.toml means empty providers pre-bridge"
        );

        let chosen = select_provider(&cfg, &inputs).expect("select with bridge");
        match chosen {
            ProviderConfig::Anthropic {
                name,
                api_key,
                base_url,
                ..
            } => {
                assert_eq!(name, "default");
                assert_eq!(api_key, "sk-test-bridge");
                assert_eq!(base_url, "https://api.anthropic.com");
            }
            ProviderConfig::OpenAiCompat { .. } => panic!("expected Anthropic"),
        }
    })
    .await;
}

#[tokio::test]
async fn legacy_openai_compat_bridge() {
    let _g = ENV_LOCK.lock().await;
    let mut vars: Vec<(&str, Option<&str>)> = SCRUBBED_KEYS.to_vec();
    vars[3] = ("OPENAI_BASE_URL", Some("http://vllm.internal:8000/v1"));
    vars[2] = ("OPENAI_API_KEY", Some("sk-openai"));
    vars[8] = ("XDG_CONFIG_HOME", Some("/nonexistent/path"));

    temp_env::async_with_vars(vars, async {
        let inputs = ChatConfigInputs {
            model: Some("qwen-72b".into()),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        let chosen = select_provider(&cfg, &inputs).expect("select");
        match chosen {
            ProviderConfig::OpenAiCompat {
                name,
                base_url,
                api_key,
                ..
            } => {
                assert_eq!(name, "default");
                assert_eq!(base_url, "http://vllm.internal:8000/v1");
                assert_eq!(api_key.as_deref(), Some("sk-openai"));
            }
            ProviderConfig::Anthropic { .. } => panic!("expected OpenAiCompat"),
        }
    })
    .await;
}

//! Issue #1 sub-need 2: same kind=anthropic but pointing at an
//! internal endpoint. Two providers coexist (prod + internal); the
//! user picks via `runtime.default_provider` in the file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn two_anthropic_providers_internal_selected() {
    let _g = ENV_LOCK.lock().await;

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_provider = "anthropic-internal"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "k-prod"
            base_url = "https://api.anthropic.com"

            [[providers]]
            name = "anthropic-internal"
            kind = "anthropic"
            api_key = "k-internal"
            base_url = "https://internal.api/anthropic/v1"
        "#,
    )
    .unwrap();

    let vars: Vec<(&str, Option<&str>)> = vec![("COGITO_CONFIG", None), ("XDG_CONFIG_HOME", None)];

    let path_for_inputs = path.clone();
    temp_env::async_with_vars(vars, async move {
        let inputs = ChatConfigInputs {
            config_path: Some(path_for_inputs),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        assert_eq!(cfg.providers.len(), 2);

        let chosen = select_provider(&cfg, &inputs).expect("select");
        match chosen {
            ProviderConfig::Anthropic {
                name,
                api_key,
                base_url,
                ..
            } => {
                assert_eq!(name, "anthropic-internal");
                assert_eq!(api_key, "k-internal");
                assert_eq!(base_url, "https://internal.api/anthropic/v1");
            }
            ProviderConfig::OpenAiCompat { .. } | ProviderConfig::OpenAiResponses { .. } => {
                panic!("expected Anthropic")
            }
        }
    })
    .await;
}

#[tokio::test]
async fn cli_provider_flag_overrides_file_default() {
    let _g = ENV_LOCK.lock().await;

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_provider = "anthropic-prod"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "k-prod"

            [[providers]]
            name = "anthropic-internal"
            kind = "anthropic"
            api_key = "k-internal"
            base_url = "https://internal/v1"
        "#,
    )
    .unwrap();

    let vars: Vec<(&str, Option<&str>)> = vec![("COGITO_CONFIG", None), ("XDG_CONFIG_HOME", None)];

    let path_for_inputs = path.clone();
    temp_env::async_with_vars(vars, async move {
        let inputs = ChatConfigInputs {
            config_path: Some(path_for_inputs),
            provider: Some("anthropic-internal".into()),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        let chosen = select_provider(&cfg, &inputs).expect("select");
        match chosen {
            ProviderConfig::Anthropic { name, .. } => {
                assert_eq!(name, "anthropic-internal");
            }
            ProviderConfig::OpenAiCompat { .. } | ProviderConfig::OpenAiResponses { .. } => {
                panic!("expected Anthropic")
            }
        }
    })
    .await;
}

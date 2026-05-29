//! `--base-url` flag must override the chosen provider's `base_url`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn cli_base_url_overrides_file_base_url() {
    let _g = ENV_LOCK.lock().await;

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [[providers]]
            name = "anthropic-only"
            kind = "anthropic"
            api_key = "k"
            base_url = "https://from-file"
        "#,
    )
    .unwrap();

    let vars: Vec<(&str, Option<&str>)> = vec![("COGITO_CONFIG", None), ("XDG_CONFIG_HOME", None)];

    let path_for_inputs = path.clone();
    temp_env::async_with_vars(vars, async move {
        let inputs = ChatConfigInputs {
            config_path: Some(path_for_inputs),
            base_url: Some("https://from-cli".into()),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        let chosen = select_provider(&cfg, &inputs).expect("select");
        match chosen {
            ProviderConfig::Anthropic { base_url, .. } => {
                assert_eq!(base_url, "https://from-cli");
            }
            ProviderConfig::OpenAiCompat { .. } | ProviderConfig::OpenAiResponses { .. } => {
                panic!("expected Anthropic")
            }
        }
    })
    .await;
}

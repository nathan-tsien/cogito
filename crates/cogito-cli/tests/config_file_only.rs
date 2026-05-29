//! File-only configuration: cogito.toml declares an Anthropic provider
//! with `${ANTHROPIC_API_KEY}` interpolated.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn file_declares_anthropic_with_env_interpolation() {
    let _g = ENV_LOCK.lock().await;

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "./sessions"
            default_model = "claude-opus-4-7"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "${ANTHROPIC_API_KEY}"
            base_url = "https://api.anthropic.com"
        "#,
    )
    .unwrap();

    let vars: Vec<(&str, Option<&str>)> = vec![
        ("COGITO_CONFIG", None),
        ("XDG_CONFIG_HOME", None),
        ("ANTHROPIC_API_KEY", Some("sk-file-test")),
    ];

    let path_for_inputs = path.clone();
    temp_env::async_with_vars(vars, async move {
        let inputs = ChatConfigInputs {
            config_path: Some(path_for_inputs),
            ..Default::default()
        };
        let cfg = load_layered_config(&inputs).await.expect("load");
        assert_eq!(
            cfg.runtime.default_provider.as_deref(),
            Some("anthropic-prod")
        );
        let chosen = select_provider(&cfg, &inputs).expect("select");
        match chosen {
            ProviderConfig::Anthropic { name, api_key, .. } => {
                assert_eq!(name, "anthropic-prod");
                assert_eq!(api_key, "sk-file-test");
            }
            ProviderConfig::OpenAiCompat { .. } | ProviderConfig::OpenAiResponses { .. } => {
                panic!("expected Anthropic")
            }
        }
    })
    .await;
}

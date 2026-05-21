//! Configuration helpers for `cogito chat`. Exposed `pub` so the
//! integration tests under `crates/cogito-cli/tests/` can exercise
//! the boundary without going through the full Runtime.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use cogito_config::{
    ConfigLoader, EnvConfigLoader, FileConfigLoader, RuntimeConfig, RuntimeConfigPartial,
    RuntimeSectionPartial, merge_layers,
};
use cogito_model::ProviderConfig;

/// Subset of `ChatArgs` needed by the config helpers. The CLI build
/// passes a real `ChatArgs`; tests pass a `ChatConfigInputs` directly.
#[derive(Debug, Default, Clone)]
pub struct ChatConfigInputs {
    /// Path to a `cogito.toml`. Highest precedence in the search path.
    pub config_path: Option<PathBuf>,
    /// Model identifier. Forwarded to `runtime.default_model` in the
    /// CLI layer.
    pub model: Option<String>,
    /// Provider name. Forwarded to `runtime.default_provider` in the
    /// CLI layer.
    pub provider: Option<String>,
    /// Base URL override applied post-merge to the selected provider.
    pub base_url: Option<String>,
    /// Directory where per-session JSONL files are stored. Forwarded
    /// to `runtime.session_root` in the CLI layer.
    pub session_root: Option<PathBuf>,
}

/// Load the three-layer (`file > env > cli`) configuration and
/// finalize.
pub async fn load_layered_config(inputs: &ChatConfigInputs) -> Result<RuntimeConfig> {
    let file = FileConfigLoader::resolve(inputs.config_path.as_ref())
        .context("resolving config file path")?;
    let env = EnvConfigLoader;
    let cli_partial = cli_inputs_to_partial(inputs);

    let layers = vec![
        file.load().await.context("loading config file")?,
        env.load().await.context("loading environment")?,
        cli_partial,
    ];
    merge_layers(layers)
        .finalize()
        .map_err(|e| anyhow!("finalizing config: {e}"))
}

/// Convert CLI-style inputs into a `RuntimeConfigPartial` for use as
/// the highest-precedence merge layer. Only the inputs that
/// correspond to `[runtime]` table fields are forwarded; `config_path`
/// selects the file, `base_url` is applied post-merge in
/// `select_provider`.
fn cli_inputs_to_partial(inputs: &ChatConfigInputs) -> RuntimeConfigPartial {
    let any = inputs.model.is_some() || inputs.provider.is_some() || inputs.session_root.is_some();
    RuntimeConfigPartial {
        runtime: any.then(|| RuntimeSectionPartial {
            session_root: inputs.session_root.clone(),
            default_provider: inputs.provider.clone(),
            default_model: inputs.model.clone(),
            strategies_dir: None,
        }),
        providers: None,
        mcp_servers: None,
    }
}

/// Synthesize a `default` provider from legacy environment variables
/// when no `cogito.toml` and no explicit providers are declared. This
/// preserves the Sprint 2 workflow: `cogito chat --model claude-opus-4-7`
/// with only `ANTHROPIC_API_KEY` set continues to work.
///
/// Selection follows Sprint 2 inference: `claude-*` models route to
/// Anthropic, otherwise OpenAI-compat.
///
/// `cli_base_url` is the value of the CLI `--base-url` flag; it is
/// used as a fallback for `OPENAI_BASE_URL` so Sprint 2 invocations
/// like `cogito chat --model gpt-4o --base-url http://...` continue
/// to work without `OPENAI_BASE_URL` set.
pub fn synthesize_legacy_provider(
    model: &str,
    cli_base_url: Option<&str>,
) -> Result<ProviderConfig> {
    if model.starts_with("claude-") || std::env::var("ANTHROPIC_API_KEY").is_ok() {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set (no cogito.toml found either)")?;
        Ok(ProviderConfig::Anthropic {
            name: "default".into(),
            api_key: key,
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout_secs: None,
        })
    } else {
        let base_url = std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| cli_base_url.map(String::from))
            .context(
                "OPENAI_BASE_URL not set and no cogito.toml found; \
                 set OPENAI_BASE_URL, pass --base-url, or declare providers in a config file",
            )?;
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        Ok(ProviderConfig::OpenAiCompat {
            name: "default".into(),
            api_key,
            base_url,
            auth_header: "Authorization".into(),
            auth_scheme: "Bearer".into(),
            timeout_secs: None,
        })
    }
}

/// Pick the provider entry for this run. Resolution order:
///
/// 1. If `cfg.providers` is empty, synthesize from legacy ENV
///    (Sprint 2 bridge); `inputs.base_url` is used as fallback for
///    `OPENAI_BASE_URL`.
/// 2. Else if `cfg.runtime.default_provider` is set, look it up by
///    name; error if not found.
/// 3. Else (auto-select already applied by `finalize`), error.
///
/// Then apply CLI `--base-url` as a post-merge field patch on the
/// chosen provider.
pub fn select_provider(cfg: &RuntimeConfig, inputs: &ChatConfigInputs) -> Result<ProviderConfig> {
    let model_for_synth = inputs
        .model
        .as_deref()
        .or(cfg.runtime.default_model.as_deref())
        .unwrap_or("");

    let mut chosen = if cfg.providers.is_empty() {
        synthesize_legacy_provider(model_for_synth, inputs.base_url.as_deref())?
    } else {
        let name =
            cfg.runtime.default_provider.as_deref().ok_or_else(|| {
                anyhow!("no default_provider selected and no auto-select possible")
            })?;
        cfg.providers
            .iter()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| anyhow!("provider `{name}` not found in config"))?
    };

    if let Some(b) = &inputs.base_url {
        chosen = patch_base_url(chosen, b.clone());
    }
    Ok(chosen)
}

/// Replace the `base_url` field on the chosen provider, preserving
/// every other field.
#[must_use]
pub fn patch_base_url(cfg: ProviderConfig, new_base_url: String) -> ProviderConfig {
    match cfg {
        ProviderConfig::Anthropic {
            name,
            api_key,
            anthropic_version,
            timeout_secs,
            ..
        } => ProviderConfig::Anthropic {
            name,
            api_key,
            base_url: new_base_url,
            anthropic_version,
            timeout_secs,
        },
        ProviderConfig::OpenAiCompat {
            name,
            api_key,
            auth_header,
            auth_scheme,
            timeout_secs,
            ..
        } => ProviderConfig::OpenAiCompat {
            name,
            api_key,
            base_url: new_base_url,
            auth_header,
            auth_scheme,
            timeout_secs,
        },
    }
}

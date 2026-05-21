//! `cogito chat` — interactive REPL subcommand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use cogito_config::{
    ConfigLoader, EnvConfigLoader, FileConfigLoader, RuntimeConfig, RuntimeConfigPartial,
    RuntimeSectionPartial, merge_layers,
};
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_model::{ProviderConfig, build_gateway};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use tokio::io::{self, AsyncBufReadExt, BufReader};

/// Arguments for the `chat` subcommand.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Path to a `cogito.toml`. Highest precedence in the search path.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Model identifier (e.g. `claude-opus-4-7`, `gpt-4o`). Overrides
    /// `runtime.default_model` from the config.
    #[arg(long)]
    pub model: Option<String>,

    /// Provider name (matches `[[providers]] name = "..."` in the config).
    /// Overrides `runtime.default_provider`.
    #[arg(long)]
    pub provider: Option<String>,

    /// Base URL override applied to the selected provider AFTER merge.
    #[arg(long)]
    pub base_url: Option<String>,

    /// Directory where per-session JSONL files are stored. Overrides
    /// `runtime.session_root`.
    #[arg(long)]
    pub session_root: Option<PathBuf>,

    /// Resume an existing session by ULID. A new session is created if omitted.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,
}

/// Build the layered configuration: file + env + CLI args (in
/// ascending precedence), merge, finalize.
async fn load_layered_config(args: &ChatArgs) -> Result<RuntimeConfig> {
    let file =
        FileConfigLoader::resolve(args.config.as_ref()).context("resolving config file path")?;
    let env = EnvConfigLoader;
    let cli_partial = cli_args_to_partial(args);

    let layers = vec![
        file.load().await.context("loading config file")?,
        env.load().await.context("loading environment")?,
        cli_partial,
    ];
    merge_layers(layers)
        .finalize()
        .map_err(|e| anyhow!("finalizing config: {e}"))
}

/// Convert CLI args into a `RuntimeConfigPartial` for use as the
/// highest-precedence merge layer. Only the args that correspond to
/// `[runtime]` table fields are forwarded; `--config` selects the file,
/// `--base-url` is applied post-merge in `select_provider`, and
/// `--system` / `--session-id` are session-scoped, not config-scoped.
fn cli_args_to_partial(args: &ChatArgs) -> RuntimeConfigPartial {
    let any = args.model.is_some() || args.provider.is_some() || args.session_root.is_some();
    RuntimeConfigPartial {
        runtime: any.then(|| RuntimeSectionPartial {
            session_root: args.session_root.clone(),
            default_provider: args.provider.clone(),
            default_model: args.model.clone(),
            strategies_dir: None,
        }),
        providers: None,
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
fn synthesize_legacy_provider(model: &str, cli_base_url: Option<&str>) -> Result<ProviderConfig> {
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
///    (Sprint 2 bridge).
/// 2. Else if `cfg.runtime.default_provider` is set, look it up by
///    name; error if not found.
/// 3. Else (auto-select rule already applied by `finalize`), if
///    exactly one provider exists, use it.
/// 4. Else, error.
///
/// Then apply CLI `--base-url` as a post-merge field patch on the
/// chosen provider.
fn select_provider(cfg: &RuntimeConfig, args: &ChatArgs) -> Result<ProviderConfig> {
    let model_for_synth = args
        .model
        .as_deref()
        .or(cfg.runtime.default_model.as_deref())
        .unwrap_or("");

    let mut chosen = if cfg.providers.is_empty() {
        synthesize_legacy_provider(model_for_synth, args.base_url.as_deref())?
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

    if let Some(b) = &args.base_url {
        chosen = patch_base_url(chosen, b.clone());
    }
    Ok(chosen)
}

fn patch_base_url(cfg: ProviderConfig, new_base_url: String) -> ProviderConfig {
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

/// Entry point for the `chat` subcommand.
// `print!` / `println!` are intentional here: the chat REPL must write model
// output to stdout without tracing timestamps or log levels.
#[allow(clippy::print_stdout)]
pub async fn run(args: ChatArgs) -> Result<()> {
    let cfg = load_layered_config(&args).await?;
    let provider_cfg = select_provider(&cfg, &args)?;
    let gateway: Arc<dyn ModelGateway> =
        build_gateway(provider_cfg).map_err(|e| anyhow!("building gateway: {e}"))?;

    let model_id = args
        .model
        .clone()
        .or_else(|| cfg.runtime.default_model.clone())
        .ok_or_else(|| anyhow!("--model required (or set runtime.default_model in cogito.toml)"))?;

    let store = Arc::new(JsonlStore::new(cfg.runtime.session_root.clone()));
    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let mut strategy = HarnessStrategy::default_with_model(&model_id);
    if let Some(sys) = args.system {
        strategy.system_prompt = sys;
    }

    let runtime = Runtime::builder()
        .store(store)
        .model(gateway)
        .tools(tools)
        .strategy(strategy)
        .build()
        .context("building runtime")?;

    // Parse or generate the session ID.
    let session_id = match args.session_id {
        Some(s) => s
            .parse::<SessionId>()
            .context("invalid session_id (need ULID)")?,
        None => SessionId::new(),
    };

    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .map_err(|e| anyhow!("open_session: {e:?}"))?;

    tracing::info!(%session_id, "cogito chat started (type /quit to exit, Ctrl-C to cancel turn)");

    // Spawn a Ctrl-C handler that cancels the current in-flight turn without
    // terminating the REPL — the user can keep typing after cancellation.
    let cancel_handle = handle.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = cancel_handle.cancel_turn().await;
        }
    });

    // `AsyncBufReadExt::lines()` rejects non-UTF-8 bytes and aborts the REPL with
    // "stream did not contain valid UTF-8" (common with GBK terminals or pasted
    // binary). Read raw bytes and decode lossily instead.
    let mut stdin = BufReader::new(io::stdin());
    let mut line_buf = Vec::new();
    let mut sub = handle.subscribe();

    loop {
        tokio::select! {
            // Read the next line from stdin (byte-oriented; UTF-8 lossy).
            read = stdin.read_until(b'\n', &mut line_buf) => match read {
                Ok(0) => break,
                Ok(_) => {
                    while matches!(line_buf.last(), Some(b'\n' | b'\r')) {
                        line_buf.pop();
                    }
                    let l = String::from_utf8_lossy(&line_buf).into_owned();
                    line_buf.clear();

                    if l.trim() == "/quit" {
                        break;
                    }
                    if l.trim().is_empty() {
                        continue;
                    }
                    handle.send_user(l).await.context("send_user")?;
                }
                Err(e) => return Err(e).context("stdin read"),
            },
            // Forward real-time text deltas to stdout.
            evt = sub.recv() => match evt {
                Ok(StreamEvent::TextDelta { chunk }) => {
                    use std::io::Write as _;
                    print!("{chunk}");
                    let _ = std::io::stdout().flush();
                }
                Ok(_) => {}
                // Broadcast channel lagged or closed — treat as session end.
                Err(_) => break,
            },
        }
    }

    let _ = handle.shutdown(Duration::from_secs(30)).await;
    Ok(())
}

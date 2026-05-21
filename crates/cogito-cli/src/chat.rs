//! `cogito chat` — interactive REPL subcommand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_model::build_gateway;
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

/// Build the chat session's `ToolProvider`: registers builtins, brings
/// up MCP servers (per `cogito.toml`), prints the startup banner to
/// stderr, and composes the two via `CompositeToolProvider::Strict`
/// when MCP brought up at least one server.
///
/// Returns the builtin-only provider when no MCP servers were
/// configured or all of them failed (see ADR-0018 §3.5 — MCP failures
/// are non-fatal to Runtime).
async fn build_tool_provider(
    cfg: &cogito_config::RuntimeConfig,
) -> Result<Arc<dyn cogito_protocol::tool::ToolProvider>> {
    let builtin: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let mcp_build = cogito_mcp::build_mcp_provider(&cfg.mcp_servers).await;

    // Banner: merge parse-time + handshake-time failures.
    let all_failures: Vec<cogito_mcp::McpStartupFailure> = cfg
        .mcp_parse_failures
        .iter()
        .cloned()
        .chain(mcp_build.failures.iter().cloned())
        .collect();

    // Per-server tool counts: enumerate cfg.mcp_servers, count descriptors
    // whose qualified name starts with `mcp__<name>__`.
    let mut successful_tool_counts: Vec<(String, usize)> = Vec::new();
    if let Some(provider) = mcp_build.provider.as_ref() {
        let descriptors = provider.list();
        for cfg_entry in &cfg.mcp_servers {
            let prefix = format!("mcp__{}__", cfg_entry.name);
            let count = descriptors
                .iter()
                .filter(|d| d.name.starts_with(&prefix))
                .count();
            if count > 0 {
                successful_tool_counts.push((cfg_entry.name.clone(), count));
            }
        }
    }

    // Print the banner to stderr (ADR-0018 §3.5.3 — surfaces silent skips).
    let mut stderr = std::io::stderr();
    if let Err(e) = crate::banner::render_banner(
        &mut stderr,
        &cfg.mcp_servers,
        &all_failures,
        &successful_tool_counts,
    ) {
        tracing::warn!("failed to render mcp startup banner: {e}");
    }

    // Compose providers. When MCP brought up anything, layer it under
    // Strict (builtins must not start with `mcp__` per ADR-0018 §4;
    // debug_assert in BuiltinToolProviderBuilder enforces this).
    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = match mcp_build.provider {
        Some(mcp) => Arc::new(
            cogito_tools::CompositeToolProvider::new(
                vec![builtin, mcp],
                cogito_tools::NamingPolicy::Strict,
            )
            .map_err(|e| anyhow!("compose builtins + mcp: {e}"))?,
        ),
        None => builtin,
    };
    Ok(tools)
}

/// Entry point for the `chat` subcommand.
// `print!` / `println!` are intentional here: the chat REPL must write model
// output to stdout without tracing timestamps or log levels.
#[allow(clippy::print_stdout)]
pub async fn run(args: ChatArgs) -> Result<()> {
    let inputs = cogito_cli::chat_config::ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    };
    let cfg = cogito_cli::chat_config::load_layered_config(&inputs).await?;
    let provider_cfg = cogito_cli::chat_config::select_provider(&cfg, &inputs)?;
    let gateway: Arc<dyn ModelGateway> =
        build_gateway(provider_cfg).map_err(|e| anyhow!("building gateway: {e}"))?;

    let model_id = args
        .model
        .clone()
        .or_else(|| cfg.runtime.default_model.clone())
        .ok_or_else(|| anyhow!("--model required (or set runtime.default_model in cogito.toml)"))?;

    let store = Arc::new(JsonlStore::new(cfg.runtime.session_root.clone()));
    let tools = build_tool_provider(&cfg).await?;

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
                    handle.submit_user_text(l).await.context("submit_user_text")?;
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

//! `cogito chat` — interactive REPL subcommand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::render::Renderer;
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
use std::io::Write as _;

use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Window after a Ctrl-C that already initiated a turn cancellation
/// during which a second Ctrl-C is interpreted as "exit now" rather
/// than "cancel again". Matches the conventional REPL behaviour
/// (Python, `IPython`, node) where a double-tap escapes a hung cancel.
const CTRL_C_EXIT_WINDOW: Duration = Duration::from_secs(2);

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

    tracing::info!(
        %session_id,
        "cogito chat started (type /quit to exit, Ctrl-C to cancel turn / exit when idle)"
    );

    run_repl(handle.clone()).await?;
    let _ = handle.shutdown(Duration::from_secs(30)).await;
    Ok(())
}

/// REPL event loop. Owns stdin reading, signal handling, and stream
/// subscription; lives as its own function so `run()` stays under
/// `clippy::too_many_lines` and the loop can be tested in isolation.
async fn run_repl(handle: cogito_core::runtime::SessionHandle) -> Result<()> {
    // Ctrl-C handler — forwards each press as a message; the main loop
    // interprets it relative to current state (in-flight turn vs idle).
    // Bounded mpsc(8) so multiple rapid presses don't deadlock the
    // signal task.
    let (ctrl_c_tx, mut ctrl_c_rx) = mpsc::channel::<()>(8);
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            if ctrl_c_tx.send(()).await.is_err() {
                break;
            }
        }
    });

    // `AsyncBufReadExt::lines()` rejects non-UTF-8 bytes and aborts the REPL with
    // "stream did not contain valid UTF-8" (common with GBK terminals or pasted
    // binary). Read raw bytes and decode lossily instead.
    let mut stdin = BufReader::new(io::stdin());
    let mut line_buf = Vec::new();
    let mut sub = handle.subscribe();
    let mut renderer = Renderer::for_stdout();

    // Tracked across iterations so Ctrl-C can decide cancel-vs-exit.
    // `turn_in_flight` is set on `submit_user_text` and cleared on the
    // next terminal `StreamEvent`. `cancel_initiated_at` enables the
    // double-tap escape: a second Ctrl-C within CTRL_C_EXIT_WINDOW
    // exits even if the first cancel hasn't reported a terminal event
    // yet (e.g. a model gateway hanging on shutdown).
    let mut turn_in_flight = false;
    let mut cancel_initiated_at: Option<Instant> = None;

    renderer.prompt_user()?;

    loop {
        tokio::select! {
            ctrl_c = ctrl_c_rx.recv() => {
                if ctrl_c.is_none() {
                    // Signal task exited; rare but treat as session end.
                    break;
                }
                let within_window = cancel_initiated_at
                    .is_some_and(|t| t.elapsed() < CTRL_C_EXIT_WINDOW);
                if turn_in_flight && !within_window {
                    let _ = handle.cancel_turn().await;
                    cancel_initiated_at = Some(Instant::now());
                    let _ = writeln!(
                        std::io::stderr(),
                        "\n(Ctrl-C: cancelling turn — press again within 2s to exit)"
                    );
                } else {
                    let _ = writeln!(std::io::stderr(), "\n(Ctrl-C: exiting)");
                    break;
                }
            },
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
                        renderer.prompt_user()?;
                        continue;
                    }
                    handle.submit_user_text(l).await.context("submit_user_text")?;
                    turn_in_flight = true;
                }
                Err(e) => return Err(e).context("stdin read"),
            },
            evt = sub.recv() => match evt {
                Ok(e) => {
                    // TurnResumed is mid-turn (agent continues) — no fresh prompt.
                    let terminal = matches!(
                        &e,
                        StreamEvent::TurnCompleted
                            | StreamEvent::TurnFailed { .. }
                            | StreamEvent::TurnCancelled
                            | StreamEvent::TurnPaused
                    );
                    renderer.on_stream_event(&e)?;
                    if terminal {
                        turn_in_flight = false;
                        cancel_initiated_at = None;
                        renderer.prompt_user()?;
                    }
                }
                // Broadcast channel lagged or closed — treat as session end.
                Err(_) => break,
            },
        }
    }
    Ok(())
}

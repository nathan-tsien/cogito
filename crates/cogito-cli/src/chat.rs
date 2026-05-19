//! `cogito chat` — interactive REPL subcommand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_model::{AnthropicConfig, AnthropicGateway, OpenAiCompatConfig, OpenAiCompatGateway};
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
    /// Model identifier (e.g. `claude-opus-4-7`, `gpt-4o`).
    #[arg(long)]
    pub model: String,
    /// LLM provider. Inferred from model name if omitted: `claude-*` → anthropic, else openai-compat.
    #[arg(long, value_parser = ["anthropic", "openai-compat"])]
    pub provider: Option<String>,
    /// Base URL for the OpenAI-compatible endpoint (or `OPENAI_BASE_URL` env var).
    #[arg(long)]
    pub base_url: Option<String>,
    /// Directory where per-session JSONL files are stored.
    #[arg(long, default_value = "./sessions")]
    pub session_root: PathBuf,
    /// Resume an existing session by ULID. A new session is created if omitted.
    #[arg(long)]
    pub session_id: Option<String>,
    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,
}

/// Construct a [`ModelGateway`] from the CLI args. Pulls API keys / base URLs
/// from env vars (`ANTHROPIC_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_API_KEY`).
fn build_gateway(args: &ChatArgs) -> Result<Arc<dyn ModelGateway>> {
    let provider = args.provider.clone().unwrap_or_else(|| {
        if args.model.starts_with("claude-") {
            "anthropic".into()
        } else {
            "openai-compat".into()
        }
    });
    match provider.as_str() {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
            Ok(Arc::new(
                AnthropicGateway::new(AnthropicConfig::with_api_key(key))
                    .map_err(|e| anyhow!("anthropic gateway: {e}"))?,
            ))
        }
        "openai-compat" => {
            let base_url = args
                .base_url
                .clone()
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .context("--base-url or OPENAI_BASE_URL required for openai-compat")?;
            let mut cfg = OpenAiCompatConfig::with_base_url(base_url);
            cfg.api_key = std::env::var("OPENAI_API_KEY").ok();
            Ok(Arc::new(
                OpenAiCompatGateway::new(cfg).map_err(|e| anyhow!("openai-compat gateway: {e}"))?,
            ))
        }
        _ => anyhow::bail!("invalid provider: {provider}"),
    }
}

/// Entry point for the `chat` subcommand.
// `print!` / `println!` are intentional here: the chat REPL must write model
// output to stdout without tracing timestamps or log levels.
#[allow(clippy::print_stdout)]
pub async fn run(args: ChatArgs) -> Result<()> {
    let gateway = build_gateway(&args)?;

    // JsonlStore::new is synchronous and infallible.
    let store = Arc::new(JsonlStore::new(args.session_root.clone()));
    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let mut strategy = HarnessStrategy::default_with_model(&args.model);
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

//! `cogito chat` — interactive REPL subcommand.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::render::Renderer;
use anyhow::{Context, Result, anyhow};
use clap::{Args, ValueEnum};
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_model::build_gateway;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolResult;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt;
use std::io::Write as _;

use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// CLI value for `--mode`. Mirrors `OpenMode` but lives in the Surface
/// crate so clap can derive `ValueEnum` without touching the Brain
/// layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ChatMode {
    /// Session must not exist in the store. Default when `--session-id`
    /// is omitted.
    New,
    /// Session must exist; replay all prior events to the screen
    /// before opening the REPL prompt.
    Resume,
    /// Like `Resume` but tolerant of an empty store. Default when
    /// `--session-id` is supplied without an explicit `--mode`.
    Attach,
}

impl From<ChatMode> for OpenMode {
    fn from(m: ChatMode) -> Self {
        match m {
            ChatMode::New => OpenMode::New,
            ChatMode::Resume => OpenMode::Resume,
            ChatMode::Attach => OpenMode::Attach,
        }
    }
}

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

    /// Open mode: `new` (default without `--session-id`), `resume`, or
    /// `attach` (default with `--session-id`). `attach` is tolerant of
    /// an empty store; `resume` requires the session to exist.
    #[arg(long, value_enum)]
    pub mode: Option<ChatMode>,

    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,
}

/// Resolve the open mode from CLI flags. Rejects incoherent
/// combinations early — `--mode resume` / `--mode attach` without a
/// `--session-id` would otherwise silently try to resume a freshly
/// minted ULID and bottom out in an opaque
/// `ResumeFailed("no such session in store")` from the runtime.
fn resolve_mode(args: &ChatArgs) -> Result<ChatMode> {
    let has_id = args.session_id.is_some();
    match (args.mode, has_id) {
        (Some(ChatMode::New), true) => Err(anyhow!(
            "--mode new conflicts with --session-id; drop one or use --mode attach"
        )),
        (Some(ChatMode::Resume), false) => Err(anyhow!(
            "--mode resume requires --session-id (the session to resume). \
             Tip: when invoking via `make chat`, the Make variable is \
             `SESSION_ID=…` (underscore), not `SESSION-ID=…`."
        )),
        (Some(ChatMode::Attach), false) => Err(anyhow!(
            "--mode attach requires --session-id (the session to attach to). \
             Tip: when invoking via `make chat`, the Make variable is \
             `SESSION_ID=…` (underscore), not `SESSION-ID=…`."
        )),
        (Some(m), _) => Ok(m),
        (None, true) => Ok(ChatMode::Attach),
        (None, false) => Ok(ChatMode::New),
    }
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
    // Keep a typed handle for the post-`open_session` replay; the
    // Runtime builder takes its own `Arc<dyn ConversationStore>` clone.
    let store_for_replay: Arc<dyn ConversationStore> = store.clone();
    let tools = build_tool_provider(&cfg).await?;

    let mut strategy = HarnessStrategy::default_with_model(&model_id);
    if let Some(sys) = args.system.clone() {
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
    let session_id = match &args.session_id {
        Some(s) => s
            .parse::<SessionId>()
            .context("invalid session_id (need ULID)")?,
        None => SessionId::new(),
    };

    let mode = resolve_mode(&args)?;

    let handle = runtime
        .open_session(session_id, OpenMode::from(mode))
        .await
        .map_err(|e| anyhow!("open_session: {e:?}"))?;

    tracing::info!(
        %session_id, ?mode,
        "cogito chat started (type /quit to exit, Ctrl-C to cancel turn / exit when idle)"
    );

    run_repl(handle.clone(), store_for_replay, session_id, mode).await?;
    let _ = handle.shutdown(Duration::from_secs(30)).await;
    Ok(())
}

/// REPL event loop. Owns stdin reading, signal handling, and stream
/// subscription; lives as its own function so `run()` stays under
/// `clippy::too_many_lines` and the loop can be tested in isolation.
///
/// When `mode` is `Resume` or `Attach`, prior `ConversationEvent`s
/// for `session_id` are streamed through the renderer before the
/// first prompt — so a resumed user sees the history they expect to
/// continue from.
async fn run_repl(
    handle: cogito_core::runtime::SessionHandle,
    store: Arc<dyn ConversationStore>,
    session_id: SessionId,
    mode: ChatMode,
) -> Result<()> {
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

    if matches!(mode, ChatMode::Resume | ChatMode::Attach) {
        replay_history(store.as_ref(), session_id, &mut renderer).await?;
    }

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

/// Read every persisted `ConversationEvent` for `session_id` and feed
/// each user-visible payload to the renderer. Returns Ok(()) and skips
/// silently when the session has no events — that's the legitimate
/// `Attach`-to-fresh-session case.
///
/// Replay does **not** subscribe to the live stream; it strictly
/// re-renders history. The REPL loop opens its own broadcast
/// subscription afterwards, so a turn already in flight when the
/// session was reopened will surface live (its `StreamEvent`s
/// supersede the persisted ones once the actor resumes).
async fn replay_history(
    store: &dyn ConversationStore,
    session_id: SessionId,
    renderer: &mut Renderer<std::io::Stdout>,
) -> Result<()> {
    // Tool calls are persisted as two events (`ToolUseRecorded` then
    // `ToolResultRecorded`); the renderer prints them as a single
    // block, so we buffer the start until the matching end arrives.
    // Key: persisted `call_id`. Value: (start_ts_millis, tool_name, args).
    let mut tool_starts: HashMap<String, (i64, String, serde_json::Value)> = HashMap::new();
    let mut event_count = 0u64;

    // `replay(_, 0)` yields events with `seq > 0` — i.e. everything
    // after `SessionStarted` (seq 0). That suits us: we don't render
    // session meta during history replay.
    let mut stream = store.replay(session_id, 0);
    while let Some(ev) = stream.next().await {
        let event = ev.context("replay event")?;
        event_count += 1;
        match event.payload {
            EventPayload::TurnStarted { user_input } => {
                if let Some(text) = first_user_text(&user_input) {
                    renderer.replay_user_input(text)?;
                }
            }
            EventPayload::AssistantMessageAppended { text } => {
                renderer.replay_assistant_block(&text)?;
            }
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
            } => {
                tool_starts.insert(call_id, (event.ts.timestamp_millis(), tool_name, args));
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                if let Some((start_ms, tool_name, args)) = tool_starts.remove(&call_id) {
                    // Wall-clock delta; `max(0)` guards against
                    // (unlikely) backwards-going timestamps.
                    let elapsed_ms =
                        u128::try_from((event.ts.timestamp_millis() - start_ms).max(0))
                            .unwrap_or(0);
                    let (ok, err_msg): (bool, Option<&str>) = match &result {
                        ToolResult::Output(_) => (true, None),
                        ToolResult::Error { message, .. } => (false, Some(message.as_str())),
                        // `ToolResult` is `#[non_exhaustive]`; treat
                        // unknown future variants as opaque failures.
                        _ => (false, None),
                    };
                    renderer.replay_tool_call(&tool_name, &args, elapsed_ms, ok, err_msg)?;
                }
            }
            EventPayload::TurnFailed { reason } => {
                renderer.replay_turn_failed(&format_turn_failure(&reason))?;
            }
            // Skip internal lifecycle events that don't have a
            // user-facing rendering in the live path either:
            // ContextManage*, PromptComposed, ModelCall*,
            // TurnCompleted, TurnPaused, JobCompletedRecorded,
            // SessionStarted (already filtered by `from_seq = 0`).
            _ => {}
        }
    }

    if event_count > 0 {
        renderer.replay_banner(&format!(
            "--- end of replay ({event_count} events, session {session_id}) ---"
        ))?;
    }
    Ok(())
}

/// First `Text` block from a user-input content vec, or `None` if the
/// turn was kicked off by non-text content (e.g. an image in a future
/// multimodal turn). Replay shows text only — non-text inputs would
/// need a richer renderer.
fn first_user_text(blocks: &[ContentBlock]) -> Option<&str> {
    blocks.iter().find_map(|b| match b {
        ContentBlock::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

/// Render a persisted `TurnFailureReason` as a one-liner suitable for
/// the `[error] …` slot. Mirrors the live-stream `TurnFailed.reason`
/// string field that the actor builds at failure time.
fn format_turn_failure(reason: &cogito_protocol::turn::TurnFailureReason) -> String {
    use cogito_protocol::turn::TurnFailureReason as R;
    match reason {
        R::StoreUnavailable => "store unavailable".into(),
        R::ModelGatewayFailed { message } => format!("model gateway: {message}"),
        R::TurnPanicked { location } => format!("turn panicked at {location}"),
        R::TurnTimedOut => "turn timed out".into(),
        R::HookRejected { hook_name, message } => {
            format!("hook {hook_name} rejected: {message}")
        }
        R::ResumeFailed { message } => format!("resume failed: {message}"),
        // `TurnFailureReason` is `#[non_exhaustive]`; fall back to
        // Debug for any variant added in a future schema bump.
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args_with(mode: Option<ChatMode>, session_id: Option<&str>) -> ChatArgs {
        ChatArgs {
            config: None,
            model: None,
            provider: None,
            base_url: None,
            session_root: None,
            session_id: session_id.map(str::to_owned),
            mode,
            system: None,
        }
    }

    #[test]
    fn resolve_mode_defaults_to_new_without_session_id() {
        let m = resolve_mode(&args_with(None, None)).unwrap();
        assert_eq!(m, ChatMode::New);
    }

    #[test]
    fn resolve_mode_defaults_to_attach_when_session_id_present() {
        let m = resolve_mode(&args_with(None, Some("01H0000000000000000000000A"))).unwrap();
        assert_eq!(m, ChatMode::Attach);
    }

    #[test]
    fn resolve_mode_explicit_resume_wins() {
        let m = resolve_mode(&args_with(
            Some(ChatMode::Resume),
            Some("01H0000000000000000000000A"),
        ))
        .unwrap();
        assert_eq!(m, ChatMode::Resume);
    }

    #[test]
    fn resolve_mode_explicit_new_without_id_works() {
        let m = resolve_mode(&args_with(Some(ChatMode::New), None)).unwrap();
        assert_eq!(m, ChatMode::New);
    }

    #[test]
    fn resolve_mode_new_with_session_id_is_an_error() {
        let err = resolve_mode(&args_with(
            Some(ChatMode::New),
            Some("01H0000000000000000000000A"),
        ))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("--mode new conflicts with --session-id"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn resolve_mode_resume_without_session_id_is_an_error() {
        let err = resolve_mode(&args_with(Some(ChatMode::Resume), None)).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("--mode resume requires --session-id"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("SESSION_ID") && msg.contains("SESSION-ID"),
            "error should hint about the SESSION_ID vs SESSION-ID Make-variable pitfall: {msg}"
        );
    }

    #[test]
    fn resolve_mode_attach_without_session_id_is_an_error() {
        let err = resolve_mode(&args_with(Some(ChatMode::Attach), None)).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("--mode attach requires --session-id"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn first_user_text_returns_first_text_block() {
        let blocks = vec![ContentBlock::Text {
            text: "hello".into(),
        }];
        assert_eq!(first_user_text(&blocks), Some("hello"));
    }

    #[test]
    fn first_user_text_returns_none_when_no_text_block() {
        let blocks: Vec<ContentBlock> = vec![];
        assert_eq!(first_user_text(&blocks), None);
    }

    #[test]
    fn format_turn_failure_renders_model_gateway() {
        use cogito_protocol::turn::TurnFailureReason;
        let s = format_turn_failure(&TurnFailureReason::ModelGatewayFailed {
            message: "boom".into(),
        });
        assert_eq!(s, "model gateway: boom");
    }
}

//! Build a Runtime + open a Session for the TUI. Mirrors
//! `cogito-cli::chat::run`'s prelude up to (but not including) the
//! event loop; the TUI's loop lives in `event_loop::run`.
//!
//! The TUI's runtime build is conceptually 1:1 with the CLI's — only
//! the consumer (App vs Renderer + REPL) differs. To keep that
//! coupling explicit we share `cogito-cli`'s `chat_config::*` and
//! `chat::resolve_strategy` helpers via the library promotion done in
//! Phase 1.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use cogito_cli::chat::{ChatArgs, ChatMode, ResolveError, resolve_strategy};
use cogito_cli::chat_config::{
    ChatConfigInputs, build_runtime_config_and_registry, build_skill_provider, patch_base_url,
};
use cogito_core::runtime::{OpenMode, Runtime, SessionHandle};
use cogito_jobs::{LocalJobManager, RunTestsTool};
use cogito_protocol::ConversationStore;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::JobManager;
use cogito_protocol::strategy_registry::StrategyRegistry;
use cogito_protocol::tool::ToolProvider;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, CompositeToolProvider, NamingPolicy, ReadFile};

use crate::app::App;
use crate::cli::{TuiArgs, TuiMode};
use crate::render_model::{ChatModel, ToolTreeModel};
use crate::resume::{InitialState, load_initial_state};
use crate::ui::input::InputWidget;

/// Output of the build: an App ready to enter the event loop and
/// the captured MCP banner lines to prepend to chat.
pub struct Built {
    /// Ready-to-run App.
    pub app: App,
    /// MCP banner lines (textual) to push as `SystemNotices`. The TUI
    /// has already pushed these into `app.chat`; the field is retained
    /// so the integration tests (Task 20) can assert on the banner text
    /// without inspecting `ChatModel` internals.
    pub mcp_banner: Vec<String>,
}

/// Build Runtime + open Session + assemble App. Errors here propagate
/// to `main`, which prints them and exits non-zero WITHOUT entering
/// raw mode.
///
/// # Errors
///
/// Returns `anyhow::Error` if config load, strategy resolution,
/// provider selection, gateway construction, runtime build, session
/// open, or initial-state load fails.
pub async fn build(args: &TuiArgs) -> Result<Built> {
    let inputs = inputs_from_args(args);
    let (cfg, registry) = build_runtime_config_and_registry(&inputs)
        .await
        .context("loading config + strategies")?;

    let cli_args = args_to_cli_chatargs(args);
    let (strategy, provider_cfg) =
        resolve_strategy(&cli_args, &cfg, registry.as_ref()).map_err(|e| match e {
            ResolveError::UnknownStrategy { name, available } => anyhow!(
                "unknown strategy `{name}`; available: {avail}",
                avail = available.join(", ")
            ),
            other => anyhow!(other),
        })?;

    // CLI `--base-url` is a post-merge field patch; apply it on the
    // provider chosen by `resolve_strategy` so the Sprint 2 flag
    // continues to override base_url for any provider kind.
    let provider_cfg = if let Some(b) = args.base_url.as_ref() {
        patch_base_url(provider_cfg, b.clone())
    } else {
        provider_cfg
    };

    let gateway =
        cogito_model::build_gateway(provider_cfg).map_err(|e| anyhow!("building gateway: {e}"))?;

    let store = Arc::new(JsonlStore::new(cfg.runtime.session_root.clone()));
    let store_for_app: Arc<dyn ConversationStore> = store.clone();

    // Construct the `LocalJobManager` singleton up here so the SAME
    // `Arc` flows into both `RunTestsTool` (typed for `submit`) and
    // `RuntimeBuilder::job_manager` (typed for `on_complete`). See
    // ADR-0008 and ADR-0025.
    let job_mgr: Arc<LocalJobManager> = LocalJobManager::new();

    let (tool_provider, mcp_banner_lines) =
        build_tools_with_banner(&cfg, Arc::clone(&job_mgr)).await?;

    let skills = build_skill_provider(&cfg)?;

    // `Arc<LocalJobManager>` -> `Arc<dyn JobManager>` via the unsized
    // coercion impl on `Arc<T>`.
    let job_mgr_dyn: Arc<dyn JobManager> = job_mgr;
    let mut builder = Runtime::builder()
        .store(store.clone())
        .model(gateway)
        .tools(tool_provider)
        .strategy(strategy.clone())
        .job_manager(job_mgr_dyn);
    if let Some(provider) = skills {
        builder = builder.skills(provider);
    }
    let runtime = builder.build().context("building runtime")?;

    // Parse or generate the session ID.
    let session_id: SessionId = match &args.session_id {
        Some(s) => s.parse().context("invalid session_id (need ULID)")?,
        None => SessionId::new(),
    };

    let mode = resolve_open_mode(args)?;
    let handle: SessionHandle = runtime
        .open_session(session_id, mode)
        .await
        .map_err(|e| anyhow!("open_session: {e:?}"))?;

    let is_new = matches!(mode, OpenMode::New);
    let initial = load_initial_state(&store_for_app, &session_id, is_new).await?;
    let (chat, tools, turn_count, turn_in_progress) =
        replay_into_models(&mcp_banner_lines, initial);

    let registry_dyn: Arc<dyn StrategyRegistry> = registry;
    let app = App {
        handle,
        registry: registry_dyn,
        store: store_for_app,
        session_id_str: session_id.to_string(),
        session_root: Some(cfg.runtime.session_root.clone()),
        chat,
        tools,
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        show_tools: true,
        popup: None,
        strategy_name: args
            .strategy
            .clone()
            .or_else(|| cfg.runtime.default_strategy.clone())
            .unwrap_or_else(|| "<synthesized>".into()),
        model_id: strategy.model_params.model.clone(),
        turn_count,
        turn_in_progress,
        cancel_seen_at: None,
        should_quit: false,
    };

    Ok(Built {
        app,
        mcp_banner: mcp_banner_lines,
    })
}

/// Seed empty `ChatModel` + `ToolTreeModel` with (a) the MCP banner
/// lines as system notices and (b) the replayed stream events from the
/// session log. Returns the populated models plus the lifecycle
/// counters derived from the replayed turns.
fn replay_into_models(
    banner_lines: &[String],
    initial: InitialState,
) -> (ChatModel, ToolTreeModel, u32, bool) {
    let mut chat = ChatModel::new();
    for line in banner_lines {
        chat.push_notice(line.clone());
    }
    let mut tools = ToolTreeModel::new();
    let mut turn_count: u32 = 0;
    let mut turn_in_progress = false;
    if let InitialState::Replayed { stream_events } = initial {
        for ev in &stream_events {
            chat.on_event(ev);
            tools.on_event(ev);
            match ev {
                cogito_protocol::stream::StreamEvent::TurnStarted => turn_in_progress = true,
                cogito_protocol::stream::StreamEvent::TurnCompleted => {
                    turn_in_progress = false;
                    turn_count = turn_count.saturating_add(1);
                }
                cogito_protocol::stream::StreamEvent::TurnFailed { .. }
                | cogito_protocol::stream::StreamEvent::TurnCancelled
                | cogito_protocol::stream::StreamEvent::TurnPaused => turn_in_progress = false,
                _ => {}
            }
        }
    }
    (chat, tools, turn_count, turn_in_progress)
}

fn inputs_from_args(args: &TuiArgs) -> ChatConfigInputs {
    ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    }
}

fn args_to_cli_chatargs(args: &TuiArgs) -> ChatArgs {
    ChatArgs {
        config: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
        session_id: args.session_id.clone(),
        mode: args.mode.map(|m| match m {
            TuiMode::New => ChatMode::New,
            TuiMode::Resume => ChatMode::Resume,
            TuiMode::Attach => ChatMode::Attach,
        }),
        system: args.system.clone(),
        strategy: args.strategy.clone(),
        list_strategies: args.list_strategies,
    }
}

/// Resolve the TUI's session open mode. The defaults mirror the CLI's
/// `resolve_mode`: `--mode` wins if present, otherwise `Attach` if a
/// session id is given and `New` otherwise. Incoherent combinations
/// (e.g. `--mode resume` without `--session-id`) are rejected with
/// the same human-facing message as the CLI.
fn resolve_open_mode(args: &TuiArgs) -> Result<OpenMode> {
    let has_id = args.session_id.is_some();
    match (args.mode, has_id) {
        (Some(TuiMode::New), true) => Err(anyhow!(
            "--mode new conflicts with --session-id; drop one or use --mode attach"
        )),
        (Some(TuiMode::Resume), false) => Err(anyhow!(
            "--mode resume requires --session-id (the session to resume)"
        )),
        (Some(TuiMode::Attach), false) => Err(anyhow!(
            "--mode attach requires --session-id (the session to attach to)"
        )),
        (Some(TuiMode::Resume), true) => Ok(OpenMode::Resume),
        (Some(TuiMode::Attach) | None, true) => Ok(OpenMode::Attach),
        (Some(TuiMode::New) | None, false) => Ok(OpenMode::New),
    }
}

/// Compose builtin tools + the async `run_tests` job tool + MCP tools.
/// Mirrors `cogito-cli::chat::build_tool_provider` but captures the
/// banner into a `Vec<String>` instead of printing to stderr — the TUI
/// surfaces the banner inside the chat scrollback rather than racing
/// against raw-mode setup on stderr.
async fn build_tools_with_banner(
    cfg: &cogito_config::RuntimeConfig,
    job_mgr: Arc<LocalJobManager>,
) -> Result<(Arc<dyn ToolProvider>, Vec<String>)> {
    let builtin: Arc<dyn ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    let run_tests: Arc<dyn ToolProvider> = Arc::new(RunTestsTool::new(job_mgr));
    let local: Arc<dyn ToolProvider> = Arc::new(
        CompositeToolProvider::new(vec![builtin, run_tests], NamingPolicy::Strict)
            .map_err(|e| anyhow!("compose builtin + run_tests: {e}"))?,
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
    // whose qualified name starts with `mcp__<name>__`. Mirrors the CLI.
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

    let mut banner_buf: Vec<u8> = Vec::new();
    cogito_cli::banner::render_banner(
        &mut banner_buf,
        &cfg.mcp_servers,
        &all_failures,
        &successful_tool_counts,
    )
    .context("rendering MCP banner")?;
    let banner_text = String::from_utf8_lossy(&banner_buf).to_string();
    let banner_lines: Vec<String> = banner_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    let tools: Arc<dyn ToolProvider> = match mcp_build.provider {
        Some(mcp) => Arc::new(
            CompositeToolProvider::new(vec![local, mcp], NamingPolicy::Strict)
                .map_err(|e| anyhow!("compose local + mcp: {e}"))?,
        ),
        None => local,
    };
    Ok((tools, banner_lines))
}

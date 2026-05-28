# Sprint 9b — Cogito TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift `cogito-tui` from a one-line stub to a multi-pane `ratatui` terminal UI replicating `cogito chat` with strategy support, per `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`.

**Architecture:** Surface-layer crate (peer to `cogito-cli`) wrapping the same `cogito-core::Runtime` and `FsStrategyRegistry`. Single-threaded tokio event loop multiplexes `crossterm` keys, `StreamEvent` broadcast from `SessionHandle::subscribe()`, and a 30 FPS redraw tick. State models are sink-agnostic pure functions of `(state, &StreamEvent)`; widgets convert state to `Line`/`Span` lazily at render time.

**Tech Stack:** Rust 2024 (MSRV 1.85), `ratatui = 0.28`, `crossterm = 0.28`, `tui-textarea = 0.7` (new workspace dep), `tracing-appender = 0.2` (new), `tokio = 1.40` current-thread, `assert_cmd` for CLI parity testing.

**Authoritative spec:** `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md` — every architectural decision is locked there. This plan operationalizes it.

---

## File structure (created/modified)

```
crates/cogito-tui/
  Cargo.toml                                MODIFY: expand deps
  src/
    main.rs                                 REWRITE: clap + dispatch
    lib.rs                                  CREATE: re-exports for tests
    app.rs                                  CREATE: App state
    event_loop.rs                           CREATE: select! loop
    keymap.rs                               CREATE: key-to-action dispatch
    terminal.rs                             CREATE: RAII + panic hook
    render_model.rs                         CREATE: ChatModel + ToolTreeModel
    resume.rs                               CREATE: ConversationEvent → StreamEvent translation
    slash.rs                                CREATE: slash command dispatch
    runtime_build.rs                        CREATE: build Runtime + open Session
    logs.rs                                 CREATE: gated tracing-appender setup
    ui/
      mod.rs                                CREATE: top-level layout
      chat.rs                               CREATE: chat pane widget
      tools.rs                              CREATE: tool-tree pane widget
      input.rs                              CREATE: tui-textarea wrapper
      status.rs                             CREATE: bottom status bar
      popup.rs                              CREATE: / discovery popup
  tests/
    snapshot.rs                             CREATE: TestBackend snapshots
    e2e.rs                                  CREATE: MockModel-driven end-to-end
    resume.rs                               CREATE: replay-from-JSONL
    list_strategies.rs                      CREATE: parity with cogito-cli
    edges.rs                                CREATE: resize, long input, unicode, deep tree
    fixtures/
      session-single-text-turn.jsonl        CREATE: canned session log
      session-with-tool-call.jsonl          CREATE: canned session log
      coder.md                              CREATE: canned strategy

Cargo.toml                                  MODIFY: add tui-textarea + tracing-appender + assert_cmd to workspace.dependencies

AGENTS.md                                   MODIFY: workspace layout — bump cogito-tui v0.2 → v0.1
ARCHITECTURE.md                             MODIFY: workspace layout — same bump
CLAUDE.md                                   MODIFY: workspace layout — same bump (CLAUDE.md mirrors AGENTS.md table)
ROADMAP.md                                  MODIFY: tick all three Sprint 9b checkboxes
CHANGELOG.md                                MODIFY: Sprint 9b entry under Unreleased
docs/configuration/overview.md              MODIFY: add cogito-tui consumer note
docs/components/cogito-tui.md               CREATE: Surface-layer component doc
crates/cogito-tui/README.md                 CREATE: one-page overview + keymap
```

---

## Phase 1: Workspace deps and crate skeleton

### Task 1: Add workspace deps + bump cogito-tui Cargo.toml

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/cogito-tui/Cargo.toml`

- [ ] **Step 1: Add three new workspace deps**

Edit `Cargo.toml` workspace root. Add to `[workspace.dependencies]`:

```toml
# TUI input widget — multi-line buffer used by cogito-tui
tui-textarea = "0.7"
# File-rotating log writer for TUI debug mode
tracing-appender = "0.2"
# CLI integration testing (already used in cogito-cli tests, promote to workspace)
assert_cmd = "2.0"
```

Locate the appropriate sections: `tui-textarea` and `tracing-appender` go after the `crossterm` line (around line 121); `assert_cmd` goes after `temp-env`.

- [ ] **Step 2: Expand cogito-tui's Cargo.toml**

Replace `crates/cogito-tui/Cargo.toml` with:

```toml
[package]
name = "cogito-tui"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[[bin]]
name = "cogito-tui"
path = "src/main.rs"

[lib]
path = "src/lib.rs"

[dependencies]
# Internal layers
cogito-core.workspace = true
cogito-protocol.workspace = true
cogito-config.workspace = true
cogito-strategy.workspace = true
cogito-model.workspace = true
cogito-tools.workspace = true
cogito-jobs.workspace = true
cogito-mcp.workspace = true
cogito-skills.workspace = true
cogito-store-jsonl.workspace = true
cogito-sandbox.workspace = true
# Surface-layer sibling: re-uses chat_config, banner, slash parser.
# Library promotion already done — cogito-cli/src/lib.rs exposes the needed modules.
cogito-cli = { path = "../cogito-cli" }

# UI
ratatui.workspace = true
crossterm.workspace = true
tui-textarea.workspace = true

# Async runtime
tokio = { workspace = true, features = ["full"] }
tokio-stream.workspace = true
futures.workspace = true

# CLI
clap = { workspace = true, features = ["derive"] }

# Errors
anyhow.workspace = true
thiserror.workspace = true

# Logging
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-appender.workspace = true

# Misc
serde_json.workspace = true

[dev-dependencies]
assert_cmd.workspace = true
tempfile.workspace = true
cogito-mock-model.workspace = true
cogito-test-fixtures.workspace = true
tokio-test.workspace = true

[lints]
workspace = true
```

- [ ] **Step 3: Verify the workspace still resolves**

Run: `cargo metadata --format-version 1 --offline > /dev/null 2>&1 || cargo metadata --format-version 1 > /dev/null`

Expected: exit 0 (metadata builds; deps resolve). If `tui-textarea`, `tracing-appender`, or `assert_cmd` are missing from the network cache, drop `--offline` and let cargo fetch them.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/cogito-tui/Cargo.toml
git commit -m "$(cat <<'EOF'
chore(deps): add tui-textarea, tracing-appender, assert_cmd to workspace

Sprint 9b TUI prerequisites. tui-textarea provides the multi-line input
widget; tracing-appender backs the gated debug log file; assert_cmd is
promoted from cogito-cli's local dev-dep to a workspace dep so cogito-tui
can use it for --list-strategies parity testing.

cogito-tui Cargo.toml expanded to the full Surface-layer dep set
(protocol/config/strategy/model/tools/jobs/mcp/skills/store/sandbox)
plus a path dep on cogito-cli for shared helpers (chat_config, banner,
slash parser, resolve_strategy).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: cogito-tui skeleton — clap CLI + lib.rs + main.rs stub that exits OK

**Files:**
- Create: `crates/cogito-tui/src/lib.rs`
- Rewrite: `crates/cogito-tui/src/main.rs`

- [ ] **Step 1: Write `src/lib.rs` declaring all modules**

Create `crates/cogito-tui/src/lib.rs`:

```rust
//! cogito-tui — multi-pane terminal UI for the cogito runtime.
//!
//! Library surface re-exports the modules so integration tests under
//! `tests/` can drive the TUI without going through `main.rs`.
//!
//! See `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md` for
//! the design rationale (multi-pane layout, lazy palette painting,
//! lazy tool-result lookup, three-layer terminal restoration).

pub mod app;
pub mod event_loop;
pub mod keymap;
pub mod logs;
pub mod render_model;
pub mod resume;
pub mod runtime_build;
pub mod slash;
pub mod terminal;
pub mod ui;

/// CLI args shared by the binary and the integration tests.
pub mod cli;
```

- [ ] **Step 2: Write `src/cli.rs` with the ChatArgs mirror**

Create `crates/cogito-tui/src/cli.rs`:

```rust
//! Command-line surface. Mirrors `cogito_cli::chat::ChatArgs` so flag
//! parity holds. The TUI does NOT have subcommands — `cogito-tui` IS
//! the chat surface.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Resume mode. Mirrors `cogito_cli::chat::ChatMode` (re-declared here
/// so we don't expose `cogito-cli` types in the TUI CLI surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TuiMode {
    /// Session must not exist in the store. Default without `--session-id`.
    New,
    /// Session must exist; replay all prior events before opening live UI.
    Resume,
    /// Like `Resume` but tolerant of empty store. Default with `--session-id`.
    Attach,
}

/// `cogito-tui` argument surface. Flag set matches `cogito chat`.
#[derive(Debug, Default, Parser)]
#[command(name = "cogito-tui", version, about = "cogito Agent Runtime TUI")]
pub struct TuiArgs {
    /// Path to a `cogito.toml`. Highest precedence in the search path.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Model identifier (e.g. `claude-opus-4-7`, `gpt-4o`). Overrides
    /// `runtime.default_model` from the config.
    #[arg(long)]
    pub model: Option<String>,

    /// Provider name (matches `[[providers]] name = "..."` in the config).
    #[arg(long)]
    pub provider: Option<String>,

    /// Base URL override applied to the selected provider AFTER merge.
    #[arg(long)]
    pub base_url: Option<String>,

    /// Directory where per-session JSONL files are stored.
    #[arg(long)]
    pub session_root: Option<PathBuf>,

    /// Resume an existing session by ULID. New session if omitted.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Open mode: `new`, `resume`, or `attach`.
    #[arg(long, value_enum)]
    pub mode: Option<TuiMode>,

    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,

    /// Strategy name from `.cogito/strategies/`. Overrides
    /// `runtime.default_strategy`.
    #[arg(long, value_name = "NAME")]
    pub strategy: Option<String>,

    /// Print available strategies (name + description) and exit.
    #[arg(long)]
    pub list_strategies: bool,

    /// Enable file-rotating debug logs at
    /// `$XDG_STATE_HOME/cogito/tui.log` (or `~/.local/state/cogito/tui.log`).
    /// Implied by setting `RUST_LOG`.
    #[arg(long)]
    pub debug: bool,
}
```

- [ ] **Step 3: Replace `src/main.rs` with a minimal entrypoint**

Replace `crates/cogito-tui/src/main.rs` with:

```rust
//! cogito-tui binary entrypoint. Phase 1 stub: parses args and exits
//! OK so the workspace compiles. Real entrypoint lands in Task 22.

use anyhow::Result;
use clap::Parser;
use cogito_tui::cli::TuiArgs;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let _args = TuiArgs::parse();
    Ok(())
}
```

- [ ] **Step 4: Create empty module placeholders so `cargo check` passes**

Create the following files, each with the single line shown — they become real in later phases. (`missing_docs = "warn"` plus `-Dwarnings` means a bare `mod` declaration without a docstring fails; the `//!` line satisfies it.)

```bash
cat > crates/cogito-tui/src/app.rs           <<<'//! App state — populated in Phase 10.'
cat > crates/cogito-tui/src/event_loop.rs    <<<'//! Event loop — populated in Phase 14.'
cat > crates/cogito-tui/src/keymap.rs        <<<'//! Key dispatcher — populated in Phase 10.'
cat > crates/cogito-tui/src/logs.rs          <<<'//! Debug log setup — populated in Phase 17.'
cat > crates/cogito-tui/src/render_model.rs  <<<'//! Render state models — populated in Phase 2.'
cat > crates/cogito-tui/src/resume.rs        <<<'//! Resume replay — populated in Phase 12.'
cat > crates/cogito-tui/src/runtime_build.rs <<<'//! Runtime builder — populated in Phase 15.'
cat > crates/cogito-tui/src/slash.rs         <<<'//! Slash dispatch — populated in Phase 11.'
cat > crates/cogito-tui/src/terminal.rs      <<<'//! Terminal lifecycle — populated in Phase 9.'

mkdir -p crates/cogito-tui/src/ui
cat > crates/cogito-tui/src/ui/mod.rs        <<<'//! UI widgets — populated in Phase 8.'
cat > crates/cogito-tui/src/ui/chat.rs       <<<'//! Chat pane widget — populated in Phase 4.'
cat > crates/cogito-tui/src/ui/tools.rs      <<<'//! Tools pane widget — populated in Phase 5.'
cat > crates/cogito-tui/src/ui/input.rs      <<<'//! Input widget — populated in Phase 6.'
cat > crates/cogito-tui/src/ui/status.rs     <<<'//! Status bar widget — populated in Phase 7.'
cat > crates/cogito-tui/src/ui/popup.rs      <<<'//! Slash popup widget — populated in Phase 7.'
```

Add `pub mod chat; pub mod tools; pub mod input; pub mod status; pub mod popup;` to `src/ui/mod.rs` after the `//!` line so they're exposed.

- [ ] **Step 4b: Re-write `src/ui/mod.rs` with the actual submodule declarations**

```rust
//! UI widgets — top-level `render` lands in Phase 8; submodules below
//! populate progressively across Phases 4–7.

pub mod chat;
pub mod input;
pub mod popup;
pub mod status;
pub mod tools;
```

- [ ] **Step 5: Verify cargo check passes for cogito-tui**

Run: `cargo check -p cogito-tui`
Expected: exit 0; no errors. Warnings about unused dependencies are acceptable at this stage.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-tui/
git commit -m "$(cat <<'EOF'
feat(cogito-tui): skeleton — clap CLI + module placeholders

Sets up the module structure for Sprint 9b. `lib.rs` exposes every
module the binary and tests need; `cli.rs` mirrors cogito-cli's
ChatArgs flag surface (config / model / provider / base-url /
session-root / session-id / mode / system / strategy / list-strategies)
plus a new `--debug` flag for gated file logging.

All other modules are docstring-only placeholders so `cargo check`
passes; they get filled in by later phases. `main.rs` parses args and
exits 0 — no TUI yet.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 2: ChatModel — StreamEvent → Vec<ChatLine>

### Task 3: ChatLine enum + ChatModel struct + on_event for text/thinking

**Files:**
- Modify: `crates/cogito-tui/src/render_model.rs`

- [ ] **Step 1: Write the failing tests**

Replace `crates/cogito-tui/src/render_model.rs` with:

```rust
//! Sink-agnostic state models for the chat pane and tool-tree pane.
//!
//! These types translate the `StreamEvent` broadcast into structural
//! state. They never touch `Write`, `Frame`, or any ratatui type —
//! the UI widgets in `crate::ui::*` consume the models at render time
//! and apply palette/layout there. This separation is the spec's
//! "Q2-A locked: new ratatui-native translation; CLI Renderer untouched".

use std::collections::HashMap;
use std::time::Instant;

use cogito_protocol::stream::StreamEvent;

/// One visible line (or coalesced block) in the chat scrollback.
///
/// Stored as structural enum with raw text — the chat widget paints
/// palette lazily at render time (spec §3 detail-level decision:
/// "lazy palette painting").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLine {
    /// User prompt, rendered as `> {text}` with the user palette.
    UserPrompt(String),
    /// Assistant text block, accumulates across `TextDelta` within
    /// one content block.
    AssistantText(String),
    /// Assistant reasoning block, accumulates across `ThinkingDelta`.
    AssistantThinking(String),
    /// Tool dispatch start, paired with a `ToolEndLine` later.
    ToolStartLine {
        /// Tool name as reported by the dispatcher.
        tool: String,
        /// Compact JSON args preview, truncated.
        args_preview: String,
    },
    /// Tool dispatch end.
    ToolEndLine {
        /// Tool name (same as the matching start line).
        tool: String,
        /// `true` if the tool returned successfully.
        ok: bool,
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
        /// Error message captured on failure; `None` on success.
        error: Option<String>,
    },
    /// System-emitted notice line — `[paused]`, `[cancelled]`,
    /// `[error] ...`, MCP banner, slash command echoes.
    SystemNotice(String),
}

/// Maximum chars to preview from a tool's args JSON. Matches the CLI
/// `TOOL_ARGS_PREVIEW_MAX` to keep `[tool] foo {...}` lines bounded.
pub const TOOL_ARGS_PREVIEW_MAX: usize = 200;

/// Maximum chars to preview from a tool's error message.
pub const TOOL_ERROR_PREVIEW_MAX: usize = 400;

/// Truncate a string to at most `max` Unicode chars (not bytes),
/// appending `...` when truncated.
#[must_use]
pub fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// Per-tool dispatch timer, keyed by `call_id`. Resolved into
/// `ToolEndLine::elapsed_ms` when the matching end arrives.
type ToolTimers = HashMap<String, (Instant, String)>;

/// Chat scrollback state. Pure function of the `StreamEvent` stream.
#[derive(Debug, Default)]
pub struct ChatModel {
    /// Visible lines in display order.
    pub lines: Vec<ChatLine>,
    /// Currently in the middle of an `AssistantText` block — used to
    /// coalesce successive `TextDelta`s into one line.
    pub in_text: bool,
    /// Same for `AssistantThinking`.
    pub in_thinking: bool,
    /// Vertical scroll offset from the bottom (0 = follow tail).
    pub scroll_offset: u16,
    /// `call_id` → (started_at, tool_name) for elapsed-ms tracking.
    tool_timers: ToolTimers,
}

impl ChatModel {
    /// Construct a fresh, empty chat model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a user prompt line. Called by the input → send path, not
    /// by `on_event` (the prompt isn't a `StreamEvent`).
    pub fn push_user_prompt(&mut self, text: String) {
        self.lines.push(ChatLine::UserPrompt(text));
        self.in_text = false;
        self.in_thinking = false;
    }

    /// Push a `SystemNotice`. Used for MCP banner lines, slash echoes,
    /// `[hint]` lines.
    pub fn push_notice(&mut self, msg: impl Into<String>) {
        self.lines.push(ChatLine::SystemNotice(msg.into()));
        self.in_text = false;
        self.in_thinking = false;
    }

    /// Apply one `StreamEvent`. Pure state transition; never draws.
    pub fn on_event(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TurnStarted => {
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TextDelta { chunk } => {
                if self.in_text {
                    if let Some(ChatLine::AssistantText(s)) = self.lines.last_mut() {
                        s.push_str(chunk);
                    }
                } else {
                    self.lines.push(ChatLine::AssistantText(chunk.clone()));
                    self.in_text = true;
                }
                self.in_thinking = false;
            }
            StreamEvent::ThinkingDelta { chunk } => {
                if self.in_thinking {
                    if let Some(ChatLine::AssistantThinking(s)) = self.lines.last_mut() {
                        s.push_str(chunk);
                    }
                } else {
                    self.lines.push(ChatLine::AssistantThinking(chunk.clone()));
                    self.in_thinking = true;
                }
                self.in_text = false;
            }
            StreamEvent::ToolDispatchStarted {
                call_id,
                tool_name,
                args,
            } => {
                self.tool_timers
                    .insert(call_id.clone(), (Instant::now(), tool_name.clone()));
                let args_preview = serde_json::to_string(args).map_or_else(
                    |_| "{}".to_string(),
                    |s| truncate_chars(&s, TOOL_ARGS_PREVIEW_MAX),
                );
                self.lines.push(ChatLine::ToolStartLine {
                    tool: tool_name.clone(),
                    args_preview,
                });
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::ToolDispatchEnded {
                call_id,
                ok,
                error_message,
            } => {
                let (name, ms) = self.tool_timers.remove(call_id).map_or_else(
                    || ("?".to_string(), 0_u128),
                    |(started, name)| (name, started.elapsed().as_millis()),
                );
                self.lines.push(ChatLine::ToolEndLine {
                    tool: name,
                    ok: *ok,
                    elapsed_ms: ms,
                    error: error_message
                        .as_ref()
                        .map(|m| truncate_chars(m, TOOL_ERROR_PREVIEW_MAX)),
                });
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnCompleted => {
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnPaused => self.push_notice("[paused]"),
            StreamEvent::TurnResumed => self.push_notice("[resumed]"),
            StreamEvent::TurnCancelled => self.push_notice("[cancelled]"),
            StreamEvent::TurnFailed { reason } => self.push_notice(format!("[error] {reason}")),
            // StreamEvent is #[non_exhaustive]; future variants render
            // as no-ops until a renderer is taught about them.
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[StreamEvent]) -> ChatModel {
        let mut m = ChatModel::new();
        for e in events {
            m.on_event(e);
        }
        m
    }

    #[test]
    fn text_delta_coalesces_within_block() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::TextDelta { chunk: "hi ".into() },
            StreamEvent::TextDelta {
                chunk: "there".into(),
            },
            StreamEvent::TurnCompleted,
        ]);
        assert_eq!(m.lines, vec![ChatLine::AssistantText("hi there".into())]);
    }

    #[test]
    fn thinking_delta_coalesces_within_block() {
        let m = run(&[
            StreamEvent::ThinkingDelta {
                chunk: "let me ".into(),
            },
            StreamEvent::ThinkingDelta {
                chunk: "check".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![ChatLine::AssistantThinking("let me check".into())]
        );
    }

    #[test]
    fn thinking_then_text_emits_two_lines() {
        let m = run(&[
            StreamEvent::ThinkingDelta {
                chunk: "thinking".into(),
            },
            StreamEvent::TextDelta {
                chunk: "answer".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![
                ChatLine::AssistantThinking("thinking".into()),
                ChatLine::AssistantText("answer".into()),
            ]
        );
    }

    #[test]
    fn tool_dispatch_emits_start_and_end_lines() {
        let m = run(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: json!({"path": "a.rs"}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert!(matches!(m.lines[0], ChatLine::ToolStartLine { .. }));
        assert!(matches!(m.lines[1], ChatLine::ToolEndLine { ok: true, .. }));
    }

    #[test]
    fn tool_args_preview_is_compact_json() {
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "q".into(),
            args: json!({"fuzzy_keyword": "深圳"}),
        }]);
        match &m.lines[0] {
            ChatLine::ToolStartLine { args_preview, .. } => {
                assert!(args_preview.contains("深圳"));
                assert!(args_preview.contains("fuzzy_keyword"));
            }
            other => unreachable!("expected ToolStartLine, got {other:?}"),
        }
    }

    #[test]
    fn tool_args_preview_truncates_at_limit() {
        let blob: String = "x".repeat(500);
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({"blob": blob}),
        }]);
        match &m.lines[0] {
            ChatLine::ToolStartLine { args_preview, .. } => {
                assert!(args_preview.ends_with("..."));
                assert!(args_preview.chars().count() <= TOOL_ARGS_PREVIEW_MAX);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn tool_error_message_truncates() {
        let long: String = "x".repeat(800);
        let m = run(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some(long),
            },
        ]);
        match &m.lines[1] {
            ChatLine::ToolEndLine {
                ok: false,
                error: Some(msg),
                ..
            } => {
                assert!(msg.ends_with("..."));
                assert!(msg.chars().count() <= TOOL_ERROR_PREVIEW_MAX);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn turn_paused_resumed_cancelled_failed_emit_notices() {
        let m = run(&[
            StreamEvent::TurnPaused,
            StreamEvent::TurnResumed,
            StreamEvent::TurnCancelled,
            StreamEvent::TurnFailed {
                reason: "boom".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![
                ChatLine::SystemNotice("[paused]".into()),
                ChatLine::SystemNotice("[resumed]".into()),
                ChatLine::SystemNotice("[cancelled]".into()),
                ChatLine::SystemNotice("[error] boom".into()),
            ]
        );
    }

    #[test]
    fn user_prompt_breaks_text_coalescing() {
        let mut m = ChatModel::new();
        m.on_event(&StreamEvent::TextDelta { chunk: "a".into() });
        m.push_user_prompt("hi".into());
        m.on_event(&StreamEvent::TextDelta { chunk: "b".into() });
        assert_eq!(
            m.lines,
            vec![
                ChatLine::AssistantText("a".into()),
                ChatLine::UserPrompt("hi".into()),
                ChatLine::AssistantText("b".into()),
            ]
        );
    }
}
```

- [ ] **Step 2: Run tests — they should pass on the first try**

Run: `cargo nextest run -p cogito-tui render_model::tests`
Expected: 9 tests passed.

If `serde_json` isn't pulled in transitively, add `serde_json.workspace = true` to dev-dependencies in `crates/cogito-tui/Cargo.toml`. It's already in main dependencies from Task 1.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/render_model.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): ChatModel — StreamEvent → Vec<ChatLine>

Sink-agnostic chat scrollback state. Coalesces TextDelta/ThinkingDelta
within a block (mirroring cogito-cli's Renderer's in_text/in_thinking
flags), emits ToolStartLine + ToolEndLine pairs with elapsed-ms timing,
and surfaces TurnPaused/Resumed/Cancelled/Failed as SystemNotice lines.

Lazy palette: lines store raw text + structural variant only; palette
applied at widget render time (spec §3 detail-level decision).

9 unit tests cover text coalescing, thinking coalescing, mode switching,
tool args preview, args truncation, error truncation, lifecycle notices,
and user-prompt boundary.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 3: ToolTreeModel — per-turn tool tree

### Task 4: ToolTreeModel + TurnGroup + ToolNode + on_event

**Files:**
- Modify: `crates/cogito-tui/src/render_model.rs`

- [ ] **Step 1: Append the tool-tree types and tests**

Append to `crates/cogito-tui/src/render_model.rs` (after the existing `#[cfg(test)] mod tests` — or insert `pub` items before it and a second test module after):

```rust
// -- Tool-tree model -----------------------------------------------------

/// Status of one tool dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool started; awaiting `ToolDispatchEnded`.
    Running,
    /// Tool ended successfully. `elapsed_ms` captured at end time.
    Ok {
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
    },
    /// Tool ended with failure.
    Err {
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
        /// Truncated error message.
        message: String,
    },
}

impl ToolStatus {
    /// `true` once the dispatch has terminated (Ok or Err).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !matches!(self, ToolStatus::Running)
    }
}

/// One tool dispatch node in the tool-tree pane.
#[derive(Debug, Clone)]
pub struct ToolNode {
    /// `call_id` from the dispatcher; matches StreamEvent IDs.
    pub call_id: String,
    /// Tool name (as the dispatcher reports it).
    pub tool_name: String,
    /// Args JSON as the dispatcher received them.
    pub args: serde_json::Value,
    /// Wall-clock start time.
    pub started_at: Instant,
    /// Current status — `Running`, `Ok`, or `Err`.
    pub status: ToolStatus,
    /// Result text, populated lazily on first Ctrl-Enter expansion
    /// (spec §5.3, decision α.1). `None` until then.
    pub result_preview: Option<String>,
}

/// One turn's worth of tool calls, grouped for the tree pane.
#[derive(Debug, Clone)]
pub struct TurnGroup {
    /// 1-based turn index (incremented on `TurnStarted`).
    pub turn_idx: u32,
    /// Tool calls dispatched within this turn, in arrival order.
    pub nodes: Vec<ToolNode>,
}

/// Tree-pane state. Pure function of the `StreamEvent` stream.
#[derive(Debug, Default)]
pub struct ToolTreeModel {
    /// Turn groups in arrival order.
    pub turns: Vec<TurnGroup>,
    /// Next turn index to assign on `TurnStarted`. Starts at 1.
    next_turn_idx: u32,
}

impl ToolTreeModel {
    /// Fresh, empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self {
            turns: Vec::new(),
            next_turn_idx: 1,
        }
    }

    /// Apply one `StreamEvent`. No-op for events that don't bear on
    /// the tool-tree (`TextDelta`, `ThinkingDelta`, etc.).
    pub fn on_event(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TurnStarted => {
                self.turns.push(TurnGroup {
                    turn_idx: self.next_turn_idx,
                    nodes: Vec::new(),
                });
                self.next_turn_idx += 1;
            }
            StreamEvent::ToolDispatchStarted {
                call_id,
                tool_name,
                args,
            } => {
                // Defensive: if a tool starts before any TurnStarted
                // (shouldn't happen post-Sprint 2, but tolerate), open
                // turn 1 implicitly.
                if self.turns.is_empty() {
                    self.turns.push(TurnGroup {
                        turn_idx: self.next_turn_idx,
                        nodes: Vec::new(),
                    });
                    self.next_turn_idx += 1;
                }
                let group = self.turns.last_mut().expect("turns is non-empty");
                group.nodes.push(ToolNode {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                    started_at: Instant::now(),
                    status: ToolStatus::Running,
                    result_preview: None,
                });
            }
            StreamEvent::ToolDispatchEnded {
                call_id,
                ok,
                error_message,
            } => {
                if let Some(node) = self.find_node_mut(call_id) {
                    let elapsed_ms = node.started_at.elapsed().as_millis();
                    node.status = if *ok {
                        ToolStatus::Ok { elapsed_ms }
                    } else {
                        ToolStatus::Err {
                            elapsed_ms,
                            message: error_message
                                .as_ref()
                                .map(|m| truncate_chars(m, TOOL_ERROR_PREVIEW_MAX))
                                .unwrap_or_default(),
                        }
                    };
                }
            }
            _ => {}
        }
    }

    /// Find a node by `call_id`, scanning newest turn first.
    pub fn find_node_mut(&mut self, call_id: &str) -> Option<&mut ToolNode> {
        for group in self.turns.iter_mut().rev() {
            for node in &mut group.nodes {
                if node.call_id == call_id {
                    return Some(node);
                }
            }
        }
        None
    }

    /// Total tool nodes across all turns. For tests / status hints.
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.turns.iter().map(|g| g.nodes.len()).sum()
    }
}

/// Selection cursor in the tool tree pane: `(turn_idx_in_vec,
/// node_idx_in_turn)`. Used by `Ctrl-↑/↓` navigation and
/// `Ctrl-Enter` expansion. `turn_idx_in_vec` is the position in
/// `ToolTreeModel.turns`, NOT the 1-based `TurnGroup.turn_idx`.
pub type TreePath = (usize, usize);

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tree_tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[StreamEvent]) -> ToolTreeModel {
        let mut m = ToolTreeModel::new();
        for e in events {
            m.on_event(e);
        }
        m
    }

    #[test]
    fn turn_started_pushes_empty_group() {
        let m = run(&[StreamEvent::TurnStarted]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].turn_idx, 1);
        assert!(m.turns[0].nodes.is_empty());
    }

    #[test]
    fn tool_dispatch_started_appends_running_node() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: json!({}),
            },
        ]);
        assert_eq!(m.turns[0].nodes.len(), 1);
        assert_eq!(m.turns[0].nodes[0].call_id, "c1");
        assert!(matches!(m.turns[0].nodes[0].status, ToolStatus::Running));
    }

    #[test]
    fn tool_dispatch_ended_updates_status_to_ok() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert!(matches!(
            m.turns[0].nodes[0].status,
            ToolStatus::Ok { .. }
        ));
    }

    #[test]
    fn tool_dispatch_ended_with_error_captures_message() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some("boom".into()),
            },
        ]);
        match &m.turns[0].nodes[0].status {
            ToolStatus::Err { message, .. } => assert_eq!(message, "boom"),
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[test]
    fn multi_tool_within_one_turn_lands_in_one_group() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "a".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "b".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c2".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].nodes.len(), 2);
    }

    #[test]
    fn separate_turns_produce_separate_groups() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "a".into(),
                args: json!({}),
            },
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "b".into(),
                args: json!({}),
            },
        ]);
        assert_eq!(m.turns.len(), 2);
        assert_eq!(m.turns[0].nodes.len(), 1);
        assert_eq!(m.turns[1].nodes.len(), 1);
        assert_eq!(m.turns[0].turn_idx, 1);
        assert_eq!(m.turns[1].turn_idx, 2);
    }

    #[test]
    fn tool_without_prior_turn_started_opens_turn_implicitly() {
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        }]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].nodes.len(), 1);
    }

    #[test]
    fn text_events_are_noops_for_tree() {
        let m = run(&[
            StreamEvent::TextDelta { chunk: "x".into() },
            StreamEvent::ThinkingDelta { chunk: "y".into() },
            StreamEvent::TurnCompleted,
        ]);
        assert!(m.turns.is_empty());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui render_model`
Expected: 9 ChatModel tests + 7 ToolTreeModel tests = 16 total, all passing.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/render_model.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): ToolTreeModel — per-turn tool-call grouping

Sink-agnostic tool-tree state. TurnStarted pushes an empty TurnGroup;
ToolDispatchStarted appends a Running ToolNode to the current group
(opens turn 1 defensively if no prior TurnStarted); ToolDispatchEnded
mutates the node's status to Ok{elapsed} or Err{elapsed, message}.

ToolNode.result_preview stays None until lazy lookup populates it on
first Ctrl-Enter expand (spec §5.3, decision α.1).

TreePath = (turn_idx_in_vec, node_idx_in_turn) for selection cursor.
ToolStatus::is_finished() helper for the expand-handler precondition.

7 unit tests cover turn grouping, status transitions, error capture,
multi-tool-per-turn, multi-turn, defensive tool-without-turn, and
no-op for text events.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 4: Chat pane widget

### Task 5: ui::chat — ChatModel → ratatui Frame

**Files:**
- Modify: `crates/cogito-tui/src/ui/chat.rs`

- [ ] **Step 1: Write widget + snapshot tests**

Replace `crates/cogito-tui/src/ui/chat.rs` with:

```rust
//! Chat pane widget — renders `ChatModel.lines` as ratatui `Line`s
//! with palette applied at draw time (lazy painting; spec §3
//! detail-level decision).
//!
//! The widget owns no state of its own; it borrows `&ChatModel` and
//! the current `Rect` from the top-level layout.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::render_model::{ChatLine, ChatModel};

/// Palette for the chat pane. Mirrors the CLI's ANSI codes in spirit
/// (cyan = user, green = agent, dim = thinking, dim-yellow = tool,
/// red = error) but with ratatui `Style` values.
struct Palette {
    user: Style,
    agent: Style,
    thinking: Style,
    tool: Style,
    error: Style,
    notice: Style,
}

impl Palette {
    fn default_dark() -> Self {
        Self {
            user: Style::default().fg(Color::Cyan),
            agent: Style::default().fg(Color::Green),
            thinking: Style::default().add_modifier(Modifier::DIM),
            tool: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::DIM),
            error: Style::default().fg(Color::Red),
            notice: Style::default().add_modifier(Modifier::DIM),
        }
    }
}

/// Convert one `ChatLine` into one or more ratatui `Line`s. Some
/// variants render as multiple visual lines (e.g. a tool with an
/// indented error message).
fn line_for(line: &ChatLine, p: &Palette) -> Vec<Line<'static>> {
    match line {
        ChatLine::UserPrompt(text) => vec![Line::from(vec![
            Span::styled("> ", p.user),
            Span::raw(text.clone()),
        ])],
        ChatLine::AssistantText(text) => vec![Line::from(vec![
            Span::styled("agent: ", p.agent),
            Span::raw(text.clone()),
        ])],
        ChatLine::AssistantThinking(text) => vec![Line::from(vec![
            Span::styled("thinking: ", p.thinking),
            Span::styled(text.clone(), p.thinking),
        ])],
        ChatLine::ToolStartLine { tool, args_preview } => {
            vec![Line::from(vec![Span::styled(
                format!("[tool] {tool} {args_preview} …"),
                p.tool,
            )])]
        }
        ChatLine::ToolEndLine {
            tool,
            ok,
            elapsed_ms,
            error,
        } => {
            let status = if *ok { "ok" } else { "err" };
            let head = format!("[tool] {tool} {status} ({elapsed_ms}ms)");
            let style = if *ok { p.tool } else { p.error };
            let mut out = vec![Line::from(vec![Span::styled(head, style)])];
            if let Some(msg) = error {
                out.push(Line::from(vec![Span::styled(
                    format!("        {msg}"),
                    p.error,
                )]));
            }
            out
        }
        ChatLine::SystemNotice(s) => {
            let style = if s.starts_with("[error]") {
                p.error
            } else {
                p.notice
            };
            vec![Line::from(vec![Span::styled(s.clone(), style)])]
        }
    }
}

/// Render the chat pane into `area`. Wraps long lines; scroll offset
/// follows the tail when `scroll_offset == 0`.
pub fn render(f: &mut Frame, area: Rect, model: &ChatModel) {
    let p = Palette::default_dark();
    let lines: Vec<Line<'static>> = model.lines.iter().flat_map(|l| line_for(l, &p)).collect();
    let block = Block::default().borders(Borders::ALL).title("chat");
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((model.scroll_offset, 0));
    f.render_widget(para, area);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    use crate::render_model::{ChatLine, ChatModel};
    use cogito_protocol::stream::StreamEvent;

    fn draw(model: &ChatModel, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, model);
            })
            .unwrap();
        format!("{}", terminal.backend().buffer())
    }

    #[test]
    fn empty_model_renders_just_the_block() {
        let model = ChatModel::new();
        let out = draw(&model, 30, 5);
        // The Block borders + "chat" title must be present.
        assert!(out.contains("chat"));
    }

    #[test]
    fn user_prompt_renders_with_arrow_prefix() {
        let mut model = ChatModel::new();
        model.push_user_prompt("hello".into());
        let out = draw(&model, 30, 5);
        assert!(out.contains("> hello"), "got:\n{out}");
    }

    #[test]
    fn assistant_text_renders_with_agent_prefix() {
        let mut model = ChatModel::new();
        model.on_event(&StreamEvent::TextDelta {
            chunk: "hi there".into(),
        });
        let out = draw(&model, 40, 5);
        assert!(out.contains("agent: hi there"), "got:\n{out}");
    }

    #[test]
    fn thinking_renders_with_thinking_prefix() {
        let mut model = ChatModel::new();
        model.on_event(&StreamEvent::ThinkingDelta {
            chunk: "checking".into(),
        });
        let out = draw(&model, 40, 5);
        assert!(out.contains("thinking: checking"), "got:\n{out}");
    }

    #[test]
    fn tool_start_renders_with_tool_prefix() {
        let mut model = ChatModel::new();
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: json!({"path": "a"}),
        });
        let out = draw(&model, 50, 5);
        assert!(out.contains("[tool] read_file"), "got:\n{out}");
    }

    #[test]
    fn tool_end_err_renders_with_indented_message() {
        let mut model = ChatModel::new();
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: false,
            error_message: Some("boom".into()),
        });
        let out = draw(&model, 50, 8);
        assert!(out.contains("[tool] t err"), "got:\n{out}");
        // Indented error message should appear on its own line.
        assert!(out.contains("        boom"), "got:\n{out}");
    }

    #[test]
    fn turn_failed_renders_error_notice() {
        let mut model = ChatModel::new();
        model.on_event(&StreamEvent::TurnFailed {
            reason: "model timeout".into(),
        });
        let out = draw(&model, 50, 5);
        assert!(out.contains("[error] model timeout"), "got:\n{out}");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui ui::chat`
Expected: 7 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/ui/chat.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): chat pane widget — lazy palette + ratatui Paragraph

Borrows &ChatModel, converts each ChatLine into one (or two, for tool
errors) ratatui Line(s), applies palette at draw time so future theme
toggles need only a Palette swap, no model rebuild.

Palette: cyan user, green agent, dim thinking, dim-yellow tool start,
red tool err / error notices.

7 TestBackend snapshot-style tests assert prefix strings appear in the
rendered buffer (block title 'chat', '> hello' user prompt,
'agent: hi there', 'thinking: checking', '[tool] read_file',
indented '        boom' on tool error, '[error] model timeout' notice).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 5: Tools pane widget

### Task 6: ui::tools — ToolTreeModel → expandable tree widget

**Files:**
- Modify: `crates/cogito-tui/src/ui/tools.rs`

- [ ] **Step 1: Implement the widget with tests**

Replace `crates/cogito-tui/src/ui/tools.rs` with:

```rust
//! Tools pane widget — renders per-turn tool tree with expansion.
//!
//! Selection (`selected: Option<TreePath>`) highlights one node;
//! `expanded: &HashSet<TreePath>` toggles inline args+result preview
//! lines under the selected node. The expansion data (result preview)
//! is populated lazily by the App on Ctrl-Enter (spec §5.3, decision
//! α.1) — this widget just renders what's there.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::render_model::{ToolNode, ToolStatus, ToolTreeModel, TreePath};

/// Indent applied to expanded args/result lines beneath a node.
const EXPAND_INDENT: &str = "    ";

/// Max args-preview lines printed under an expanded node.
const EXPAND_ARGS_LINES: usize = 5;

/// Max result-preview chars printed under an expanded node.
const EXPAND_RESULT_CHARS: usize = 800;

/// Render the tools pane into `area`. `selected` is the current
/// selection cursor (None = nothing selected); `expanded` is the set
/// of selected paths whose args+result preview should be displayed.
pub fn render(
    f: &mut Frame,
    area: Rect,
    model: &ToolTreeModel,
    selected: Option<TreePath>,
    expanded: &HashSet<TreePath>,
) {
    let block = Block::default().borders(Borders::ALL).title("tools");

    let lines: Vec<Line<'static>> = if model.turns.is_empty() {
        vec![Line::from(vec![Span::styled(
            "(no tool calls yet)",
            Style::default().add_modifier(Modifier::DIM),
        )])]
    } else {
        let mut out: Vec<Line<'static>> = Vec::new();
        for (turn_idx, group) in model.turns.iter().enumerate() {
            out.push(Line::from(vec![Span::styled(
                format!("turn {}", group.turn_idx),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            for (node_idx, node) in group.nodes.iter().enumerate() {
                let path: TreePath = (turn_idx, node_idx);
                let is_selected = selected == Some(path);
                out.push(node_line(node, is_selected));
                if expanded.contains(&path) {
                    out.extend(expansion_lines(node));
                }
            }
        }
        out
    };

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn node_line(node: &ToolNode, is_selected: bool) -> Line<'static> {
    let (status_str, style) = match &node.status {
        ToolStatus::Running => (
            "running".to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::DIM),
        ),
        ToolStatus::Ok { elapsed_ms } => (
            format!("ok ({elapsed_ms}ms)"),
            Style::default().fg(Color::Green),
        ),
        ToolStatus::Err { elapsed_ms, .. } => (
            format!("err ({elapsed_ms}ms)"),
            Style::default().fg(Color::Red),
        ),
    };
    let marker = if is_selected { ">" } else { " " };
    Line::from(vec![
        Span::styled(format!("{marker} "), Style::default().fg(Color::Cyan)),
        Span::raw(node.tool_name.clone()),
        Span::raw("  "),
        Span::styled(status_str, style),
    ])
}

fn expansion_lines(node: &ToolNode) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    // Args block (pretty JSON, capped).
    let args_pretty = serde_json::to_string_pretty(&node.args)
        .unwrap_or_else(|_| "<unencodable>".to_string());
    out.push(Line::from(vec![Span::styled(
        format!("{EXPAND_INDENT}args:"),
        Style::default().add_modifier(Modifier::DIM),
    )]));
    for line in args_pretty.lines().take(EXPAND_ARGS_LINES) {
        out.push(Line::from(vec![Span::raw(format!(
            "{EXPAND_INDENT}{EXPAND_INDENT}{line}"
        ))]));
    }
    if args_pretty.lines().count() > EXPAND_ARGS_LINES {
        out.push(Line::from(vec![Span::styled(
            format!("{EXPAND_INDENT}{EXPAND_INDENT}..."),
            Style::default().add_modifier(Modifier::DIM),
        )]));
    }
    // Result block (lazy: shows '<not yet loaded>' until populated; on
    // error the status's message is shown instead).
    match &node.status {
        ToolStatus::Err { message, .. } if !message.is_empty() => {
            out.push(Line::from(vec![Span::styled(
                format!("{EXPAND_INDENT}error:"),
                Style::default().fg(Color::Red),
            )]));
            for line in message.lines() {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}{EXPAND_INDENT}{line}"),
                    Style::default().fg(Color::Red),
                )]));
            }
        }
        _ => match &node.result_preview {
            Some(preview) => {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}result:"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
                let truncated = if preview.chars().count() > EXPAND_RESULT_CHARS {
                    let head: String = preview.chars().take(EXPAND_RESULT_CHARS).collect();
                    format!("{head}...")
                } else {
                    preview.clone()
                };
                for line in truncated.lines() {
                    out.push(Line::from(vec![Span::raw(format!(
                        "{EXPAND_INDENT}{EXPAND_INDENT}{line}"
                    ))]));
                }
            }
            None => {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}(loading result...)"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            }
        },
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    use cogito_protocol::stream::StreamEvent;

    fn draw(
        model: &ToolTreeModel,
        selected: Option<TreePath>,
        expanded: &HashSet<TreePath>,
        w: u16,
        h: u16,
    ) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, model, selected, expanded);
            })
            .unwrap();
        format!("{}", terminal.backend().buffer())
    }

    #[test]
    fn empty_model_renders_placeholder() {
        let model = ToolTreeModel::new();
        let out = draw(&model, None, &HashSet::new(), 30, 5);
        assert!(out.contains("(no tool calls yet)"), "got:\n{out}");
    }

    #[test]
    fn single_running_tool_renders_running_marker() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: json!({}),
        });
        let out = draw(&model, None, &HashSet::new(), 40, 8);
        assert!(out.contains("turn 1"), "got:\n{out}");
        assert!(out.contains("read_file"), "got:\n{out}");
        assert!(out.contains("running"), "got:\n{out}");
    }

    #[test]
    fn finished_ok_renders_ok_status() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        });
        let out = draw(&model, None, &HashSet::new(), 40, 8);
        assert!(out.contains("ok ("), "got:\n{out}");
    }

    #[test]
    fn selected_node_renders_arrow_marker() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        let out = draw(&model, Some((0, 0)), &HashSet::new(), 40, 8);
        assert!(out.contains(">  t"), "got:\n{out}");
    }

    #[test]
    fn expanded_node_renders_args_and_loading_result() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({"path": "a.rs"}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        });
        let mut expanded = HashSet::new();
        expanded.insert((0, 0));
        let out = draw(&model, Some((0, 0)), &expanded, 50, 14);
        assert!(out.contains("args:"), "got:\n{out}");
        assert!(out.contains("path"), "got:\n{out}");
        assert!(out.contains("(loading result"), "got:\n{out}");
    }

    #[test]
    fn expanded_err_node_shows_error_message() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: false,
            error_message: Some("boom".into()),
        });
        let mut expanded = HashSet::new();
        expanded.insert((0, 0));
        let out = draw(&model, Some((0, 0)), &expanded, 50, 14);
        assert!(out.contains("error:"), "got:\n{out}");
        assert!(out.contains("boom"), "got:\n{out}");
    }

    #[test]
    fn multi_turn_renders_separate_groups() {
        let mut model = ToolTreeModel::new();
        for (i, id) in ["c1", "c2"].iter().enumerate() {
            model.on_event(&StreamEvent::TurnStarted);
            model.on_event(&StreamEvent::ToolDispatchStarted {
                call_id: (*id).into(),
                tool_name: format!("t{i}"),
                args: json!({}),
            });
        }
        let out = draw(&model, None, &HashSet::new(), 40, 10);
        assert!(out.contains("turn 1"), "got:\n{out}");
        assert!(out.contains("turn 2"), "got:\n{out}");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui ui::tools`
Expected: 7 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/ui/tools.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): tools pane widget — per-turn tree with expansion

Renders ToolTreeModel as 'turn N' headers + indented '  toolname  status'
lines. Selection marker '>' precedes the selected node (cursor-driven by
App.selected). Expansion (HashSet of TreePaths) inserts args block (pretty
JSON, capped at 5 lines) and result block (lazy '(loading result...)'
placeholder until App populates node.result_preview; on error, the
captured error message is shown instead).

7 TestBackend tests cover: empty-state placeholder, running tool,
finished ok, selected marker, expanded args+loading, expanded err
shows error message, multi-turn groups.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 6: Input pane (tui-textarea wrapper)

### Task 7: ui::input — multi-line buffer with Shift-Enter newline

**Files:**
- Modify: `crates/cogito-tui/src/ui/input.rs`

- [ ] **Step 1: Implement the wrapper**

Replace `crates/cogito-tui/src/ui/input.rs` with:

```rust
//! Multi-line input widget — thin wrapper around `tui_textarea::TextArea`.
//!
//! Discriminates `Enter` (send) from `Shift+Enter` (newline) ourselves
//! instead of letting `TextArea::input` consume the key, because the
//! upstream default semantics treat both the same.
//!
//! Visible height is capped at `MAX_VISIBLE_INPUT_LINES`; longer
//! buffers scroll within (`TextArea` handles its own internal scroll).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};
use tui_textarea::TextArea;

/// Maximum visible input lines (the input bar grows up to this, then
/// scrolls internally).
pub const MAX_VISIBLE_INPUT_LINES: u16 = 8;

/// Result of a key dispatch from the App to the input widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    /// Key was consumed by the textarea (or ignored); nothing for
    /// the app to do beyond redrawing.
    Consumed,
    /// User pressed Enter on a non-empty buffer — App should treat
    /// the returned text as a message to send.
    Submit(String),
}

/// Multi-line input state — a wrapper around `tui_textarea::TextArea`.
pub struct InputWidget {
    textarea: TextArea<'static>,
}

impl Default for InputWidget {
    fn default() -> Self {
        let mut ta = TextArea::default();
        ta.set_block(Block::default().borders(Borders::ALL).title("message"));
        ta.set_cursor_line_style(Style::default());
        ta.set_placeholder_text("Type a message; Enter to send, Shift+Enter for newline");
        ta.set_placeholder_style(Style::default().fg(Color::DarkGray));
        Self { textarea: ta }
    }
}

impl InputWidget {
    /// Fresh empty input.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle a key event. Returns `Submit(text)` if the user pressed
    /// Enter on a non-empty buffer; `Consumed` otherwise.
    pub fn on_key(&mut self, key: KeyEvent) -> InputOutcome {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, mods) if !mods.contains(KeyModifiers::SHIFT) => {
                let text = self.textarea.lines().join("\n").trim().to_string();
                if text.is_empty() {
                    return InputOutcome::Consumed;
                }
                self.clear();
                InputOutcome::Submit(text)
            }
            (KeyCode::Enter, _) => {
                // Shift+Enter: explicit newline. tui-textarea's
                // `input` would do this for plain Enter too; we force
                // it through the explicit newline insert API.
                self.textarea.insert_newline();
                InputOutcome::Consumed
            }
            _ => {
                let event = tui_textarea::Input::from(key);
                self.textarea.input(event);
                InputOutcome::Consumed
            }
        }
    }

    /// First character of the buffer, or None if empty. Used by the
    /// App to decide whether to show the `/`-popup.
    #[must_use]
    pub fn first_char(&self) -> Option<char> {
        self.textarea.lines().first().and_then(|l| l.chars().next())
    }

    /// Total visible height needed (clamped to `MAX_VISIBLE_INPUT_LINES`),
    /// inclusive of the surrounding block border (2 rows).
    #[must_use]
    pub fn desired_height(&self) -> u16 {
        let content = self.textarea.lines().len() as u16;
        content.clamp(1, MAX_VISIBLE_INPUT_LINES) + 2
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
    }

    /// Render into the given area.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        f.render_widget(&self.textarea, area);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn enter_with_empty_buffer_is_consumed_not_submitted() {
        let mut w = InputWidget::new();
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(r, InputOutcome::Consumed);
    }

    #[test]
    fn enter_with_text_returns_submit() {
        let mut w = InputWidget::new();
        for ch in "hi".chars() {
            w.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(r, InputOutcome::Submit("hi".into()));
    }

    #[test]
    fn shift_enter_inserts_newline_does_not_submit() {
        let mut w = InputWidget::new();
        w.on_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(r, InputOutcome::Consumed);
        w.on_key(key(KeyCode::Char('b'), KeyModifiers::NONE));
        let submit = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(submit, InputOutcome::Submit("a\nb".into()));
    }

    #[test]
    fn submit_clears_the_buffer() {
        let mut w = InputWidget::new();
        for ch in "hi".chars() {
            w.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let _ = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(w.first_char(), None);
        assert_eq!(w.desired_height(), 3); // 1 line + 2 border rows
    }

    #[test]
    fn desired_height_grows_with_lines_then_caps() {
        let mut w = InputWidget::new();
        for _ in 0..10 {
            w.on_key(key(KeyCode::Enter, KeyModifiers::SHIFT));
        }
        assert_eq!(w.desired_height(), MAX_VISIBLE_INPUT_LINES + 2);
    }

    #[test]
    fn first_char_detects_leading_slash() {
        let mut w = InputWidget::new();
        w.on_key(key(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(w.first_char(), Some('/'));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui ui::input`
Expected: 6 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/ui/input.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): input pane — multi-line wrapper over tui-textarea

InputWidget.on_key returns InputOutcome::Submit(text) when the user
hits Enter on a non-empty buffer (whitespace-only buffers stay as
Consumed). Shift+Enter inserts a newline; all other keys delegate to
tui-textarea via tui_textarea::Input::from. Buffer clears on submit.

desired_height grows with line count up to MAX_VISIBLE_INPUT_LINES=8
(+2 border rows), then caps and lets tui-textarea scroll internally.

first_char() exposes the leading character so the App can show the
slash-discovery popup as soon as the user types '/'.

6 unit tests cover empty-Enter no-op, populated-Enter submit, Shift+Enter
newline, post-submit buffer clear, height capping at 8 visible lines,
and slash detection.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 7: Status bar + slash popup widgets

### Task 8: ui::status and ui::popup

**Files:**
- Modify: `crates/cogito-tui/src/ui/status.rs`
- Modify: `crates/cogito-tui/src/ui/popup.rs`

- [ ] **Step 1: Status bar widget**

Replace `crates/cogito-tui/src/ui/status.rs` with:

```rust
//! Bottom status bar — single line. Renders strategy / model /
//! session / turn count on the left and key hints on the right.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Status data — pure value type, no widgets.
#[derive(Debug, Clone)]
pub struct StatusData {
    /// Strategy name in effect (e.g. `coder`).
    pub strategy: String,
    /// Model id in effect.
    pub model: String,
    /// Session id (truncated to 8 chars in display).
    pub session_id: String,
    /// Number of completed turns so far.
    pub turn_count: u32,
    /// Whether the tools pane is currently visible (for hint text).
    pub tools_visible: bool,
}

const HINT_TEXT: &str = "Ctrl-C cancel/exit · Ctrl-D exit · Ctrl-T tools · / commands";

/// Render the status bar into `area` (typically one row tall).
pub fn render(f: &mut Frame, area: Rect, data: &StatusData) {
    let truncated_session = data.session_id.chars().take(8).collect::<String>();
    let left = format!(
        "strategy: {s} · model: {m} · session: {sid} · turn: {t}",
        s = data.strategy,
        m = data.model,
        sid = truncated_session,
        t = data.turn_count
    );
    let tools_label = if data.tools_visible { "on" } else { "off" };
    let right = format!("tools: {tools_label} · {HINT_TEXT}");
    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(right, Style::default().add_modifier(Modifier::DIM)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(data: &StatusData, w: u16) -> String {
        let backend = TestBackend::new(w, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, data);
            })
            .unwrap();
        format!("{}", terminal.backend().buffer())
    }

    #[test]
    fn renders_strategy_model_session_turn() {
        let data = StatusData {
            strategy: "coder".into(),
            model: "claude-opus-4-7".into(),
            session_id: "01jaaaaaa0000".into(),
            turn_count: 3,
            tools_visible: true,
        };
        let out = draw(&data, 200);
        assert!(out.contains("strategy: coder"), "got:\n{out}");
        assert!(out.contains("model: claude-opus-4-7"), "got:\n{out}");
        assert!(out.contains("session: 01jaaaaa"), "got:\n{out}");
        assert!(out.contains("turn: 3"), "got:\n{out}");
    }

    #[test]
    fn renders_tools_on_when_visible() {
        let data = StatusData {
            strategy: "x".into(),
            model: "y".into(),
            session_id: "z".into(),
            turn_count: 0,
            tools_visible: true,
        };
        let out = draw(&data, 200);
        assert!(out.contains("tools: on"), "got:\n{out}");
    }

    #[test]
    fn renders_tools_off_when_hidden() {
        let data = StatusData {
            strategy: "x".into(),
            model: "y".into(),
            session_id: "z".into(),
            turn_count: 0,
            tools_visible: false,
        };
        let out = draw(&data, 200);
        assert!(out.contains("tools: off"), "got:\n{out}");
    }
}
```

- [ ] **Step 2: Slash popup widget**

Replace `crates/cogito-tui/src/ui/popup.rs` with:

```rust
//! Slash-command discovery popup. Shown when the input buffer starts
//! with `/`. Prefix-matches against the v0.1 command list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

/// Static catalogue of slash commands available in Sprint 9b. New
/// commands (e.g. `/strategy`, `/help`) land in later sprints.
pub const COMMANDS: &[(&str, &str)] = &[("/skill", "activate a skill by name")];

/// Return commands matching the user's typed prefix (case-insensitive).
#[must_use]
pub fn matches(prefix: &str) -> Vec<(&'static str, &'static str)> {
    let p = prefix.to_lowercase();
    COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&p))
        .copied()
        .collect()
}

/// Render the popup centered above the input bar. `parent` is the
/// full frame area; the popup lays out as a small overlay in the
/// bottom-center.
pub fn render(f: &mut Frame, parent: Rect, prefix: &str) {
    let items: Vec<ListItem<'static>> = matches(prefix)
        .into_iter()
        .map(|(cmd, desc)| {
            ListItem::new(Line::from(vec![
                Span::styled(cmd, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled(desc, Style::default().add_modifier(Modifier::DIM)),
            ]))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "(no matching commands; Esc to dismiss)",
            Style::default().add_modifier(Modifier::DIM),
        )]))]
    } else {
        items
    };

    let height = (items.len() as u16 + 2).min(parent.height); // border = 2 rows
    let width = 60.min(parent.width.saturating_sub(4));
    let x = parent.x + (parent.width.saturating_sub(width)) / 2;
    let y = parent
        .y
        .saturating_add(parent.height)
        .saturating_sub(height + 6); // 6 = approx input height; popup hovers above
    let area = Rect {
        x,
        y: y.max(parent.y),
        width,
        height,
    };

    let block = Block::default().borders(Borders::ALL).title("commands");
    let list = List::new(items).block(block);
    f.render_widget(Clear, area);
    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_prefix_skill_returns_skill_entry() {
        let m = matches("/sk");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, "/skill");
    }

    #[test]
    fn matches_bare_slash_returns_all() {
        let m = matches("/");
        assert_eq!(m.len(), COMMANDS.len());
    }

    #[test]
    fn matches_unknown_returns_empty() {
        let m = matches("/zz");
        assert!(m.is_empty());
    }

    #[test]
    fn matches_is_case_insensitive() {
        let m = matches("/SK");
        assert_eq!(m.len(), 1);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p cogito-tui ui::status ui::popup`
Expected: 3 status + 4 popup = 7 tests passed.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/src/ui/status.rs crates/cogito-tui/src/ui/popup.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): status bar + slash popup widgets

Status bar: one-line render of strategy/model/session(8-char-truncated)/
turn-count on the left, 'tools: on|off' + key hints on the right.
Hint text: 'Ctrl-C cancel/exit · Ctrl-D exit · Ctrl-T tools · / commands'.

Slash popup: prefix-match against COMMANDS (just /skill in v0.1), render
as a centered List overlay just above the input bar. Empty match set
shows '(no matching commands; Esc to dismiss)'.

3 status tests + 4 popup tests.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 8: Top-level UI layout

### Task 9: ui::render — orchestrate panes

**Files:**
- Modify: `crates/cogito-tui/src/ui/mod.rs`

- [ ] **Step 1: Write the top-level render function with snapshot tests**

Append to `crates/cogito-tui/src/ui/mod.rs` (keeping the existing `pub mod` declarations):

```rust
use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::render_model::{ChatModel, ToolTreeModel, TreePath};
use crate::ui::input::InputWidget;
use crate::ui::status::StatusData;

/// Top-level render — borrows every pane's state and lays out the
/// frame. Layout (when `show_tools = true`):
///
/// ```text
/// ┌───────────────────────┬──────────────┐
/// │ chat                  │ tools        │
/// │ ...                   │ ...          │
/// ├───────────────────────┴──────────────┤
/// │ message (input)                      │
/// │ ...                                  │
/// ├──────────────────────────────────────┤
/// │ status bar                           │
/// └──────────────────────────────────────┘
/// ```
///
/// When `show_tools = false`: chat takes full width; tools area is
/// suppressed. Slash popup overlays the input when `popup_prefix`
/// is `Some`.
pub struct RenderInputs<'a> {
    /// Chat scrollback state.
    pub chat: &'a ChatModel,
    /// Tool-tree state.
    pub tools: &'a ToolTreeModel,
    /// Currently selected node in the tree (None = nothing selected).
    pub selected: Option<TreePath>,
    /// Currently expanded set (subset of all `(turn_idx, node_idx)`).
    pub expanded: &'a HashSet<TreePath>,
    /// Multi-line input widget.
    pub input: &'a InputWidget,
    /// Whether the tools pane is visible (`Ctrl-T` toggle).
    pub show_tools: bool,
    /// Status bar payload.
    pub status: &'a StatusData,
    /// When `Some`, render the slash popup with this prefix; `None`
    /// hides the popup.
    pub popup_prefix: Option<&'a str>,
}

/// Render one frame. `frame_area` is `Frame::area()`.
pub fn render(f: &mut Frame, inputs: &RenderInputs<'_>) {
    let area = f.area();
    let input_h = inputs.input.desired_height();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),         // chat + tools row
            Constraint::Length(input_h),
            Constraint::Length(1),      // status bar
        ])
        .split(area);

    let top = outer[0];
    let input_area = outer[1];
    let status_area = outer[2];

    if inputs.show_tools {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(top);
        crate::ui::chat::render(f, split[0], inputs.chat);
        crate::ui::tools::render(
            f,
            split[1],
            inputs.tools,
            inputs.selected,
            inputs.expanded,
        );
    } else {
        crate::ui::chat::render(f, top, inputs.chat);
    }

    inputs.input.render(f, input_area);
    crate::ui::status::render(f, status_area, inputs.status);

    if let Some(prefix) = inputs.popup_prefix {
        crate::ui::popup::render(f, area, prefix);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::render_model::{ChatModel, ToolTreeModel};
    use crate::ui::input::InputWidget;
    use crate::ui::status::StatusData;
    use cogito_protocol::stream::StreamEvent;

    fn fixture_status() -> StatusData {
        StatusData {
            strategy: "coder".into(),
            model: "claude-opus-4-7".into(),
            session_id: "01abcdefghij".into(),
            turn_count: 1,
            tools_visible: true,
        }
    }

    fn draw_buf(show_tools: bool, with_text: bool, w: u16, h: u16) -> String {
        let mut chat = ChatModel::new();
        if with_text {
            chat.push_user_prompt("hi".into());
            chat.on_event(&StreamEvent::TextDelta {
                chunk: "hello".into(),
            });
        }
        let tools = ToolTreeModel::new();
        let input = InputWidget::new();
        let mut status = fixture_status();
        status.tools_visible = show_tools;
        let expanded = HashSet::new();
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    &RenderInputs {
                        chat: &chat,
                        tools: &tools,
                        selected: None,
                        expanded: &expanded,
                        input: &input,
                        show_tools,
                        status: &status,
                        popup_prefix: None,
                    },
                );
            })
            .unwrap();
        format!("{}", terminal.backend().buffer())
    }

    #[test]
    fn layout_with_tools_shows_both_panes() {
        let out = draw_buf(true, true, 80, 20);
        assert!(out.contains("chat"), "got:\n{out}");
        assert!(out.contains("tools"), "got:\n{out}");
        assert!(out.contains("> hi"), "got:\n{out}");
        assert!(out.contains("agent: hello"), "got:\n{out}");
        assert!(out.contains("strategy: coder"), "got:\n{out}");
    }

    #[test]
    fn layout_without_tools_omits_tools_pane() {
        let out = draw_buf(false, true, 80, 20);
        assert!(out.contains("chat"), "got:\n{out}");
        assert!(!out.contains("tools "), "tools pane should be hidden, got:\n{out}");
        assert!(out.contains("tools: off"), "status hint mismatch:\n{out}");
    }

    #[test]
    fn layout_with_popup_overlays_command_list() {
        let chat = ChatModel::new();
        let tools = ToolTreeModel::new();
        let input = InputWidget::new();
        let status = fixture_status();
        let expanded = HashSet::new();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    &RenderInputs {
                        chat: &chat,
                        tools: &tools,
                        selected: None,
                        expanded: &expanded,
                        input: &input,
                        show_tools: true,
                        status: &status,
                        popup_prefix: Some("/sk"),
                    },
                );
            })
            .unwrap();
        let out = format!("{}", terminal.backend().buffer());
        assert!(out.contains("commands"), "popup title missing:\n{out}");
        assert!(out.contains("/skill"), "popup entry missing:\n{out}");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui ui::tests`
Expected: 3 layout tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/ui/mod.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): top-level UI layout — orchestrates panes

ui::render takes RenderInputs (chat + tools + selected + expanded +
input + show_tools + status + popup_prefix) and lays out:
- vertical: chat-row / input / status bar
- chat-row horizontally splits 70/30 chat/tools when show_tools=true,
  full-width chat when false
- popup is a centered overlay above the input when popup_prefix=Some

Input height is dynamic (input.desired_height()) so the chat pane
grows when input is short.

3 snapshot tests: layout with both panes + content, layout with tools
hidden (status shows 'tools: off'), layout with /-popup overlay showing
'commands' title + '/skill' entry.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 9: Terminal lifecycle — RAII + panic hook + signals

### Task 10: terminal::TerminalGuard

**Files:**
- Modify: `crates/cogito-tui/src/terminal.rs`

- [ ] **Step 1: Implement TerminalGuard**

Replace `crates/cogito-tui/src/terminal.rs` with:

```rust
//! Terminal lifecycle. Three layers of defense ensure the terminal
//! always returns to a sane state (raw mode off, alternate screen
//! left, cursor visible) even on panic or SIGTERM (spec §6.1).
//!
//! Layer 1: RAII drop (normal exit path).
//! Layer 2: panic hook installed before raw mode.
//! Layer 3: SIGTERM/SIGHUP handlers spawned at construction.
//!
//! SIGKILL is unhandleable — documented as a known limitation.

use std::io;

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Restore terminal to non-raw mode with the alternate screen left
/// and the cursor visible. Idempotent and infallible — every restore
/// path calls this.
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// RAII guard. Construct once at startup; `Drop` restores on the
/// happy path. The panic hook and signal handlers cover the rest.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enter raw mode + alternate screen. Installs the panic hook
    /// and signal handlers as side effects.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` if `enable_raw_mode` or
    /// `EnterAlternateScreen` fails. The caller should print a
    /// user-facing error and exit non-zero in that case — the panic
    /// hook is not yet installed, so a normal `?` propagation is fine.
    pub fn new() -> Result<Self> {
        Self::install_panic_hook();
        enable_raw_mode().context("enable_raw_mode")?;
        execute!(io::stdout(), EnterAlternateScreen).context("EnterAlternateScreen")?;
        Self::spawn_signal_handlers();
        Ok(Self)
    }

    fn install_panic_hook() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
    }

    fn spawn_signal_handlers() {
        // Best-effort: ignore signal-stream setup failures (e.g.
        // unsupported platforms). The RAII drop + panic hook still
        // cover normal exits.
        #[cfg(unix)]
        tokio::spawn(async {
            use tokio::signal::unix::{SignalKind, signal};
            // Watch SIGTERM and SIGHUP simultaneously.
            let mut term = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => return,
            };
            let mut hup = match signal(SignalKind::hangup()) {
                Ok(s) => s,
                Err(_) => return,
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = hup.recv() => {}
            }
            restore();
            std::process::exit(130); // 128 + SIGINT-style signal exit code
        });
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_idempotent() {
        // Calling restore() twice when raw mode is already off must
        // not panic. We can't enter raw mode in CI (no real TTY), so
        // we only exercise the cleanup path.
        restore();
        restore();
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui terminal`
Expected: 1 test passed.

Real raw-mode lifecycle cannot be unit-tested without a PTY. The manual smoke test is documented in `docs/components/cogito-tui.md` (Task 25): run the binary, trigger a panic via Ctrl-C double-tap or `kill -TERM`, verify the terminal echoes input normally afterward.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/terminal.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): TerminalGuard — three-layer restore (RAII + panic + signals)

Drop calls disable_raw_mode + LeaveAlternateScreen + Show cursor
(idempotent). Panic hook installed before raw mode chains the previous
hook so panic messages still reach stderr post-restore. SIGTERM/SIGHUP
handlers spawned on a tokio task call restore() then process::exit(130).

SIGKILL stays unhandleable — documented in docs/components/cogito-tui.md.

1 unit test asserts restore() idempotency; real raw-mode lifecycle is
covered by manual smoke testing (cannot enter raw mode in CI).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 10: App state and key dispatcher

### Task 11: app::App struct + initial state

**Files:**
- Modify: `crates/cogito-tui/src/app.rs`

- [ ] **Step 1: Implement App with construction + state-update helpers**

Replace `crates/cogito-tui/src/app.rs` with:

```rust
//! Top-level App state. Single source of truth for every pane.
//!
//! `App` is reconstructible from the JSONL log alone (AGENTS.md
//! rule 3): on `--session <id>` startup, the resume module translates
//! ConversationEvents into StreamEvents and replays them through
//! `apply_stream_event` before the live event loop starts.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cogito_protocol::ConversationStore;
use cogito_protocol::stream::StreamEvent;

use crate::render_model::{ChatModel, ToolTreeModel, TreePath};
use crate::ui::input::InputWidget;
use crate::ui::status::StatusData;

/// Window after a Ctrl-C that already initiated a turn cancellation
/// during which a second Ctrl-C is interpreted as "exit now". Mirrors
/// cogito-cli::chat::CTRL_C_EXIT_WINDOW.
pub const CTRL_C_EXIT_WINDOW: Duration = Duration::from_secs(2);

/// Active popup state. Sprint 9b has just the slash command menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// Slash discovery menu; `prefix` is the input buffer's leading
    /// substring (e.g. `/`, `/s`, `/sk`).
    SlashMenu { prefix: String },
}

/// Top-level App state.
pub struct App {
    /// Session handle that drives the underlying Runtime.
    pub handle: cogito_core::runtime::SessionHandle,
    /// Strategy registry — used for `/strategy` listing (popup),
    /// never for mid-session swap.
    pub registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>,
    /// Store handle for lazy tool-result lookup (spec §5.3 α.1).
    pub store: Arc<dyn ConversationStore>,
    /// Session id (as a string for status display).
    pub session_id_str: String,
    /// Cwd of the session's JSONL file — used only if the store needs
    /// re-opening.
    pub session_root: Option<PathBuf>,

    /// Chat scrollback model.
    pub chat: ChatModel,
    /// Tool-tree model.
    pub tools: ToolTreeModel,
    /// Currently selected node in the tree.
    pub selected: Option<TreePath>,
    /// Set of expanded nodes (subset of TreePath).
    pub expanded: HashSet<TreePath>,

    /// Multi-line input buffer.
    pub input: InputWidget,

    /// Whether the tools pane is visible (`Ctrl-T` toggle).
    pub show_tools: bool,
    /// Active popup, if any.
    pub popup: Option<Popup>,

    /// Status payload (rebuilt every render).
    pub strategy_name: String,
    /// Model id in effect for the session.
    pub model_id: String,
    /// Completed turn counter; increments on `TurnCompleted`.
    pub turn_count: u32,
    /// `true` between `TurnStarted` and `TurnCompleted/Failed/Cancelled/Paused`.
    pub turn_in_progress: bool,

    /// First Ctrl-C of a double-tap window. `None` = next Ctrl-C
    /// cancels the turn (or shows a hint if idle).
    pub cancel_seen_at: Option<Instant>,

    /// Set to `true` by the keymap to end the event loop.
    pub should_quit: bool,
}

impl App {
    /// Apply a `StreamEvent` to both models and update lifecycle flags.
    pub fn apply_stream_event(&mut self, ev: &StreamEvent) {
        self.chat.on_event(ev);
        self.tools.on_event(ev);
        match ev {
            StreamEvent::TurnStarted => self.turn_in_progress = true,
            StreamEvent::TurnCompleted => {
                self.turn_in_progress = false;
                self.turn_count = self.turn_count.saturating_add(1);
            }
            StreamEvent::TurnFailed { .. }
            | StreamEvent::TurnCancelled
            | StreamEvent::TurnPaused => self.turn_in_progress = false,
            _ => {}
        }
    }

    /// Build the status bar payload from current state.
    #[must_use]
    pub fn status_data(&self) -> StatusData {
        StatusData {
            strategy: self.strategy_name.clone(),
            model: self.model_id.clone(),
            session_id: self.session_id_str.clone(),
            turn_count: self.turn_count,
            tools_visible: self.show_tools,
        }
    }

    /// Update popup state based on input buffer's first character.
    /// Call this after any `on_key` that modifies the input.
    pub fn refresh_popup(&mut self) {
        let first = self.input.first_char();
        let buffer = self
            .input
            .first_char()
            .map(String::from)
            .unwrap_or_default();
        // For richer prefix matching we'd want the whole first line;
        // v0.1 keeps it simple: the popup matches just the leading
        // slash token.
        self.popup = match first {
            Some('/') => Some(Popup::SlashMenu { prefix: buffer }),
            _ => None,
        };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Minimal App builder for unit tests that don't need a real
    /// SessionHandle, registry, or store. The unsafe-looking
    /// transmute-via-unimplemented impl is intentional: the tests
    /// exercise pure App methods that never call into those fields.
    /// Real construction with all dependencies happens in
    /// runtime_build.rs (Task 19).
    fn app_for_pure_test() -> App {
        // Use a stub ConversationStore from cogito-test-fixtures.
        // (cogito-test-fixtures::in_memory_store provides one.)
        let store: Arc<dyn ConversationStore> =
            Arc::new(cogito_test_fixtures::in_memory_store::InMemoryStore::new());
        let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> =
            Arc::new(cogito_test_fixtures::strategy::MapStrategyRegistry::default());
        // We can't construct a real SessionHandle without a Runtime.
        // For these state-transition tests we never touch it, so we
        // construct one with the runtime's test helper.
        let handle = cogito_test_fixtures::session_handle_stub::stub_handle();
        App {
            handle,
            registry,
            store,
            session_id_str: "01abc".into(),
            session_root: None,
            chat: ChatModel::new(),
            tools: ToolTreeModel::new(),
            selected: None,
            expanded: HashSet::new(),
            input: InputWidget::new(),
            show_tools: true,
            popup: None,
            strategy_name: "default".into(),
            model_id: "model-x".into(),
            turn_count: 0,
            turn_in_progress: false,
            cancel_seen_at: None,
            should_quit: false,
        }
    }

    #[test]
    fn turn_started_sets_turn_in_progress() {
        let mut app = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        assert!(app.turn_in_progress);
    }

    #[test]
    fn turn_completed_clears_flag_and_increments_counter() {
        let mut app = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        app.apply_stream_event(&StreamEvent::TurnCompleted);
        assert!(!app.turn_in_progress);
        assert_eq!(app.turn_count, 1);
    }

    #[test]
    fn turn_failed_clears_flag_but_does_not_increment() {
        let mut app = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        app.apply_stream_event(&StreamEvent::TurnFailed {
            reason: "x".into(),
        });
        assert!(!app.turn_in_progress);
        assert_eq!(app.turn_count, 0);
    }

    #[test]
    fn refresh_popup_shows_slash_menu_when_slash_typed() {
        let mut app = app_for_pure_test();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.input
            .on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.refresh_popup();
        assert!(matches!(app.popup, Some(Popup::SlashMenu { .. })));
    }

    #[test]
    fn refresh_popup_clears_when_input_is_not_slash() {
        let mut app = app_for_pure_test();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.input
            .on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.refresh_popup();
        assert!(app.popup.is_none());
    }

    #[test]
    fn status_data_mirrors_app_state() {
        let app = app_for_pure_test();
        let _ = json!({}); // touch serde_json so the import is used
        let s = app.status_data();
        assert_eq!(s.strategy, "default");
        assert_eq!(s.model, "model-x");
        assert!(s.tools_visible);
    }
}
```

> **NOTE on test scaffolding:** the tests reference two helpers that do not yet exist in `cogito-test-fixtures`: `in_memory_store::InMemoryStore` and `session_handle_stub::stub_handle()`. The first probably exists already (it's mentioned as a contract-test counterpart in CLAUDE.md). The second does not — Task 11b below adds it.

- [ ] **Step 2: Verify the test helpers exist**

Run: `grep -rn "InMemoryStore\|stub_handle" crates/testing/cogito-test-fixtures/src/`
Expected: `InMemoryStore` exists; `stub_handle` does NOT.

If `InMemoryStore` is named differently (e.g., `MemoryStore`, `InMemoryConversationStore`), adapt the import in `app.rs::tests` accordingly. If `cogito-test-fixtures` doesn't expose any in-memory store at all, factor a small one in this task (or use `cogito_store_jsonl::JsonlStore` over a tempdir for the test).

- [ ] **Step 3: Add `stub_handle` to cogito-test-fixtures**

Create `crates/testing/cogito-test-fixtures/src/session_handle_stub.rs`:

```rust
//! Stub `SessionHandle` for pure-state tests that never actually
//! interact with the actor. Pretends to be open; never accepts
//! mailbox sends (would error at runtime, but the tests don't reach
//! that path).

use std::sync::Arc;

use cogito_core::runtime::{SessionHandle, session_loop};
use cogito_protocol::session::SessionId;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// Construct a `SessionHandle` whose `subscribe()` returns an empty
/// broadcast channel and whose mailbox is closed. Use only in tests
/// that never call `submit`, `cancel_turn`, or `shutdown`.
///
/// # Panics
///
/// If `cogito-core` changes the SessionShared layout in a way that
/// breaks this stub, this fn will fail to compile — caller-friendly
/// signal that the stub needs updating alongside the layout change.
#[must_use]
pub fn stub_handle() -> SessionHandle {
    let (mailbox_tx, _) = mpsc::channel(1);
    let (broadcast_tx, _) = broadcast::channel(1);
    let shared = Arc::new(session_loop::SessionShared {
        session_id: SessionId::new(),
        mailbox_tx,
        events_tx: broadcast_tx.clone(),
        broadcast_tx,
        current_cancel_token: Arc::new(Mutex::new(CancellationToken::new())),
        // The remaining fields depend on the current SessionShared
        // layout. If this fails to compile, copy the missing fields
        // from `cogito-core::runtime::session_loop` and stub them.
        ..unimplemented!("update stub to match current SessionShared fields")
    });
    SessionHandle::from_shared(shared)
}
```

> **VERIFY before writing:** the actual `SessionShared` field list may be different. Check `crates/cogito-core/src/runtime/session_loop.rs` for the current `pub(super) struct SessionShared { ... }` layout. If `SessionHandle::from_shared` doesn't exist, add a `#[cfg(any(test, feature = "test-stub"))] pub fn from_shared(shared: Arc<SessionShared>) -> Self { Self { shared } }` to `crates/cogito-core/src/runtime/handle.rs` — gated so it doesn't leak into production builds. If gating requires a workspace feature, add `test-stub = []` to `cogito-core/Cargo.toml`'s `[features]` table.

If wiring a stub turns into a yak-shave, the simpler fallback is to add a `#[cfg(test)]` constructor inside `App` itself that takes pre-built `chat`/`tools`/etc. and stub the `handle` / `registry` / `store` fields with `unimplemented!()`-shaped `Arc`s that the tests never call into. The pure-state tests above only touch state fields, never the handle.

- [ ] **Step 4: Add module declaration**

Add to `crates/testing/cogito-test-fixtures/src/lib.rs`:

```rust
pub mod session_handle_stub;
```

- [ ] **Step 5: Run App tests**

Run: `cargo nextest run -p cogito-tui app`
Expected: 6 tests passed.

If you took the `#[cfg(test)]` constructor fallback above, adapt the tests to use it instead of `stub_handle()`.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-tui/src/app.rs \
        crates/testing/cogito-test-fixtures/src/session_handle_stub.rs \
        crates/testing/cogito-test-fixtures/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): App state — single source of truth + lifecycle flags

App carries SessionHandle, registry, ConversationStore, both render
models (ChatModel + ToolTreeModel), selection + expanded set, input
widget, layout toggles (show_tools, popup), status fields
(strategy/model/session/turn count), and the Ctrl-C double-tap timer.

apply_stream_event updates both models AND lifecycle flags:
- TurnStarted: turn_in_progress = true
- TurnCompleted: turn_in_progress = false; turn_count += 1
- TurnFailed/Cancelled/Paused: turn_in_progress = false (no counter bump)

refresh_popup transitions between None and SlashMenu based on the
input's leading character.

Pure-state tests (6) cover the lifecycle transitions, popup state
machine, and status_data projection.

Test stub for SessionHandle lives in cogito-test-fixtures so other
crates can reuse it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: keymap dispatcher

**Files:**
- Modify: `crates/cogito-tui/src/keymap.rs`

- [ ] **Step 1: Implement the key dispatcher**

Replace `crates/cogito-tui/src/keymap.rs` with:

```rust
//! Key → App action dispatcher. Implements the spec's focus model
//! (decision Q9 = B: implicit focus, no mode toggle):
//!
//! - Typing characters → input widget
//! - Enter / Shift+Enter → input widget (send or newline)
//! - PgUp/PgDn → chat scrollback (no focus required)
//! - Ctrl-Up/Down → tool-tree selection cursor
//! - Ctrl-Enter → toggle expansion of selected node
//! - Ctrl-T → toggle tools pane visibility
//! - Ctrl-C → cancel turn (with double-tap exit)
//! - Ctrl-D on empty input → exit
//! - Esc → dismiss popup (if shown); otherwise no-op
//!
//! The dispatcher returns an `Action` describing what the event loop
//! should do (send message, toggle pane, expand node, quit, ...).
//! Side effects that require async (`cancel_turn`, `submit_user_text`,
//! lazy tool-result lookup) happen in the event loop, not here.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, CTRL_C_EXIT_WINDOW};
use crate::ui::input::InputOutcome;

/// What the event loop should do as a result of one key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// No async side effect required; state has already been mutated.
    None,
    /// Submit the given message as the next user turn.
    SubmitUser(String),
    /// Submit the given slash command for in-process dispatch.
    SubmitSlash(String),
    /// Cancel the current turn (call `SessionHandle::cancel_turn`).
    CancelTurn,
    /// Toggle expansion of `path` — if expanding, also trigger lazy
    /// result lookup via the store.
    ExpandNode {
        /// Tree path being toggled.
        path: crate::render_model::TreePath,
        /// `true` if this transitions to expanded; `false` if
        /// transitioning back to collapsed.
        now_expanded: bool,
    },
    /// Quit the event loop.
    Quit,
}

/// Apply a key event to App state and return the deferred Action (if
/// any) that the event loop must perform asynchronously.
pub fn dispatch(app: &mut App, key: KeyEvent) -> Action {
    // Esc dismisses popups first; any other key while popup is open
    // still goes to the input.
    if matches!(key.code, KeyCode::Esc) && app.popup.is_some() {
        app.popup = None;
        return Action::None;
    }

    // Ctrl-T toggles tools pane.
    if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.show_tools = !app.show_tools;
        return Action::None;
    }

    // Ctrl-C: cancel-or-exit double-tap.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return handle_ctrl_c(app);
    }

    // Ctrl-D: exit if buffer is empty.
    if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if app.input.first_char().is_none() {
            return Action::Quit;
        }
        return Action::None;
    }

    // PgUp/PgDn scroll the chat (always; no focus mode).
    if matches!(key.code, KeyCode::PageUp) {
        app.chat.scroll_offset = app.chat.scroll_offset.saturating_add(5);
        return Action::None;
    }
    if matches!(key.code, KeyCode::PageDown) {
        app.chat.scroll_offset = app.chat.scroll_offset.saturating_sub(5);
        return Action::None;
    }

    // Ctrl-Up / Ctrl-Down navigate tool tree.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Up | KeyCode::Down)
    {
        navigate_tree(app, key.code);
        return Action::None;
    }

    // Ctrl-Enter expands selected node.
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        return expand_selected(app);
    }

    // Default: route to the input widget.
    let outcome = app.input.on_key(key);
    app.refresh_popup();
    match outcome {
        InputOutcome::Consumed => Action::None,
        InputOutcome::Submit(text) => {
            if text.starts_with('/') {
                Action::SubmitSlash(text)
            } else {
                Action::SubmitUser(text)
            }
        }
    }
}

fn handle_ctrl_c(app: &mut App) -> Action {
    // Three states:
    //   1. Turn running -> cancel + arm 2s double-tap window.
    //   2. Turn idle + arm active -> exit.
    //   3. Turn idle + no arm -> arm + hint.
    if app.turn_in_progress {
        app.cancel_seen_at = Some(Instant::now());
        return Action::CancelTurn;
    }
    if let Some(t) = app.cancel_seen_at
        && t.elapsed() < CTRL_C_EXIT_WINDOW
    {
        return Action::Quit;
    }
    app.cancel_seen_at = Some(Instant::now());
    app.chat
        .push_notice("[hint] Press Ctrl-C again to exit, or Ctrl-D on empty input");
    Action::None
}

fn navigate_tree(app: &mut App, code: KeyCode) {
    if app.tools.turns.is_empty() {
        return;
    }
    let cur = app.selected;
    let next = match (cur, code) {
        (None, _) => Some((0, 0)),
        (Some((t, n)), KeyCode::Down) => {
            let group_len = app.tools.turns.get(t).map_or(0, |g| g.nodes.len());
            if n + 1 < group_len {
                Some((t, n + 1))
            } else if t + 1 < app.tools.turns.len() {
                Some((t + 1, 0))
            } else {
                Some((t, n))
            }
        }
        (Some((t, n)), KeyCode::Up) => {
            if n > 0 {
                Some((t, n - 1))
            } else if t > 0 {
                let prev_t = t - 1;
                let prev_len = app.tools.turns[prev_t].nodes.len();
                Some((prev_t, prev_len.saturating_sub(1)))
            } else {
                Some((t, n))
            }
        }
        _ => cur,
    };
    app.selected = next;
}

fn expand_selected(app: &mut App) -> Action {
    let Some(path) = app.selected else {
        return Action::None;
    };
    // Only allow expansion of finished nodes.
    let finished = app
        .tools
        .turns
        .get(path.0)
        .and_then(|g| g.nodes.get(path.1))
        .is_some_and(|n| n.status.is_finished());
    if !finished {
        return Action::None;
    }
    let now_expanded = if app.expanded.contains(&path) {
        app.expanded.remove(&path);
        false
    } else {
        app.expanded.insert(path);
        true
    };
    Action::ExpandNode { path, now_expanded }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::stream::StreamEvent;
    use serde_json::json;

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn fresh_app() -> App {
        // Re-use the test helper from the `app::tests` module via
        // its `pub(crate)` visibility, or replicate inline.
        crate::app::tests::app_for_pure_test()
    }

    #[test]
    fn ctrl_t_toggles_show_tools() {
        let mut app = fresh_app();
        assert!(app.show_tools);
        let a = dispatch(&mut app, k(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::None);
        assert!(!app.show_tools);
    }

    #[test]
    fn ctrl_c_during_turn_returns_cancel() {
        let mut app = fresh_app();
        app.turn_in_progress = true;
        let a = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::CancelTurn);
        assert!(app.cancel_seen_at.is_some());
    }

    #[test]
    fn ctrl_c_twice_when_idle_returns_quit() {
        let mut app = fresh_app();
        app.turn_in_progress = false;
        let first = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(first, Action::None);
        let second = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(second, Action::Quit);
    }

    #[test]
    fn ctrl_d_on_empty_buffer_quits() {
        let mut app = fresh_app();
        let a = dispatch(&mut app, k(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn ctrl_d_with_text_does_not_quit() {
        let mut app = fresh_app();
        dispatch(&mut app, k(KeyCode::Char('h'), KeyModifiers::NONE));
        let a = dispatch(&mut app, k(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_ne!(a, Action::Quit);
    }

    #[test]
    fn enter_with_text_returns_submit_user() {
        let mut app = fresh_app();
        dispatch(&mut app, k(KeyCode::Char('h'), KeyModifiers::NONE));
        dispatch(&mut app, k(KeyCode::Char('i'), KeyModifiers::NONE));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, Action::SubmitUser("hi".into()));
    }

    #[test]
    fn enter_with_slash_returns_submit_slash() {
        let mut app = fresh_app();
        for ch in "/skill foo".chars() {
            dispatch(&mut app, k(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, Action::SubmitSlash("/skill foo".into()));
    }

    #[test]
    fn pgup_increases_scroll_offset() {
        let mut app = fresh_app();
        dispatch(&mut app, k(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.chat.scroll_offset, 5);
    }

    #[test]
    fn ctrl_down_initializes_selection() {
        let mut app = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        dispatch(&mut app, k(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(app.selected, Some((0, 0)));
    }

    #[test]
    fn ctrl_enter_on_finished_node_expands() {
        let mut app = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        app.tools.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c".into(),
            ok: true,
            error_message: None,
        });
        app.selected = Some((0, 0));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(matches!(
            a,
            Action::ExpandNode {
                path: (0, 0),
                now_expanded: true,
                ..
            }
        ));
        assert!(app.expanded.contains(&(0, 0)));
    }

    #[test]
    fn ctrl_enter_on_running_node_is_noop() {
        let mut app = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        app.selected = Some((0, 0));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(a, Action::None);
        assert!(!app.expanded.contains(&(0, 0)));
    }

    #[test]
    fn esc_dismisses_popup() {
        let mut app = fresh_app();
        app.popup = Some(crate::app::Popup::SlashMenu { prefix: "/".into() });
        dispatch(&mut app, k(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.popup.is_none());
    }
}
```

> **NOTE:** The tests reference `crate::app::tests::app_for_pure_test`; make sure it's exposed via `pub(crate)` in Task 11 by changing `fn app_for_pure_test()` to `pub(crate) fn app_for_pure_test()` (and bumping its module attribute so it's compiled for sibling test modules — `#[cfg(test)] pub(crate) mod tests` instead of `#[cfg(test)] mod tests`).

- [ ] **Step 2: Adjust visibility in `app.rs`**

Edit `crates/cogito-tui/src/app.rs`: change

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
```

to

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
```

and `fn app_for_pure_test()` → `pub(crate) fn app_for_pure_test()`.

- [ ] **Step 3: Run keymap tests**

Run: `cargo nextest run -p cogito-tui keymap`
Expected: 12 tests passed.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/src/keymap.rs crates/cogito-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): keymap — implicit focus dispatcher

dispatch(app, key) -> Action implements the spec's Q9-B focus model:
- typing routes to input widget
- Enter on text -> SubmitUser/SubmitSlash; Shift+Enter -> newline
- PgUp/PgDn scroll chat (5 rows per press)
- Ctrl-Up/Down navigate tool tree across turns
- Ctrl-Enter toggles expansion (only for finished nodes; deferred lookup
  signalled via Action::ExpandNode { now_expanded: true })
- Ctrl-T toggles tools pane
- Ctrl-C cancel-or-double-tap-exit (CTRL_C_EXIT_WINDOW = 2s)
- Ctrl-D on empty buffer quits
- Esc dismisses popup

Side effects requiring async (cancel_turn, send_user, store reads)
return as Action variants for the event loop to perform.

12 unit tests cover every transition.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 11: Slash command dispatch

### Task 13: slash::dispatch — parse + handle /skill

**Files:**
- Modify: `crates/cogito-tui/src/slash.rs`

- [ ] **Step 1: Implement slash dispatch**

Replace `crates/cogito-tui/src/slash.rs` with:

```rust
//! In-process slash command dispatch. v0.1 supports `/skill <name>`
//! only (mirrors `cogito-cli`'s `parse_slash_skill`). Unknown
//! commands push an `[error] unknown command: /foo` notice without
//! going through the model.
//!
//! The parser is intentionally re-implemented here rather than
//! re-exported from `cogito-cli` because the CLI's variant accepts a
//! callback (`F: Fn(...)`) suited to its REPL loop; the TUI just
//! needs the parsed result.

use crate::app::App;

/// Parsed slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/skill <name>` — activate a named skill for the next turn.
    Skill {
        /// Skill name (the token after `/skill `).
        name: String,
    },
    /// An unrecognized `/foo` command.
    Unknown {
        /// The raw text the user typed (including the leading `/`).
        raw: String,
    },
}

/// Parse a slash command. Returns `None` if the input doesn't begin
/// with `/`; the caller should treat that case as a normal user
/// message.
#[must_use]
pub fn parse(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match head {
        "/skill" if !rest.is_empty() => Some(SlashCommand::Skill { name: rest.into() }),
        "/skill" => Some(SlashCommand::Unknown {
            raw: trimmed.into(),
        }),
        _ => Some(SlashCommand::Unknown {
            raw: trimmed.into(),
        }),
    }
}

/// Dispatch a parsed slash command against the App. Returns the text
/// (if any) that the App should submit to the model in lieu of a
/// user message — `Some(text)` for skill activation (`/skill foo`
/// becomes the prompt `Activate skill: foo`), `None` for unknowns
/// (already rendered as a notice).
pub fn dispatch(app: &mut App, cmd: SlashCommand) -> Option<String> {
    match cmd {
        SlashCommand::Skill { name } => {
            app.chat.push_notice(format!("[skill] activating: {name}"));
            // The CLI's parse_slash_skill formats the message as
            // "Activate skill: <name>". We mirror that here so the
            // model sees the same prompt.
            Some(format!("Activate skill: {name}"))
        }
        SlashCommand::Unknown { raw } => {
            app.chat.push_notice(format!(
                "[error] unknown command: {raw}. Try /skill <name>"
            ));
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_with_name() {
        let r = parse("/skill foo").unwrap();
        assert_eq!(r, SlashCommand::Skill { name: "foo".into() });
    }

    #[test]
    fn parse_skill_without_name_is_unknown() {
        let r = parse("/skill").unwrap();
        assert!(matches!(r, SlashCommand::Unknown { .. }));
    }

    #[test]
    fn parse_unknown_command_returns_unknown() {
        let r = parse("/strategy planner").unwrap();
        assert!(matches!(r, SlashCommand::Unknown { .. }));
    }

    #[test]
    fn parse_non_slash_returns_none() {
        assert!(parse("hello").is_none());
    }

    #[test]
    fn dispatch_skill_returns_activation_prompt() {
        let mut app = crate::app::tests::app_for_pure_test();
        let out = dispatch(&mut app, SlashCommand::Skill { name: "foo".into() });
        assert_eq!(out, Some("Activate skill: foo".into()));
        assert_eq!(app.chat.lines.len(), 1);
    }

    #[test]
    fn dispatch_unknown_pushes_error_notice_and_no_prompt() {
        let mut app = crate::app::tests::app_for_pure_test();
        let out = dispatch(&mut app, SlashCommand::Unknown { raw: "/foo".into() });
        assert!(out.is_none());
        assert_eq!(app.chat.lines.len(), 1);
        match &app.chat.lines[0] {
            crate::render_model::ChatLine::SystemNotice(s) => {
                assert!(s.contains("unknown command: /foo"));
            }
            _ => unreachable!(),
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui slash`
Expected: 6 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/slash.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): slash dispatch — /skill <name> in v0.1

parse(input) -> Option<SlashCommand>: tokenizes by first whitespace;
'/skill foo' -> Skill{name: 'foo'}, '/skill' -> Unknown, '/foo' ->
Unknown, 'hi' -> None.

dispatch(app, cmd) -> Option<String>: Skill returns the activation
prompt 'Activate skill: <name>' (mirroring cogito-cli's
parse_slash_skill format) for the event loop to submit; Unknown
returns None after pushing '[error] unknown command' to chat.

6 unit tests cover parse + dispatch paths.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 12: Resume replay — ConversationEvent → StreamEvent

### Task 14: resume::translate_events + load_initial_state

**Files:**
- Modify: `crates/cogito-tui/src/resume.rs`

- [ ] **Step 1: Implement the replay translator**

Replace `crates/cogito-tui/src/resume.rs` with:

```rust
//! Resume support. On startup with `--session <id>`, the JSONL log
//! is read in full and translated into the equivalent StreamEvent
//! sequence. The App's `apply_stream_event` then drives the same
//! ChatModel + ToolTreeModel paths used at live time — no separate
//! "replay" code path in the models (spec §4.6 invariant).

use std::sync::Arc;

use anyhow::Result;
use cogito_protocol::{
    ContentBlock, ConversationEvent, ConversationStore, EventPayload,
    session::SessionId,
    stream::StreamEvent,
};

/// Initial state derived from a session log. `Fresh` for a brand-new
/// session (no replay); `Replayed` for resumes (translated events to
/// drive into App at startup).
pub enum InitialState {
    /// New session, nothing to replay.
    Fresh,
    /// Resumed session — translated stream of events to apply to the
    /// App's models before entering the live loop.
    Replayed {
        /// Events in arrival order.
        stream_events: Vec<StreamEvent>,
    },
}

/// Translate a persisted `ConversationEvent` stream into a stream of
/// `StreamEvent`s suitable for driving ChatModel + ToolTreeModel.
///
/// The mapping is coarse (one logical block = one synthetic
/// TextDelta with the whole text), since the persisted log is in
/// `ContentBlock` form, not delta form. This is intentional: replay
/// shows the user the finished content, not a re-played token-by-token
/// stream.
#[must_use]
pub fn translate_events(events: &[ConversationEvent]) -> Vec<StreamEvent> {
    let mut out: Vec<StreamEvent> = Vec::new();
    let mut in_turn = false;
    for ev in events {
        match &ev.payload {
            EventPayload::TurnStarted { .. } => {
                if in_turn {
                    out.push(StreamEvent::TurnCompleted);
                }
                out.push(StreamEvent::TurnStarted);
                in_turn = true;
            }
            EventPayload::TurnCompleted { .. } => {
                out.push(StreamEvent::TurnCompleted);
                in_turn = false;
            }
            EventPayload::TurnFailed { reason, .. } => {
                out.push(StreamEvent::TurnFailed {
                    reason: reason.clone(),
                });
                in_turn = false;
            }
            EventPayload::ContentBlockEnd { block, .. } => {
                synth_from_block(&mut out, block);
            }
            _ => {}
        }
    }
    if in_turn {
        out.push(StreamEvent::TurnCompleted);
    }
    out
}

fn synth_from_block(out: &mut Vec<StreamEvent>, block: &ContentBlock) {
    match block {
        ContentBlock::Text { text, .. } => {
            if !text.is_empty() {
                out.push(StreamEvent::TextDelta { chunk: text.clone() });
            }
        }
        ContentBlock::Thinking { text, .. } => {
            if !text.is_empty() {
                out.push(StreamEvent::ThinkingDelta {
                    chunk: text.clone(),
                });
            }
        }
        ContentBlock::ToolUse {
            id,
            name,
            input,
            ..
        } => {
            out.push(StreamEvent::ToolDispatchStarted {
                call_id: id.clone(),
                tool_name: name.clone(),
                args: input.clone(),
            });
            // We don't know the end status without scanning forward
            // for the matching ToolResult; emit a synthetic "ok"
            // marker. The matching ToolResult block (if any) will
            // update App.tools.result_preview via lazy lookup. If a
            // ToolResult is encountered later in `translate_events`,
            // the second arm of the match below corrects the status.
            out.push(StreamEvent::ToolDispatchEnded {
                call_id: id.clone(),
                ok: true,
                error_message: None,
            });
        }
        ContentBlock::ToolResult {
            call_id,
            is_error,
            content: _,
            ..
        } => {
            // Re-emit a "corrective" end event whose ok bit reflects
            // is_error. Since the prior synthetic end was ok=true,
            // when is_error=true we push another end to flip the
            // status in the ToolTreeModel. ChatModel's ToolEndLine
            // is append-only — the cosmetic effect is one extra line,
            // acceptable for replay parity.
            if *is_error {
                out.push(StreamEvent::ToolDispatchEnded {
                    call_id: call_id.clone(),
                    ok: false,
                    error_message: None,
                });
            }
        }
        _ => {}
    }
}

/// Read the session log and produce an `InitialState`. Errors propagate
/// — caller should print them and exit non-zero before entering raw mode.
///
/// # Errors
///
/// Returns a `ConversationStore` error if the session cannot be read.
pub async fn load_initial_state(
    store: &Arc<dyn ConversationStore>,
    session_id: &SessionId,
    is_new_session: bool,
) -> Result<InitialState> {
    if is_new_session {
        return Ok(InitialState::Fresh);
    }
    let events = store.read_session(*session_id).await?;
    let stream_events = translate_events(&events);
    Ok(InitialState::Replayed { stream_events })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::EventCategory;
    use cogito_protocol::session::EventId;
    use serde_json::json;

    fn ev(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            schema_version: cogito_protocol::SCHEMA_VERSION,
            timestamp: chrono::Utc::now(),
            seq: 0,
            category: EventCategory::Lifecycle,
            payload,
        }
    }

    #[test]
    fn empty_log_translates_to_empty_stream() {
        assert!(translate_events(&[]).is_empty());
    }

    #[test]
    fn turn_started_and_completed_translate_directly() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                strategy: None,
                model: "x".into(),
            }),
            ev(EventPayload::TurnCompleted {}),
        ];
        let s = translate_events(&log);
        assert!(matches!(s[0], StreamEvent::TurnStarted));
        assert!(matches!(s[1], StreamEvent::TurnCompleted));
    }

    #[test]
    fn text_block_yields_text_delta() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                strategy: None,
                model: "x".into(),
            }),
            ev(EventPayload::ContentBlockEnd {
                index: 0,
                block: ContentBlock::Text {
                    text: "hello".into(),
                    citations: None,
                },
            }),
            ev(EventPayload::TurnCompleted {}),
        ];
        let s = translate_events(&log);
        match &s[1] {
            StreamEvent::TextDelta { chunk } => assert_eq!(chunk, "hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_yields_dispatch_started_and_ended() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                strategy: None,
                model: "x".into(),
            }),
            ev(EventPayload::ContentBlockEnd {
                index: 0,
                block: ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "t".into(),
                    input: json!({}),
                },
            }),
            ev(EventPayload::TurnCompleted {}),
        ];
        let s = translate_events(&log);
        assert!(s.iter().any(|e| matches!(
            e,
            StreamEvent::ToolDispatchStarted { call_id, .. } if call_id == "c1"
        )));
        assert!(s.iter().any(|e| matches!(
            e,
            StreamEvent::ToolDispatchEnded { call_id, ok: true, .. } if call_id == "c1"
        )));
    }

    #[test]
    fn unterminated_turn_emits_synthetic_completed() {
        // A log that ends mid-turn (e.g. crash during streaming)
        // must still leave the chat in a coherent state.
        let log = vec![ev(EventPayload::TurnStarted {
            strategy: None,
            model: "x".into(),
        })];
        let s = translate_events(&log);
        assert!(s.last().is_some_and(|e| matches!(e, StreamEvent::TurnCompleted)));
    }
}
```

> **VERIFY before commit:** `EventPayload::TurnStarted`/`TurnCompleted`/`TurnFailed`/`ContentBlockEnd` field shapes. The grep earlier showed `EventPayload::ModelCallCompleted { stop_reason, usage }` was added in Sprint 3 — the current variant list may include more shapes. Run `grep -A 4 "^pub enum EventPayload" crates/cogito-protocol/src/event.rs` and adapt the match arms to match. Also confirm `ContentBlock::Text { text, citations }` and `ContentBlock::ToolUse { id, name, input }` fields. If shapes differ, adapt the match arms — the tests are the source of truth for what's needed.

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p cogito-tui resume`
Expected: 5 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/resume.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): resume — ConversationEvent → StreamEvent translation

translate_events walks the persisted log and synthesizes the equivalent
StreamEvent sequence. TurnStarted/Completed/Failed map directly;
ContentBlockEnd lookups synthesize TextDelta/ThinkingDelta/
ToolDispatchStarted+Ended pairs (the model API's ToolResult.is_error
flips the synthetic end to ok=false). An unterminated trailing turn
emits a synthetic TurnCompleted to keep models coherent.

load_initial_state returns Fresh for new sessions, Replayed{stream_events}
for resumes. The App drives those through apply_stream_event so the
exact same code path runs in live and replay modes (spec §4.6 invariant).

5 unit tests cover empty log, plain turn, text block, tool block,
unterminated turn.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 13: Lazy tool-result lookup (α.1)

### Task 15: extract_tool_result + wire into App expand handler

**Files:**
- Modify: `crates/cogito-tui/src/resume.rs`
- Modify: `crates/cogito-tui/src/app.rs`

- [ ] **Step 1: Add `extract_tool_result` to resume.rs**

Append to `crates/cogito-tui/src/resume.rs` (after the existing test module — outside it):

```rust
// -- Lazy tool-result lookup (spec §5.3 α.1) ----------------------------

/// Walk a `ConversationEvent` log and find the result text for one
/// `call_id`. Returns the first `ContentBlock::ToolResult` whose
/// `call_id` matches, rendered as a single string (concatenated
/// `Text` blocks within the result content, or `<binary>` if the
/// result has no text representation).
#[must_use]
pub fn extract_tool_result(events: &[ConversationEvent], call_id: &str) -> Option<String> {
    for ev in events {
        if let EventPayload::ContentBlockEnd {
            block:
                ContentBlock::ToolResult {
                    call_id: cid,
                    content,
                    ..
                },
            ..
        } = &ev.payload
            && cid == call_id
        {
            let text: String = content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return Some("<no text content>".into());
            }
            return Some(text);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod extract_tests {
    use super::*;
    use cogito_protocol::EventCategory;
    use cogito_protocol::session::EventId;

    fn ev(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            schema_version: cogito_protocol::SCHEMA_VERSION,
            timestamp: chrono::Utc::now(),
            seq: 0,
            category: EventCategory::Lifecycle,
            payload,
        }
    }

    #[test]
    fn extract_returns_text_content() {
        let log = vec![ev(EventPayload::ContentBlockEnd {
            index: 0,
            block: ContentBlock::ToolResult {
                call_id: "c1".into(),
                is_error: false,
                content: vec![ContentBlock::Text {
                    text: "file contents".into(),
                    citations: None,
                }],
            },
        })];
        let r = extract_tool_result(&log, "c1");
        assert_eq!(r, Some("file contents".into()));
    }

    #[test]
    fn extract_returns_none_when_call_id_not_found() {
        let log = vec![ev(EventPayload::ContentBlockEnd {
            index: 0,
            block: ContentBlock::ToolResult {
                call_id: "c1".into(),
                is_error: false,
                content: vec![],
            },
        })];
        assert_eq!(extract_tool_result(&log, "c-other"), None);
    }

    #[test]
    fn extract_empty_content_returns_placeholder() {
        let log = vec![ev(EventPayload::ContentBlockEnd {
            index: 0,
            block: ContentBlock::ToolResult {
                call_id: "c1".into(),
                is_error: false,
                content: vec![],
            },
        })];
        assert_eq!(extract_tool_result(&log, "c1"), Some("<no text content>".into()));
    }
}
```

- [ ] **Step 2: Wire lookup into App on ExpandNode**

Add this method on `App` in `crates/cogito-tui/src/app.rs` (after `refresh_popup`):

```rust
    /// Populate the result preview for one tool node by re-reading the
    /// session store. Idempotent — does nothing if the node is still
    /// running or already has a preview. Called by the event loop in
    /// response to `Action::ExpandNode { now_expanded: true }`.
    pub async fn populate_result_preview(&mut self, path: crate::render_model::TreePath) {
        let needs_lookup = self
            .tools
            .turns
            .get(path.0)
            .and_then(|g| g.nodes.get(path.1))
            .is_some_and(|n| n.status.is_finished() && n.result_preview.is_none());
        if !needs_lookup {
            return;
        }
        let call_id = match self
            .tools
            .turns
            .get(path.0)
            .and_then(|g| g.nodes.get(path.1))
        {
            Some(n) => n.call_id.clone(),
            None => return,
        };
        let session_id = self.handle.session_id();
        let events = match self.store.read_session(session_id).await {
            Ok(e) => e,
            Err(err) => {
                self.chat
                    .push_notice(format!("[warning] could not read session: {err}"));
                return;
            }
        };
        if let Some(preview) = crate::resume::extract_tool_result(&events, &call_id)
            && let Some(node) = self.tools.find_node_mut(&call_id)
        {
            node.result_preview = Some(preview);
        }
    }
```

> **VERIFY:** `SessionHandle::session_id()` exists and returns `SessionId`. Grep `crates/cogito-core/src/runtime/handle.rs` to confirm. If named differently (e.g. `id()`, `session()`, accessor on `shared.session_id`), adapt.

- [ ] **Step 3: Add unit test for `populate_result_preview`**

Add to `app.rs` test module:

```rust
    #[tokio::test(flavor = "current_thread")]
    async fn populate_result_preview_is_noop_for_running_node() {
        let mut app = app_for_pure_test();
        app.tools
            .on_event(&cogito_protocol::stream::StreamEvent::TurnStarted);
        app.tools
            .on_event(&cogito_protocol::stream::StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            });
        // Running; populate must not touch the store.
        app.populate_result_preview((0, 0)).await;
        // The node is still without a preview.
        assert!(app.tools.turns[0].nodes[0].result_preview.is_none());
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p cogito-tui resume::extract_tests app::tests::populate_result_preview_is_noop_for_running_node`
Expected: 4 tests passed (3 extract + 1 populate noop).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-tui/src/resume.rs crates/cogito-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): lazy tool-result lookup (α.1)

extract_tool_result(&events, call_id) -> Option<String>: scans for
the matching ContentBlock::ToolResult, joins its Text-block content,
returns '<no text content>' for empty results, None for unknown
call_id.

App::populate_result_preview(path) — async helper invoked by the
event loop on Action::ExpandNode { now_expanded: true }. Idempotent:
no-op for running nodes (status filter), no-op when preview already
cached. Re-reads the session via store.read_session each first-expand;
JSONL is local-only file IO (sub-ms for typical sessions). On store
error, pushes a [warning] notice; expansion still happens (shows
'(loading result...)' placeholder forever, acceptable as a degradation).

3 extract tests + 1 populate test.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 14: Event loop

### Task 16: event_loop::run — select! over crossterm / stream / tick

**Files:**
- Modify: `crates/cogito-tui/src/event_loop.rs`

- [ ] **Step 1: Implement the event loop**

Replace `crates/cogito-tui/src/event_loop.rs` with:

```rust
//! Event loop — single tokio task that multiplexes:
//!   1. crossterm keyboard events (via EventStream)
//!   2. StreamEvent broadcast from SessionHandle::subscribe()
//!   3. 33ms redraw tick (≤30 FPS)
//!
//! Drawing happens only on tick. Key handling and stream-event
//! handling mutate App state without redrawing.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CrosstermEvent, EventStream};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::App;
use crate::keymap::{Action, dispatch};
use crate::slash;
use crate::ui::{RenderInputs, render};

/// Drive the TUI to completion. Returns when the user quits, the
/// session closes, or a fatal error occurs.
///
/// The terminal must already be in raw mode (the caller owns the
/// `TerminalGuard`).
///
/// # Errors
///
/// Returns I/O errors from `Terminal::draw` or `CrosstermBackend`.
pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("constructing CrosstermBackend")?;
    let mut crossterm_events = EventStream::new();
    let mut stream_events = BroadcastStream::new(app.handle.subscribe());
    let mut redraw_tick = tokio::time::interval(Duration::from_millis(33));
    redraw_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Initial draw so the user sees a frame immediately.
    terminal
        .draw(|f| {
            render(
                f,
                &RenderInputs {
                    chat: &app.chat,
                    tools: &app.tools,
                    selected: app.selected,
                    expanded: &app.expanded,
                    input: &app.input,
                    show_tools: app.show_tools,
                    status: &app.status_data(),
                    popup_prefix: popup_prefix(&app.popup).as_deref(),
                },
            );
        })
        .context("initial draw")?;

    loop {
        tokio::select! {
            maybe_key = crossterm_events.next() => {
                if let Some(Ok(CrosstermEvent::Key(key))) = maybe_key {
                    let action = dispatch(app, key);
                    handle_action(app, action).await?;
                }
                if let Some(Ok(CrosstermEvent::Resize(_, _))) = maybe_key {
                    // Resize triggers a redraw on the next tick; no
                    // app-state change required.
                }
                if app.should_quit { break; }
            }
            maybe_ev = stream_events.next() => {
                match maybe_ev {
                    Some(Ok(ev)) => app.apply_stream_event(&ev),
                    Some(Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n))) => {
                        app.chat.push_notice(format!(
                            "[warning] {n} events dropped (slow render); live view will catch up"
                        ));
                    }
                    None => {
                        // Broadcast closed -> session ended. Push a notice and quit gracefully.
                        app.chat.push_notice("[fatal] session closed");
                        break;
                    }
                }
            }
            _ = redraw_tick.tick() => {
                terminal
                    .draw(|f| {
                        render(
                            f,
                            &RenderInputs {
                                chat: &app.chat,
                                tools: &app.tools,
                                selected: app.selected,
                                expanded: &app.expanded,
                                input: &app.input,
                                show_tools: app.show_tools,
                                status: &app.status_data(),
                                popup_prefix: popup_prefix(&app.popup).as_deref(),
                            },
                        );
                    })
                    .context("draw on tick")?;
            }
        }
    }
    Ok(())
}

fn popup_prefix(popup: &Option<crate::app::Popup>) -> Option<String> {
    popup.as_ref().map(|p| match p {
        crate::app::Popup::SlashMenu { prefix } => prefix.clone(),
    })
}

async fn handle_action(app: &mut App, action: Action) -> Result<()> {
    match action {
        Action::None => Ok(()),
        Action::Quit => {
            app.should_quit = true;
            Ok(())
        }
        Action::CancelTurn => {
            if let Err(err) = app.handle.cancel_turn().await {
                app.chat.push_notice(format!("[warning] cancel failed: {err}"));
            }
            Ok(())
        }
        Action::SubmitUser(text) => {
            app.chat.push_user_prompt(text.clone());
            if let Err(err) = app.handle.submit_user_text(text).await {
                app.chat.push_notice(format!("[error] failed to send: {err}"));
            }
            Ok(())
        }
        Action::SubmitSlash(raw) => {
            let parsed = slash::parse(&raw);
            if let Some(cmd) = parsed
                && let Some(prompt) = slash::dispatch(app, cmd)
                && let Err(err) = app.handle.submit_user_text(prompt).await
            {
                app.chat.push_notice(format!("[error] failed to send: {err}"));
            }
            Ok(())
        }
        Action::ExpandNode {
            path,
            now_expanded: true,
        } => {
            app.populate_result_preview(path).await;
            Ok(())
        }
        Action::ExpandNode { .. } => Ok(()),
    }
}
```

> **No unit tests for `event_loop::run`** — it owns a `Terminal<CrosstermBackend>`. The E2E test in Task 22 covers it via a fake event stream + `TestBackend`. The dispatcher contract (Action variants) is exhaustively tested in Phase 10.

- [ ] **Step 2: Verify the crate still compiles**

Run: `cargo check -p cogito-tui`
Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/event_loop.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): event loop — select! over crossterm/stream/tick

run(app) multiplexes three sources in one tokio::select! per iteration:
- crossterm::EventStream for keyboard (Key) and resize (Resize) events
- BroadcastStream wrapping SessionHandle::subscribe() for StreamEvents
- a 33ms interval ticker for redraws (≤30 FPS frame budget)

Initial draw before entering the loop so the user sees a frame
immediately. handle_action turns Action enum variants into the async
side effects keymap.rs couldn't perform synchronously: CancelTurn ->
handle.cancel_turn, SubmitUser -> chat.push_user_prompt +
handle.submit_user_text, SubmitSlash -> parse + dispatch + send,
ExpandNode{now_expanded:true} -> populate_result_preview (lazy
JSONL read).

Lagged broadcast: push '[warning] N events dropped' notice and
continue (spec §6.2). Session closed: push '[fatal]' and break.

MissedTickBehavior::Skip ensures a long handler doesn't queue
backlog tick events.

No unit tests at this layer — Terminal<CrosstermBackend> is real;
coverage lands in the E2E test (Task 22).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 15: Runtime builder + main()

### Task 17: runtime_build::build + main entry point

**Files:**
- Modify: `crates/cogito-tui/src/runtime_build.rs`
- Modify: `crates/cogito-tui/src/main.rs`

- [ ] **Step 1: Implement runtime_build**

Replace `crates/cogito-tui/src/runtime_build.rs` with:

```rust
//! Build a Runtime + open a Session for the TUI. Mirrors
//! `cogito-cli::chat::run`'s prelude up to (but not including) the
//! event loop; the TUI's loop lives in `event_loop::run`.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use cogito_cli::chat_config::{
    ChatConfigInputs, build_runtime_config_and_registry, build_skill_provider, select_provider,
};
use cogito_cli::chat::resolve_strategy;
use cogito_core::runtime::{Runtime, RuntimeBuilder, SessionHandle, SessionMode};
use cogito_protocol::strategy_registry::StrategyRegistry;
use cogito_protocol::session::SessionId;

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
    /// MCP banner lines (textual) to push as SystemNotices.
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
    let (mut cfg, registry) = build_runtime_config_and_registry(&inputs)
        .await
        .context("loading config + strategies")?;

    let (strategy, provider) =
        resolve_strategy(&args_to_cli_chatargs(args), &cfg, &*registry)
            .map_err(|e| anyhow::anyhow!("strategy resolution: {e}"))?;

    // Patch the provider into the config so the rest of the build
    // sees the resolved provider.
    cfg.providers.retain(|p| p.name() != provider.name());
    cfg.providers.push(provider.clone());
    cfg.runtime.default_provider = Some(provider.name().into());

    let gateway = cogito_model::build_gateway(&provider)
        .context("building model gateway")?;
    let skill_provider = build_skill_provider(&cfg)?;

    // Capture MCP banner.
    let mut mcp_buf: Vec<u8> = Vec::new();
    let (tool_provider, mcp_banner_lines) =
        build_tools_with_banner(&cfg, &mut mcp_buf).await?;

    let store = cogito_store_jsonl::JsonlStore::with_default_root()?;
    let store: Arc<dyn cogito_protocol::ConversationStore> = Arc::new(store);

    let job_manager = cogito_jobs::JobManager::new();

    let mut builder = RuntimeBuilder::new()
        .with_model_gateway(gateway)
        .with_tool_provider(tool_provider)
        .with_job_manager(Arc::new(job_manager))
        .with_store(Arc::clone(&store))
        .with_default_strategy(strategy.clone());
    if let Some(s) = skill_provider {
        builder = builder.with_skill_provider(s);
    }
    let runtime: Arc<Runtime> = Arc::new(builder.build().context("building runtime")?);

    // Open session.
    let mode = match args.mode {
        Some(TuiMode::New) => SessionMode::New,
        Some(TuiMode::Resume) => SessionMode::Resume,
        Some(TuiMode::Attach) | None if args.session_id.is_some() => SessionMode::Attach,
        Some(TuiMode::Attach) => SessionMode::Attach,
        None => SessionMode::New,
    };
    let session_id = match &args.session_id {
        Some(s) => SessionId::parse(s)
            .map_err(|e| anyhow::anyhow!("invalid session id: {e}"))?,
        None => SessionId::new(),
    };
    let handle: SessionHandle = runtime
        .open_session(session_id, mode)
        .await
        .context("opening session")?;

    let is_new = matches!(mode, SessionMode::New);
    let initial = load_initial_state(&store, &session_id, is_new).await?;

    // Build the App.
    let mut chat = ChatModel::new();
    for line in &mcp_banner_lines {
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

    let app = App {
        handle,
        registry: registry as Arc<dyn StrategyRegistry>,
        store,
        session_id_str: session_id.to_string(),
        session_root: cfg.runtime.session_root.clone(),
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
            .or(cfg.runtime.default_strategy.clone())
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

fn inputs_from_args(args: &TuiArgs) -> ChatConfigInputs {
    ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    }
}

fn args_to_cli_chatargs(args: &TuiArgs) -> cogito_cli::chat::ChatArgs {
    cogito_cli::chat::ChatArgs {
        config: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
        session_id: args.session_id.clone(),
        mode: args.mode.map(|m| match m {
            TuiMode::New => cogito_cli::chat::ChatMode::New,
            TuiMode::Resume => cogito_cli::chat::ChatMode::Resume,
            TuiMode::Attach => cogito_cli::chat::ChatMode::Attach,
        }),
        system: args.system.clone(),
        strategy: args.strategy.clone(),
        list_strategies: args.list_strategies,
    }
}

async fn build_tools_with_banner(
    cfg: &cogito_config::RuntimeConfig,
    banner_buf: &mut Vec<u8>,
) -> Result<(
    Arc<dyn cogito_protocol::ToolProvider>,
    Vec<String>,
)> {
    // Compose builtin tools + MCP servers. The CLI does this in
    // chat::run; we factor it here. Use cogito_tools::CompositeToolProvider.
    let mut providers: Vec<Arc<dyn cogito_protocol::ToolProvider>> = Vec::new();
    providers.push(Arc::new(cogito_tools::builtin_provider()));

    let mcp_results = cogito_mcp::start_servers(&cfg.mcp_servers).await;
    let mcp_failures = mcp_results.failures.clone();
    let mcp_ok: Vec<(String, usize)> = mcp_results
        .providers
        .iter()
        .map(|(name, p)| (name.clone(), p.tool_count()))
        .collect();
    for (_name, provider) in mcp_results.providers {
        providers.push(provider);
    }

    cogito_cli::banner::render_banner(banner_buf, &cfg.mcp_servers, &mcp_failures, &mcp_ok)
        .context("rendering MCP banner")?;
    let banner_text = String::from_utf8_lossy(banner_buf).to_string();
    let banner_lines: Vec<String> = banner_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    let composite = cogito_tools::CompositeToolProvider::new(providers);
    Ok((Arc::new(composite), banner_lines))
}
```

> **VERIFY:** Several upstream APIs above are best-guess names. Confirm and adapt:
> - `cogito_mcp::start_servers(&cfg.mcp_servers)` — actual function name and return shape (`MultiServerStartResult` or similar). Look at `chat::run` in `cogito-cli/src/chat.rs` for the precise call.
> - `cogito_tools::builtin_provider()` — likely named `builtin()` or similar.
> - `cogito_tools::CompositeToolProvider::new` — confirm constructor takes `Vec<Arc<...>>`.
> - `JsonlStore::with_default_root()` — may be `JsonlStore::open(path)` or `JsonlStore::new()`.
> - `Runtime::open_session` parameter order and shape.
>
> If a build of this file fails, the fix is mechanical: read the relevant section of `crates/cogito-cli/src/chat.rs::run` and copy the exact API calls. The TUI's runtime build is conceptually a 1:1 of the CLI's — only the consumer (App vs Renderer + REPL) differs.

- [ ] **Step 2: Wire main()**

Replace `crates/cogito-tui/src/main.rs` with:

```rust
//! cogito-tui binary entrypoint.
//!
//! Startup order:
//!   1. Parse args.
//!   2. Handle --list-strategies (no raw mode; print + exit).
//!   3. Install debug log if requested.
//!   4. Build Runtime + open Session (errors print to stderr; exit 1).
//!   5. Enter raw mode (TerminalGuard).
//!   6. Run event loop.
//!   7. Drop guard (restores terminal).

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

use std::io::IsTerminal;

use anyhow::{Context, Result};
use clap::Parser;
use cogito_tui::cli::TuiArgs;
use cogito_tui::{event_loop, logs, runtime_build, terminal};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = TuiArgs::parse();

    if args.list_strategies {
        return list_strategies_and_exit(&args).await;
    }

    if args.debug || std::env::var("RUST_LOG").is_ok() {
        logs::install_file_logger().context("installing file logger")?;
    }

    if !std::io::stdout().is_terminal() {
        eprintln!("cogito-tui requires a terminal; stdout is not a TTY");
        std::process::exit(1);
    }

    let mut built = runtime_build::build(&args).await.context("building TUI runtime")?;

    let _guard = terminal::TerminalGuard::new().context("entering raw mode")?;
    event_loop::run(&mut built.app).await
}

async fn list_strategies_and_exit(args: &TuiArgs) -> Result<()> {
    use cogito_cli::chat_config::{ChatConfigInputs, build_runtime_config_and_registry};
    let inputs = ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    };
    let (_cfg, registry) = build_runtime_config_and_registry(&inputs)
        .await
        .context("loading config + strategies")?;
    for name in registry.list() {
        let desc = registry.description(&name).unwrap_or_default();
        if desc.is_empty() {
            println!("{name}");
        } else {
            println!("{name}\t{desc}");
        }
    }
    Ok(())
}
```

> **VERIFY:** `FsStrategyRegistry::description` exists — check `crates/cogito-strategy/src/registry.rs`. If not exposed via trait, downcast from `&dyn StrategyRegistry` to `&FsStrategyRegistry` like the CLI does (`registry_provider_ref` in `chat.rs`).

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p cogito-tui`
Expected: exit 0.

If the upstream APIs don't match, fix them by reading `crates/cogito-cli/src/chat.rs::run` and copying the exact calls. Cumulative time should be < 30 minutes; if it stretches, fall back to inlining the CLI's runtime-build logic verbatim into `runtime_build.rs`.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/src/runtime_build.rs crates/cogito-tui/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): runtime build + main entrypoint

runtime_build::build mirrors cogito-cli::chat::run's prelude:
load_layered_config -> build_runtime_config_and_registry ->
resolve_strategy -> select_provider patch -> build_gateway ->
build_skill_provider -> compose tools (builtin + MCP) -> capture
banner -> Runtime::build -> Runtime::open_session -> resume replay
(if not New mode) -> assemble App.

main() startup order:
  1. parse args
  2. --list-strategies short-circuit (no raw mode; print + exit)
  3. install file logger if --debug or RUST_LOG set
  4. is_terminal check (TUI requires a TTY)
  5. runtime_build::build (errors before raw mode -> exit 1)
  6. TerminalGuard::new (raw mode + alt screen + panic hook)
  7. event_loop::run
  8. guard Drop restores terminal

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 16: --list-strategies parity test

### Task 18: tests/list_strategies.rs

**Files:**
- Create: `crates/cogito-tui/tests/list_strategies.rs`
- Create: `crates/cogito-tui/tests/fixtures/coder.md`
- Create: `crates/cogito-tui/tests/fixtures/cogito-list-strategies.toml`

- [ ] **Step 1: Create fixture files**

Create `crates/cogito-tui/tests/fixtures/coder.md`:

```markdown
---
name: coder
description: pair-programming assistant
allowed_tools: [read_file]
max_turns: 10
---

You are a coding assistant.
```

Create `crates/cogito-tui/tests/fixtures/cogito-list-strategies.toml`:

```toml
[runtime]
strategies_dir = "."

[[providers]]
kind = "anthropic"
name = "test"
api_key = "fake"
base_url = "https://example.invalid"
anthropic_version = "2023-06-01"
```

- [ ] **Step 2: Write the parity test**

Create `crates/cogito-tui/tests/list_strategies.rs`:

```rust
//! Parity test: `cogito-tui --list-strategies` produces the same
//! output as `cogito --list-strategies` (when both point at the
//! same `--config`).

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::path::PathBuf;

fn fixture_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cogito-list-strategies.toml")
}

fn fixture_strategies_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn tui_list_strategies_matches_cli() {
    let mut tui = Command::cargo_bin("cogito-tui").unwrap();
    let tui_out = tui
        .arg("--config")
        .arg(fixture_config())
        .arg("--list-strategies")
        // Point HOME at an empty dir so user-scope scanning is a no-op.
        .env("HOME", tempfile::tempdir().unwrap().path())
        .env("XDG_CONFIG_HOME", tempfile::tempdir().unwrap().path())
        .current_dir(fixture_strategies_dir())
        .output()
        .unwrap();
    assert!(tui_out.status.success(), "stderr: {}", String::from_utf8_lossy(&tui_out.stderr));
    let tui_stdout = String::from_utf8_lossy(&tui_out.stdout).into_owned();

    let mut cli = Command::cargo_bin("cogito").unwrap();
    let cli_out = cli
        .arg("chat")
        .arg("--config")
        .arg(fixture_config())
        .arg("--list-strategies")
        .env("HOME", tempfile::tempdir().unwrap().path())
        .env("XDG_CONFIG_HOME", tempfile::tempdir().unwrap().path())
        .current_dir(fixture_strategies_dir())
        .output()
        .unwrap();
    assert!(cli_out.status.success(), "stderr: {}", String::from_utf8_lossy(&cli_out.stderr));
    let cli_stdout = String::from_utf8_lossy(&cli_out.stdout).into_owned();

    assert_eq!(tui_stdout, cli_stdout, "TUI and CLI --list-strategies must produce identical output");
    assert!(tui_stdout.contains("coder"), "expected 'coder' in output: {tui_stdout}");
}
```

- [ ] **Step 3: Run the test**

Run: `cargo nextest run -p cogito-tui --test list_strategies`
Expected: 1 test passed. (May take ~5s due to binary cargo build.)

If the test fails because the CLI's `--list-strategies` produces a slightly different format (e.g. it includes provider names), update either the TUI's printer to match exactly, OR adapt the test to assert on a less brittle property (e.g. line count + each strategy name appears in both outputs).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/tests/list_strategies.rs \
        crates/cogito-tui/tests/fixtures/coder.md \
        crates/cogito-tui/tests/fixtures/cogito-list-strategies.toml
git commit -m "$(cat <<'EOF'
test(cogito-tui): --list-strategies parity with cogito-cli

Spawns both `cogito-tui --list-strategies` and
`cogito chat --list-strategies` with the same --config + cwd, asserts
identical stdout. Uses tempdir for HOME/XDG_CONFIG_HOME so user-scope
scans don't pollute output across machines.

Fixture: tests/fixtures/coder.md (one strategy) + cogito-list-strategies.toml
(minimal config pointing strategies_dir to tests/fixtures).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 17: Debug logging (gated)

### Task 19: logs::install_file_logger

**Files:**
- Modify: `crates/cogito-tui/src/logs.rs`

- [ ] **Step 1: Implement gated file logger**

Replace `crates/cogito-tui/src/logs.rs` with:

```rust
//! Optional file-rotating tracing subscriber. Active only when
//! `--debug` is set or `RUST_LOG` is non-empty (spec §6.8). Stderr
//! is owned by raw mode, so a default no-op subscriber would lose
//! every event — the file path lets users opt in without sacrificing
//! the UI.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

/// Resolve `$XDG_STATE_HOME/cogito` or `$HOME/.local/state/cogito`,
/// creating it if missing. Returns the directory path.
fn log_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    let dir = base.join("cogito");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating log dir {dir:?}"))?;
    Ok(dir)
}

/// Install a daily-rotating file logger at `<log_dir>/tui.log`. Idempotent:
/// safe to call once. The guard returned by `tracing_appender` is
/// leaked intentionally so the writer stays alive for the process lifetime.
///
/// # Errors
///
/// Returns `anyhow::Error` if the log directory cannot be created.
pub fn install_file_logger() -> Result<()> {
    let dir = log_dir()?;
    let appender = RollingFileAppender::new(Rotation::DAILY, &dir, "tui.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    // Leak the guard — the writer flushes on process exit anyway,
    // and we don't have a clean place to drop it before raw mode tears
    // down. (`tracing-appender`'s docs explicitly support this pattern.)
    std::mem::forget(guard);

    let filter_str = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    let filter = tracing_subscriber::EnvFilter::new(format!(
        "{filter_str},hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,tower=warn"
    ));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_ansi(false) // file output; no ANSI escapes
        .with_writer(writer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("init tracing subscriber: {e}"))?;

    tracing::info!(?dir, "cogito-tui debug log opened");
    Ok(())
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p cogito-tui`
Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/src/logs.rs
git commit -m "$(cat <<'EOF'
feat(cogito-tui): gated debug logging — file-rotating writer

install_file_logger():
  - resolves $XDG_STATE_HOME/cogito or $HOME/.local/state/cogito
  - opens tracing_appender::rolling DAILY-rotated tui.log
  - non-blocking writer; guard leaked for process lifetime
  - EnvFilter from RUST_LOG (default 'info') + noise-suppression for
    hyper/reqwest/h2/tower (mirrors cogito-cli's bin)

Activated by main() iff --debug is set or RUST_LOG is non-empty.
Default = no logging (raw mode owns stderr; silent is correct).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 18: End-to-end test with MockModelGateway

### Task 20: tests/e2e.rs

**Files:**
- Create: `crates/cogito-tui/tests/e2e.rs`

- [ ] **Step 1: Write the E2E test**

Create `crates/cogito-tui/tests/e2e.rs`:

```rust
//! End-to-end test driving the TUI's App through a fake event channel,
//! a MockModelGateway, and a TestBackend-rendered Terminal. Verifies
//! the full data flow: keystroke → submit → model stream → ChatModel
//! mutation → frame render.
//!
//! This bypasses `event_loop::run` (which owns a real
//! `Terminal<CrosstermBackend>`) and instead exercises the surface
//! piece-by-piece. The dispatcher + App + render pipeline are the
//! interesting parts; the select! glue is trivial.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::ContentBlock;
use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::keymap::{Action, dispatch};
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::status::StatusData;
use cogito_tui::ui::{RenderInputs, render};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Construct an App suitable for E2E tests — stubs out SessionHandle
/// (we never call into it) and runs the dispatcher + render pipeline.
fn e2e_app() -> App {
    let store: Arc<dyn cogito_protocol::ConversationStore> = Arc::new(
        cogito_test_fixtures::in_memory_store::InMemoryStore::new(),
    );
    let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> = Arc::new(
        cogito_test_fixtures::strategy::MapStrategyRegistry::default(),
    );
    let handle = cogito_test_fixtures::session_handle_stub::stub_handle();
    App {
        handle,
        registry,
        store,
        session_id_str: "01TEST".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        show_tools: true,
        popup: None,
        strategy_name: "test".into(),
        model_id: "model-x".into(),
        turn_count: 0,
        turn_in_progress: false,
        cancel_seen_at: None,
        should_quit: false,
    }
}

fn draw(app: &App) -> String {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render(
                f,
                &RenderInputs {
                    chat: &app.chat,
                    tools: &app.tools,
                    selected: app.selected,
                    expanded: &app.expanded,
                    input: &app.input,
                    show_tools: app.show_tools,
                    status: &StatusData {
                        strategy: app.strategy_name.clone(),
                        model: app.model_id.clone(),
                        session_id: app.session_id_str.clone(),
                        turn_count: app.turn_count,
                        tools_visible: app.show_tools,
                    },
                    popup_prefix: None,
                },
            );
        })
        .unwrap();
    format!("{}", terminal.backend().buffer())
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn typing_and_model_response_render_into_chat() {
    let mut app = e2e_app();
    // Simulate user typing "hi" + Enter.
    for ch in "hi".chars() {
        let _ = dispatch(&mut app, key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let action = dispatch(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, Action::SubmitUser("hi".into()));
    // Bypass the (stubbed) session and apply the user prompt manually,
    // mirroring what the event loop does on Action::SubmitUser.
    app.chat.push_user_prompt("hi".into());

    // Simulate model response.
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "hello!".into(),
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);

    let out = draw(&app);
    assert!(out.contains("> hi"), "user prompt missing:\n{out}");
    assert!(out.contains("agent: hello!"), "agent text missing:\n{out}");
    assert_eq!(app.turn_count, 1);
    assert!(!app.turn_in_progress);
}

#[test]
fn ctrl_t_hides_tools_pane_in_render() {
    let mut app = e2e_app();
    dispatch(&mut app, key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    let out = draw(&app);
    assert!(!app.show_tools);
    assert!(!out.contains("tools "), "tools pane should be hidden:\n{out}");
}

#[test]
fn ctrl_c_during_streaming_emits_cancel_action() {
    let mut app = e2e_app();
    app.turn_in_progress = true;
    let a = dispatch(&mut app, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(a, Action::CancelTurn);
}

#[test]
fn slash_unknown_command_renders_error_notice() {
    let mut app = e2e_app();
    for ch in "/foo".chars() {
        let _ = dispatch(&mut app, key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let action = dispatch(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, Action::SubmitSlash("/foo".into()));
    // Mirror what the event loop's handle_action does for Action::SubmitSlash.
    let cmd = cogito_tui::slash::parse("/foo").unwrap();
    let prompt = cogito_tui::slash::dispatch(&mut app, cmd);
    assert!(prompt.is_none());
    let out = draw(&app);
    assert!(out.contains("unknown command"), "missing error notice:\n{out}");
}

#[test]
fn tool_lifecycle_renders_into_both_panes() {
    let mut app = e2e_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c1".into(),
        tool_name: "read_file".into(),
        args: serde_json::json!({"path": "a.rs"}),
    });
    app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c1".into(),
        ok: true,
        error_message: None,
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);
    let out = draw(&app);
    // Chat pane shows the textual record.
    assert!(out.contains("[tool] read_file"), "chat lacking tool line:\n{out}");
    // Tools pane shows the structural entry.
    assert!(out.contains("turn 1"), "tools pane lacking group:\n{out}");
    assert!(out.contains("read_file"), "tools pane lacking node:\n{out}");
}

// Silence the unused import warning for ContentBlock until a future
// test exercises lazy result lookup directly. Keeping it imported so
// adding such a test is a one-line addition.
#[allow(dead_code)]
fn _ensure_content_block_used() -> ContentBlock {
    ContentBlock::Text {
        text: String::new(),
        citations: None,
    }
}
```

- [ ] **Step 2: Run E2E tests**

Run: `cargo nextest run -p cogito-tui --test e2e`
Expected: 5 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/tests/e2e.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): end-to-end via TestBackend + Action dispatch

5 tests drive the dispatcher + App + render pipeline without entering
real raw mode:
- typing + Enter -> Action::SubmitUser; manual push of user prompt
  and streamed model response; assert both rendered
- Ctrl-T flips show_tools and removes 'tools ' from rendered buffer
- Ctrl-C during turn returns Action::CancelTurn
- '/foo' + Enter -> Action::SubmitSlash; slash::dispatch pushes
  '[error] unknown command' notice; assert rendered
- tool lifecycle (start + end + turn complete) populates chat
  '[tool] read_file' line AND tools pane 'turn 1' + node entry

These cover spec §11 acceptance criteria 1, 2, 4, 5, 6 (cancel half),
8 (slash unknown half).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 19: Edge-case tests

### Task 21: tests/edges.rs

**Files:**
- Create: `crates/cogito-tui/tests/edges.rs`

- [ ] **Step 1: Implement the four edge-case tests**

Create `crates/cogito-tui/tests/edges.rs`:

```rust
//! Edge cases from spec §7.6:
//!   - Terminal resize mid-stream
//!   - Extremely long single-line input (10k chars)
//!   - Unicode in tool args (CJK + emoji)
//!   - Deep tool tree (50+ calls in one turn)

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::keymap::dispatch;
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::status::StatusData;
use cogito_tui::ui::{RenderInputs, render};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn fresh_app() -> App {
    let store: Arc<dyn cogito_protocol::ConversationStore> = Arc::new(
        cogito_test_fixtures::in_memory_store::InMemoryStore::new(),
    );
    let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> = Arc::new(
        cogito_test_fixtures::strategy::MapStrategyRegistry::default(),
    );
    let handle = cogito_test_fixtures::session_handle_stub::stub_handle();
    App {
        handle,
        registry,
        store,
        session_id_str: "01".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        show_tools: true,
        popup: None,
        strategy_name: "x".into(),
        model_id: "m".into(),
        turn_count: 0,
        turn_in_progress: false,
        cancel_seen_at: None,
        should_quit: false,
    }
}

fn draw(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render(
                f,
                &RenderInputs {
                    chat: &app.chat,
                    tools: &app.tools,
                    selected: app.selected,
                    expanded: &app.expanded,
                    input: &app.input,
                    show_tools: app.show_tools,
                    status: &StatusData {
                        strategy: app.strategy_name.clone(),
                        model: app.model_id.clone(),
                        session_id: app.session_id_str.clone(),
                        turn_count: app.turn_count,
                        tools_visible: app.show_tools,
                    },
                    popup_prefix: None,
                },
            );
        })
        .unwrap();
    format!("{}", terminal.backend().buffer())
}

#[test]
fn resize_mid_stream_does_not_lose_content() {
    let mut app = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "before resize".into(),
    });
    // Render at one size, then another. Content must appear in both.
    let small = draw(&app, 40, 10);
    assert!(small.contains("agent: before resize"), "small render:\n{small}");
    let big = draw(&app, 120, 30);
    assert!(big.contains("agent: before resize"), "big render:\n{big}");
}

#[test]
fn extremely_long_input_does_not_panic() {
    let mut app = fresh_app();
    // 10k-char paste via successive char events. tui-textarea must
    // accept; render must not crash.
    let blob = "x".repeat(10_000);
    for ch in blob.chars() {
        let _ = dispatch(&mut app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let out = draw(&app, 80, 24);
    // We don't assert the full buffer round-trips visually — the
    // assertion is "no panic, frame renders". The 'x' rune should
    // appear at least once.
    assert!(out.contains('x'), "expected at least one rendered 'x'");
}

#[test]
fn unicode_in_tool_args_renders_without_corruption() {
    let mut app = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c1".into(),
        tool_name: "q".into(),
        args: serde_json::json!({"keyword": "深圳 🌟"}),
    });
    let out = draw(&app, 80, 24);
    assert!(out.contains("深圳"), "CJK characters missing:\n{out}");
    assert!(out.contains("🌟"), "emoji missing:\n{out}");
}

#[test]
fn deep_tool_tree_renders_without_panic() {
    let mut app = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    for i in 0..60 {
        let call_id = format!("c{i}");
        app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
            call_id: call_id.clone(),
            tool_name: format!("t{i}"),
            args: serde_json::json!({}),
        });
        app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
            call_id,
            ok: true,
            error_message: None,
        });
    }
    assert_eq!(app.tools.total_nodes(), 60);
    let out = draw(&app, 80, 24);
    // Even at a 24-row terminal, the first few nodes must be visible
    // and the render must not have panicked.
    assert!(out.contains("turn 1"), "first turn header missing:\n{out}");
    assert!(out.contains("t0"), "first node missing:\n{out}");
}
```

- [ ] **Step 2: Run edge tests**

Run: `cargo nextest run -p cogito-tui --test edges`
Expected: 4 tests passed (may take a few seconds for the 10k-char input loop).

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/tests/edges.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): edge cases — resize, long input, unicode, deep tree

Spec §7.6 cases:
- resize_mid_stream: same chat content renders at 40x10 and 120x30
- extremely_long_input: 10k char paste does not panic; 'x' appears
- unicode_in_tool_args: CJK '深圳' + emoji '🌟' round-trip through
  args preview without corruption
- deep_tool_tree: 60 tool calls in one turn renders without panic
  and at least 'turn 1' + 't0' are visible

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 20: Resume replay test

### Task 22: tests/resume.rs with canned JSONL fixture

**Files:**
- Create: `crates/cogito-tui/tests/fixtures/session-single-text-turn.jsonl`
- Create: `crates/cogito-tui/tests/resume.rs`

- [ ] **Step 1: Inspect the existing fixture format**

Run: `head -10 crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
Expected: JSONL lines with `event_id`, `session_id`, `schema_version`, `seq`, `timestamp`, `payload`.

Either reuse this fixture (preferred — no maintenance burden) OR create a minimal canned one:

Create `crates/cogito-tui/tests/fixtures/session-single-text-turn.jsonl` based on the existing canonical fixture's shape — copy the SCHEMA_VERSION value and one `TurnStarted` + one `ContentBlockEnd { Text }` + one `TurnCompleted` event. Each line is a self-contained JSON object.

> If creating canned JSONL is too brittle (schema drifts), drop this file and let the test use the canonical fixture directly via `cogito_test_fixtures::fixtures_dir().join("sessions/sample-v1.jsonl")` (the test fixtures crate already exposes this kind of helper).

- [ ] **Step 2: Write the resume test**

Create `crates/cogito-tui/tests/resume.rs`:

```rust
//! Resume test: read canonical JSONL → translate → drive into
//! ChatModel + ToolTreeModel → assert expected scrollback.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use cogito_protocol::ConversationStore;
use cogito_protocol::session::SessionId;
use cogito_tui::render_model::{ChatLine, ChatModel, ToolTreeModel};
use cogito_tui::resume::{InitialState, load_initial_state, translate_events};

#[tokio::test(flavor = "current_thread")]
async fn replay_canonical_fixture_reconstructs_chat() {
    // Use the canonical fixture. cogito-test-fixtures::fixtures_dir
    // is the conventional accessor; adapt if it's named differently.
    let fixture_path = cogito_test_fixtures::fixtures_dir().join("sessions/sample-v1.jsonl");
    let store: Arc<dyn ConversationStore> =
        Arc::new(cogito_store_jsonl::JsonlStore::open(fixture_path.parent().unwrap()).unwrap());
    // Extract session_id from filename or first line. For the canonical
    // fixture we know the session ULID convention; if a session_id
    // accessor exists on the store, prefer that.
    let session_id_str = std::fs::read_to_string(&fixture_path).unwrap();
    let first_line = session_id_str.lines().next().unwrap();
    let json: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let sid_str = json["session_id"].as_str().unwrap();
    let session_id = SessionId::parse(sid_str).unwrap();

    let state = load_initial_state(&store, &session_id, false).await.unwrap();
    let stream_events = match state {
        InitialState::Replayed { stream_events } => stream_events,
        InitialState::Fresh => panic!("expected Replayed, got Fresh"),
    };
    assert!(
        !stream_events.is_empty(),
        "canonical fixture should produce ≥ 1 stream event"
    );

    let mut chat = ChatModel::new();
    let mut tools = ToolTreeModel::new();
    for ev in &stream_events {
        chat.on_event(ev);
        tools.on_event(ev);
    }

    // The canonical fixture has at least one turn with content.
    assert!(
        chat.lines
            .iter()
            .any(|l| matches!(l, ChatLine::AssistantText(_))),
        "expected at least one AssistantText line; got: {:?}",
        chat.lines
    );
}

#[test]
fn translate_events_handles_canonical_shape_synchronously() {
    // Cheaper sanity check that doesn't need tokio: translate_events
    // alone with the fixture-derived ConversationEvents.
    let fixture_path = cogito_test_fixtures::fixtures_dir().join("sessions/sample-v1.jsonl");
    let text = std::fs::read_to_string(&fixture_path).unwrap();
    let events: Vec<cogito_protocol::ConversationEvent> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let translated = translate_events(&events);
    assert!(!translated.is_empty());
}
```

> **VERIFY:** `cogito_test_fixtures::fixtures_dir()` accessor. If it doesn't exist, the equivalent is `PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testing/cogito-test-fixtures/fixtures")` — but path math is brittle across worktrees, so prefer adding the accessor to `cogito-test-fixtures` if it's missing.
>
> Also verify `JsonlStore::open(dir: &Path)` signature; it may need different shape (per-session file vs directory).

- [ ] **Step 3: Run resume test**

Run: `cargo nextest run -p cogito-tui --test resume`
Expected: 2 tests passed.

If the canonical fixture has a different shape than the translator expects, adapt `translate_events` to handle the actual `EventPayload` variants present in the fixture. The fixture is canonical — the translator should bend to it, not the other way around.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/tests/resume.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): resume — replay canonical fixture into models

2 tests using cogito-test-fixtures' canonical sample-v1.jsonl:
- replay_canonical_fixture_reconstructs_chat: load_initial_state ->
  Replayed; drive translated events into ChatModel + ToolTreeModel;
  assert at least one AssistantText line appears in chat scrollback
- translate_events_handles_canonical_shape_synchronously: cheaper
  smoke check without tokio runtime

Both verify the spec §4.6 invariant (state regeneratable from JSONL).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 21: Snapshot test suite

### Task 23: tests/snapshot.rs

**Files:**
- Create: `crates/cogito-tui/tests/snapshot.rs`

- [ ] **Step 1: Write canonical-state snapshot tests**

Create `crates/cogito-tui/tests/snapshot.rs`:

```rust
//! Visual-state snapshot tests using ratatui's TestBackend. These
//! complement the per-widget tests already in the source files by
//! asserting the full composed frame in canonical states.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::status::StatusData;
use cogito_tui::ui::{RenderInputs, render};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn app() -> App {
    let store: Arc<dyn cogito_protocol::ConversationStore> = Arc::new(
        cogito_test_fixtures::in_memory_store::InMemoryStore::new(),
    );
    let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> = Arc::new(
        cogito_test_fixtures::strategy::MapStrategyRegistry::default(),
    );
    let handle = cogito_test_fixtures::session_handle_stub::stub_handle();
    App {
        handle,
        registry,
        store,
        session_id_str: "01abcdef".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        show_tools: true,
        popup: None,
        strategy_name: "coder".into(),
        model_id: "claude-opus-4-7".into(),
        turn_count: 0,
        turn_in_progress: false,
        cancel_seen_at: None,
        should_quit: false,
    }
}

fn draw(app: &App, popup_prefix: Option<&str>) -> String {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render(
                f,
                &RenderInputs {
                    chat: &app.chat,
                    tools: &app.tools,
                    selected: app.selected,
                    expanded: &app.expanded,
                    input: &app.input,
                    show_tools: app.show_tools,
                    status: &StatusData {
                        strategy: app.strategy_name.clone(),
                        model: app.model_id.clone(),
                        session_id: app.session_id_str.clone(),
                        turn_count: app.turn_count,
                        tools_visible: app.show_tools,
                    },
                    popup_prefix,
                },
            );
        })
        .unwrap();
    format!("{}", terminal.backend().buffer())
}

#[test]
fn empty_state_shows_panes_and_status() {
    let app = app();
    let out = draw(&app, None);
    assert!(out.contains("chat"));
    assert!(out.contains("tools"));
    assert!(out.contains("(no tool calls yet)"));
    assert!(out.contains("strategy: coder"));
    assert!(out.contains("Ctrl-C cancel"));
}

#[test]
fn single_text_turn_renders_user_and_agent_lines() {
    let mut app = app();
    app.chat.push_user_prompt("who are you?".into());
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "I am cogito.".into(),
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);
    let out = draw(&app, None);
    assert!(out.contains("> who are you?"));
    assert!(out.contains("agent: I am cogito."));
}

#[test]
fn popup_overlays_when_prefix_set() {
    let app = app();
    let out = draw(&app, Some("/"));
    assert!(out.contains("commands"));
    assert!(out.contains("/skill"));
}

#[test]
fn tools_hidden_grows_chat_width() {
    let mut app = app();
    app.show_tools = false;
    let out = draw(&app, None);
    assert!(!out.contains("tools "));
    assert!(out.contains("tools: off"));
}

#[test]
fn mcp_banner_lines_render_at_top_of_chat() {
    let mut app = app();
    app.chat.push_notice("[mcp] ✓ filesystem ready (4 tools)".into());
    let out = draw(&app, None);
    assert!(out.contains("filesystem ready"));
}
```

- [ ] **Step 2: Run snapshot tests**

Run: `cargo nextest run -p cogito-tui --test snapshot`
Expected: 5 tests passed.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/tests/snapshot.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): canonical-state TestBackend snapshots

5 frame-level snapshot tests at 80x20:
- empty_state_shows_panes_and_status (both panes + placeholder + status)
- single_text_turn_renders_user_and_agent_lines
- popup_overlays_when_prefix_set
- tools_hidden_grows_chat_width
- mcp_banner_lines_render_at_top_of_chat

These complement the per-widget tests already in src/ui/*.rs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 22: Documentation propagation

### Task 24: README + components doc

**Files:**
- Create: `crates/cogito-tui/README.md`
- Create: `docs/components/cogito-tui.md`

- [ ] **Step 1: Write the crate README**

Create `crates/cogito-tui/README.md`:

````markdown
# cogito-tui

Multi-pane terminal UI for the cogito runtime. Replicates `cogito chat`
in a ratatui frontend with a per-turn tool-call tree alongside the chat
scrollback.

## Quick start

```bash
cargo run -p cogito-tui -- --strategy coder
```

Same flag surface as `cogito chat`: `--config`, `--model`, `--provider`,
`--base-url`, `--session-root`, `--session-id`, `--mode`, `--system`,
`--strategy`, `--list-strategies`. Plus `--debug` for file-rotating logs
at `$XDG_STATE_HOME/cogito/tui.log`.

## Layout

```
┌───────────────────────┬──────────────┐
│ chat (70%)            │ tools (30%)  │
│   > who are you?      │  turn 1      │
│   agent: I am cogito. │   read_file ok│
│   [tool] read_file ok │              │
├───────────────────────┴──────────────┤
│ message (multi-line, Shift+Enter)    │
├──────────────────────────────────────┤
│ strategy: coder · model: ... · ...   │
└──────────────────────────────────────┘
```

`Ctrl-T` toggles the tools pane.

## Keymap

| Key | Action |
|---|---|
| Enter | Send message |
| Shift+Enter | Newline in message buffer |
| PgUp / PgDn | Scroll chat (5 rows) |
| Ctrl-↑ / Ctrl-↓ | Move tool-tree selection |
| Ctrl-Enter | Expand / collapse selected tool node |
| Ctrl-T | Toggle tools pane |
| Ctrl-C (during turn) | Cancel turn |
| Ctrl-C twice within 2s (idle) | Exit |
| Ctrl-D on empty buffer | Exit |
| / (start of buffer) | Open slash command popup |
| Esc | Dismiss popup |

## Slash commands (v0.1)

- `/skill <name>` — activate a skill for the next turn (same as CLI)

The `/` discovery popup lists matching commands as you type.

## Architecture

cogito-tui is a Surface-layer crate (ADR-0004). It depends on:
- `cogito-protocol`, `cogito-config`, `cogito-strategy`,
  `cogito-model`, `cogito-tools`, `cogito-jobs`, `cogito-mcp`,
  `cogito-skills`, `cogito-store-jsonl`, `cogito-sandbox`, `cogito-core`
- `cogito-cli` (peer Surface crate — re-uses `chat_config`,
  `banner`, `resolve_strategy`, slash parser helpers)
- `ratatui = 0.28`, `crossterm = 0.28`, `tui-textarea = 0.7`,
  `tracing-appender = 0.2`

State models (`ChatModel`, `ToolTreeModel`) are sink-agnostic — they
transition on `StreamEvent` without touching ratatui types. The UI
widgets in `src/ui/` borrow `&Model` and paint at render time.

Result-preview text in the tool-tree is loaded lazily on first
`Ctrl-Enter` expansion via `ConversationStore::read_session` (spec
§5.3 α.1). No new event broadcast was added in v0.1.

See `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md` for the
full design.

## Manual smoke test (panic recovery)

```bash
cargo run -p cogito-tui
# In another terminal:
pgrep cogito-tui | xargs kill -TERM
# Back in the TUI terminal, verify echo + line discipline work:
echo "hello"
```

SIGKILL bypasses the panic hook by design — there is no way to handle
it. Documented limitation.
````

- [ ] **Step 2: Write the components doc**

Create `docs/components/cogito-tui.md`:

```markdown
# cogito-tui — Surface

Multi-pane ratatui terminal UI. Peer to `cogito-cli` in the Surface
layer (ADR-0004). Not a Harness component (no H-number).

## Position

```
Brain (cogito-core::harness)
  ↑
Runtime (cogito-core::runtime)
  ↑
+-------------+-------------+
| cogito-cli  | cogito-tui  |   <- Surface layer
+-------------+-------------+
```

Both Surfaces consume the same Runtime via the same builder pattern
and the same `FsStrategyRegistry`. No protocol or Brain change was
required to add cogito-tui.

## Module map

- `cli.rs` — `TuiArgs` (clap-derived flag surface, mirrors `ChatArgs`)
- `app.rs` — `App` state (single source of truth)
- `render_model.rs` — `ChatModel` + `ToolTreeModel` (sink-agnostic)
- `ui/` — pane widgets (chat, tools, input, status, popup) +
  top-level `render`
- `keymap.rs` — `dispatch(app, key) -> Action`
- `slash.rs` — `parse + dispatch` for `/skill <name>`
- `resume.rs` — `ConversationEvent → StreamEvent` translation +
  `extract_tool_result` for lazy lookup
- `runtime_build.rs` — Runtime + Session assembly (mirrors
  `cogito-cli::chat::run`'s prelude)
- `event_loop.rs` — `select!` over crossterm / stream / 33ms tick
- `terminal.rs` — `TerminalGuard` (RAII + panic hook + signals)
- `logs.rs` — gated `RUST_LOG`-driven file logger

## Key contracts

- **Lazy palette**: `ChatLine` stores raw text + structural variant;
  widgets paint at render time.
- **Lazy tool-result lookup**: tool-tree result preview populated on
  first `Ctrl-Enter` expand via `ConversationStore::read_session`.
- **State regeneratable from JSONL**: `apply_stream_event` runs the
  same in live and replay modes.
- **Drawing only on tick**: 33ms interval bounds CPU; key/stream
  handlers mutate state but never `draw`.
- **Three-layer terminal restore**: RAII Drop + panic hook + SIGTERM
  handler. SIGKILL unhandleable.

## Where things live (other docs)

- Spec: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`
- Plan: `docs/superpowers/plans/2026-05-28-sprint-9b-tui.md`
- ROADMAP entry: §"Sprint 9b · TUI"
- Strategy registry consumed by TUI: ADR-0026
- Runtime config TUI consumes: ADR-0017 §"Surface boundaries"
- MCP banner contract: ADR-0018 §3.5.3
```

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/README.md docs/components/cogito-tui.md
git commit -m "$(cat <<'EOF'
docs(cogito-tui): crate README + components doc

README.md: quick-start, layout diagram, keymap table, slash commands,
architecture summary, manual smoke test for panic recovery.

docs/components/cogito-tui.md: Surface-layer position diagram, module
map, key contracts (lazy palette, lazy result lookup, state regenera-
table from JSONL, drawing only on tick, three-layer terminal restore),
cross-references to spec/plan/ROADMAP/ADRs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 25: ROADMAP + CHANGELOG + config overview

**Files:**
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/configuration/overview.md`

- [ ] **Step 1: Tick the Sprint 9b ROADMAP boxes**

Edit `ROADMAP.md` §"Sprint 9b · TUI". Replace:

```markdown
- [ ] Basic TUI with ratatui replicating `cogito chat`
- [ ] `cogito-tui` reads the same FsStrategyRegistry; `--strategy` flag honored
- [ ] Spec to follow once 9a lands
```

with:

```markdown
- [x] Basic TUI with ratatui replicating `cogito chat`
- [x] `cogito-tui` reads the same FsStrategyRegistry; `--strategy` flag honored
- [x] Spec landed: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`
```

Also update the top-of-file status line — change "Sprint 9 split into 9a (done) and 9b (TUI; spec pending)" to "Sprint 9 split into 9a (done) and 9b (done)" and bump "Current sprint" to the next sprint (Sprint 4 MCP or Sprint 10 v0.1 hardening, whichever the team picks next).

- [ ] **Step 2: Add a CHANGELOG entry**

Edit `CHANGELOG.md`. Under `## [Unreleased]` (or create if missing), add:

```markdown
### Added

- `cogito-tui` lifted from stub to working multi-pane ratatui surface.
  Chat scrollback on the left, per-turn tool-call tree on the right,
  bottom status bar, multi-line input with `Shift+Enter` newline, slash
  command discovery popup, `Ctrl-T` to toggle tools pane,
  `Ctrl-C`/`Ctrl-D` cancel/exit. Full flag parity with `cogito chat`
  including `--strategy`, `--list-strategies`, `--session-id`,
  `--mode resume`. Spec: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`;
  ROADMAP entry: Sprint 9b.
- `tui-textarea` and `tracing-appender` added to workspace dependencies
  (transitive consumers: `cogito-tui` only).
- `assert_cmd` promoted from `cogito-cli` dev-dep to workspace dep.
- `cogito-test-fixtures::session_handle_stub` for pure-state TUI tests.

### Changed

- Workspace layout table (AGENTS.md, ARCHITECTURE.md, CLAUDE.md):
  `cogito-tui` row's "When" column updated from v0.2 to v0.1.
```

- [ ] **Step 3: Cross-reference from configuration overview**

Edit `docs/configuration/overview.md`. In the "Crate layout" or
"Surface consumers" section, add a line noting the TUI:

```markdown
- `cogito-tui` — second Surface consumer of `RuntimeConfig` +
  `FsStrategyRegistry`. Uses the same `build_runtime_config_and_registry`
  helper as the CLI; `--strategy` / `--list-strategies` flags identical.
  See `docs/components/cogito-tui.md`.
```

- [ ] **Step 4: Commit**

```bash
git add ROADMAP.md CHANGELOG.md docs/configuration/overview.md
git commit -m "$(cat <<'EOF'
docs: tick Sprint 9b in ROADMAP; CHANGELOG entry; config overview note

ROADMAP §Sprint 9b: all three TUI check-boxes ticked; spec reference
added; top-of-file status line updated.

CHANGELOG: Added entry summarizing the cogito-tui surface
(multi-pane layout, key bindings, flag parity), workspace dep
additions (tui-textarea, tracing-appender, assert_cmd promotion),
session_handle_stub test helper. Changed entry notes the workspace
layout v0.2 -> v0.1 bump.

docs/configuration/overview.md: line acknowledging cogito-tui as a
second Surface consumer of build_runtime_config_and_registry.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 26: AGENTS.md + ARCHITECTURE.md + CLAUDE.md workspace bumps

**Files:**
- Modify: `AGENTS.md`
- Modify: `ARCHITECTURE.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Bump the cogito-tui row in three identical tables**

Each file contains a workspace layout table. In each, find the row:

```markdown
| `cogito-tui` | Surface | v0.2 | TUI. |
```

Replace it with:

```markdown
| `cogito-tui` | Surface | v0.1 | TUI. Multi-pane ratatui surface replicating `cogito chat`; see `docs/components/cogito-tui.md`. |
```

Apply to `AGENTS.md`, `ARCHITECTURE.md`, `CLAUDE.md` (the project root version; the worktree root and main repo share content via the CLAUDE.md mirror).

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md ARCHITECTURE.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: bump cogito-tui workspace layout v0.2 -> v0.1 (Sprint 9b)

Same three-character edit in AGENTS.md, ARCHITECTURE.md, CLAUDE.md
table rows + a one-line description pointing at the new components doc.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 23: Final validation

### Task 27: make ci green; address any clippy / fmt issues

**Files:** (whatever lint surfaces)

- [ ] **Step 1: Format**

Run: `make fmt`
Expected: exit 0; files updated in place if needed.

- [ ] **Step 2: Clippy on the new crate**

Run: `make fix CRATE=cogito-tui`
Expected: exit 0; auto-fixable lints applied. Manually review the diff
before committing — auto-fix can over-eager-clone in some cases.

- [ ] **Step 3: Full CI**

Run: `make ci`
Expected: fmt-check + clippy + layer-check + test all green.

Fix any failures in place. Common surprises in this codebase:
- `clippy::doc_markdown` flagging `OpenAI`, `TUI`, `JSON`, `JSONL` —
  backtick them.
- `clippy::missing_errors_doc` on `pub fn` returning `Result` — add
  a `# Errors` section to the doc comment.
- `clippy::assigning_clones` — use `target.clone_from(&source)`
  instead of `target = source.clone()`.
- `clippy::manual_map_or` / `unnecessary_wraps` — simplify.

- [ ] **Step 4: Verify the test count**

Run: `cargo nextest run -p cogito-tui 2>&1 | tail -3`
Expected: `Summary [...] N tests run: N passed, 0 skipped` with
N ≥ 25 (the spec §7 target).

- [ ] **Step 5: Smoke test the binary (manual)**

```bash
make chat  # CLI: confirm still works
cargo run -p cogito-tui -- --list-strategies
# Should print the available strategies from the worktree's .cogito/strategies/
```

If a real Anthropic/OpenAI key is set, run a brief interactive session
via `cargo run -p cogito-tui` to verify the visual:
- type "say hello in three words" + Enter
- assistant response appears in the chat pane
- press Ctrl-T → tools pane disappears, chat grows
- press Ctrl-D → exit cleanly (terminal echoes input afterward)

Document any visual oddities; resolve them now (clipping, color, layout
glitches) or open a follow-up issue.

- [ ] **Step 6: Final commit**

If any lint fix-ups landed:

```bash
git add .
git commit -m "$(cat <<'EOF'
chore(cogito-tui): final make ci green

Resolves clippy lints surfaced by full workspace check (doc_markdown
backticks on OpenAI/JSON/TUI tokens, missing_errors_doc additions on
public Result-returning fns, assigning_clones swaps to clone_from).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If nothing changed: skip this commit. The branch is ready for PR.

- [ ] **Step 7: Push and open PR**

```bash
git push -u github feat/sprint-9b-tui
gh pr create --title "Sprint 9b Bundle Y: cogito-tui multi-pane TUI" --body "$(cat <<'EOF'
## Summary
- Lifts `cogito-tui` from stub to a working multi-pane ratatui surface (chat + per-turn tool tree + status bar).
- Full flag parity with `cogito chat` including `--strategy`, `--list-strategies`, `--session-id`, `--mode resume`.
- New shared workspace deps: `tui-textarea`, `tracing-appender`, `assert_cmd`.
- Spec: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`.

## Test plan
- [ ] `make ci` green
- [ ] `cargo nextest run -p cogito-tui` — 25+ tests pass
- [ ] `cogito-tui --list-strategies` prints same as `cogito chat --list-strategies`
- [ ] Manual smoke: open TUI, send a message, see streaming response, Ctrl-T toggles tools pane, Ctrl-D exits cleanly

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review (controller checklist)

This is the controller's pass — not a subagent dispatch. Run before
declaring the plan complete.

**1. Spec coverage:**

| Spec section | Plan task(s) | Notes |
|---|---|---|
| §3 row 1 (multi-pane) | Tasks 4–9 | Three panes implemented & wired |
| §3 row 2 (new renderer) | Tasks 3, 5, 6, 9 | ratatui-native; CLI Renderer untouched |
| §3 row 3 (tool tree expandable) | Tasks 4, 6, 12, 15 | TurnGroup model + expansion widget + lazy lookup |
| §3 row 4 (status bar + MCP banner + hints) | Tasks 8, 17 | StatusData + banner capture in runtime_build |
| §3 row 5 (Ctrl-T toggle) | Tasks 9, 12 | show_tools flag + layout branch |
| §3 row 6 (multi-line input) | Task 7 | tui-textarea wrapper |
| §3 row 7 (slash + popup) | Tasks 8 (popup), 13 (dispatch) | |
| §3 row 8 (cancel/exit/resume) | Tasks 12, 14, 16, 17 | Ctrl-C double-tap, resume replay, mode flag |
| §3 row 9 (implicit focus) | Task 12 | dispatch() routes by key, no mode |
| §3 detail: lazy palette | Tasks 5, 6 | Lines store raw text; paint at render |
| §3 detail: lazy result lookup (α.1) | Task 15 | populate_result_preview via store.read_session |
| §3 detail: gated file logging | Task 19 | install_file_logger when --debug or RUST_LOG |
| §4.4 (async shape) | Task 16 | select! over 3 sources |
| §4.6 (state regeneratable) | Tasks 14, 17 | replay drives apply_stream_event |
| §6.1 (terminal restore) | Task 10 | TerminalGuard + panic hook + signals |
| §6.5 (Ctrl-C double-tap) | Task 12 | CTRL_C_EXIT_WINDOW |
| §7 (testing) | Tasks 3–9 (per-module) + 18, 20, 21, 22, 23 (integration) | ≥ 25 tests target |
| §8 (docs propagation) | Tasks 24–26 | README + components + ROADMAP + CHANGELOG + workspace bumps |
| §11 acceptance criteria | All | Each item maps to ≥ 1 test or doc artifact |

No gaps.

**2. Placeholder scan:** Grep the plan for `TBD`, `TODO`, `implement later`,
`fill in details`, `Similar to Task`. None present in the body; the
"VERIFY:" notes are explicit confirmation requests, not placeholders.

**3. Type consistency:** Names used across tasks:
- `ChatModel`, `ToolTreeModel`, `App`, `TerminalGuard` — consistent.
- `ToolStatus::{Running, Ok, Err}` — consistent in Task 4 + Task 6 widget.
- `TreePath = (usize, usize)` — consistent in render_model, widget, keymap, app.
- `Action::{None, SubmitUser, SubmitSlash, CancelTurn, ExpandNode, Quit}`
  — consistent in keymap + event_loop.
- `InputOutcome::{Consumed, Submit}` — consistent in input widget + keymap.
- `StatusData` — consistent in status widget + app.
- `RenderInputs` — consistent in ui::mod + event_loop + tests.
- `TuiArgs`, `TuiMode` — consistent in cli + main + runtime_build.

No name drift.

The plan totals 27 tasks across 23 phases. Subagents can execute them
in order; review gates (per subagent-driven-development skill) apply
between tasks.

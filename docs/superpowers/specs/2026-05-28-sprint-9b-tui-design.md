# Sprint 9b — Cogito TUI (Bundle Y) Design

**Status**: Draft (brainstorm output, awaiting user review)
**Date**: 2026-05-28
**Sprint**: 9b (Bundle Y — TUI; Bundle X = Sprint 9a review follow-ups, PR #26)
**Sibling sprint**: 9a — Multi-model + Strategy registry (merged via PR #25; ADR-0026)
**Related ADRs**: ADR-0004 (Brain/Hands/Session boundaries), ADR-0017 (Runtime config), ADR-0018 (MCP), ADR-0019 (Reasoning content), ADR-0026 (Strategy registry)
**Related spec**: `docs/superpowers/specs/2026-05-21-dev-experience-cli-display-design.md` (CLI renderer this design intentionally does NOT share)

## 1. Why this sprint exists

ROADMAP §"Sprint 9b · TUI" carries three unchecked items:

- Basic TUI with `ratatui` replicating `cogito chat`
- `cogito-tui` reads the same `FsStrategyRegistry`; `--strategy` flag honored
- Spec to follow once 9a lands

9a shipped the multi-model + strategy registry plumbing the TUI consumes
(`StrategyRegistry` trait, `cogito-strategy` crate, `resolve_strategy`
helper in `cogito-cli`). 9b lifts the `cogito-tui` crate from its Sprint 0
stub (one `println!`) into a working multi-pane terminal UI that uses
the same Runtime, the same strategy registry, and the same StreamEvent
broadcast as the CLI.

## 2. What the TUI is (and isn't)

**Is**: a Surface-layer (per ADR-0004) ratatui application that opens a
`cogito-core::Runtime` session, subscribes to `StreamEvent`s, renders
them across a chat pane + per-turn tool-tree pane + status bar, and
sends user input via `SessionHandle::submit_user_text`. Selectable
strategies via `--strategy`. Full `cogito chat` flag parity.

**Is NOT**: a different agent runtime, a different protocol, a
multimedia surface (no image rendering — v0.2 territory), a desktop GUI,
or a remotable thin client. The TUI is the same Runtime in-process,
displayed differently.

**Why a separate crate, not "CLI with `--tui` flag"**: `ratatui` brings
its own event loop and terminal contract (raw mode + alternate screen)
that conflicts with the CLI's `stdin.lines()` + ANSI-to-stdout
assumptions. Bolting both into one binary doubles the complexity of
both. Two binaries sharing the Runtime is the standard Surface-layer
pattern.

## 3. Scope (locked decisions)

The brainstorm settled nine scope-defining questions. Captured here so
the plan downstream cannot re-litigate them.

| # | Topic | Decision | Alternatives rejected |
|---|---|---|---|
| 1 | Surface shape | Multi-pane: chat + per-turn tool tree + status bar | Single-pane REPL clone (too thin a deliverable); fully-rich Codex-style with usage panel (out of v0.1 budget) |
| 2 | Renderer | New ratatui-native translation in `cogito-tui`; CLI `Renderer` untouched | Shared "presentation core" crate (real work, deserves its own ADR); ANSI buffering via `ansi-to-tui` (extra dep, partial reuse) |
| 3 | Tool tree | Per-turn tree, expandable for args/result preview | Flat live list (too little); session-wide tree (unbounded memory, navigation noise) |
| 4 | Status bar | Bottom single line + MCP banner as scrollback header + key hints | Top+bottom dual bar (chrome cost); minimal-only (no key discoverability) |
| 5 | Geometry | Chat-left / tools-right; `Ctrl-T` toggles tools pane | Fixed 70/30 (no escape hatch); resizable + persisted (needless persistence layer) |
| 6 | Input | Multi-line buffer; `Shift-Enter` newline, `Enter` sends; cap ~8 visible lines | Single-line `> ` (parity with CLI, worse than CLI for paste); external `$EDITOR` (too disruptive) |
| 7 | Slash commands | CLI's set (`/skill <name>`) + `/`-triggered discovery popup | Mirror-only (no discoverability win); add `/strategy` mid-session swap (needs Brain API not yet available) |
| 8 | Cancel/exit/resume | CLI parity: `Ctrl-C` cancel + double-tap exit; `Ctrl-D` exit; `--session` / `--resume-latest` honored | Minimal (no resume — contradicts ROADMAP); session picker popup (needs `list_sessions()` storage API) |
| 9 | Focus model | Implicit: `PgUp/Dn` chat, `Ctrl-↑/↓` tool nav, `Ctrl-Enter` expand | Modal nav (`-- INPUT --`/`-- NAV --` mode, jarring); Tab focus cycling (forces focus indicators on every pane) |

Three further detail-level decisions made during section presentation:

- **Lazy palette painting**: `ChatLine` stays as structural enum + raw
  text; ratatui `Span` palette is applied at render time, not on event
  ingestion. Future theme toggle is a re-render, not a re-translation.
- **Lazy tool-result lookup (α.1)**: the tool-tree pane reads
  `ConversationEvent::ContentBlockEnd { block: ContentBlock::ToolResult }`
  from the JSONL store on `Ctrl-Enter` expansion, caches on the
  `ToolNode`. **No new `ConversationEvent` broadcast added to
  `SessionShared`**; that is a clean follow-up if a second subscriber
  ever appears.
- **File logging gated by `RUST_LOG` / `--debug`**: TUI mode normally
  emits nothing to stderr (raw mode owns it); when debug logging is
  requested, `tracing` events go to `$XDG_STATE_HOME/cogito/tui.log`
  with size rotation.

## 4. Architecture

### 4.1 Layer position

`cogito-tui` is a Surface crate (peer to `cogito-cli`). Per ADR-0004:

```
Brain (cogito-core::harness)           — only sees Protocol traits
   ↑
Runtime (cogito-core::runtime)         — wires Brain to Hands/Boundary/Session
   ↑
Surfaces:  cogito-cli      cogito-tui     consumer Server (v0.4)
                ↓               ↓                 ↓
                ↳ all consume Runtime via the same builder
```

The TUI imports concrete Hands/Boundary/Session crates exactly like the
CLI does. No new Brain dependency, no new Protocol trait, no Runtime
change.

### 4.2 Crate layout

```
crates/cogito-tui/
  Cargo.toml                       (extends current stub deps)
  src/
    main.rs                        clap CLI + dispatch (tui::run / list_strategies / --help)
    lib.rs                         re-exports for tests
    app.rs                         App state (session handle, panes, focus, mcp banner)
    event_loop.rs                  select! { crossterm | StreamEvent | redraw tick }
    keymap.rs                      central key-to-action table (Ctrl-C, Ctrl-T, etc.)
    terminal.rs                    raw-mode + alt-screen setup; panic hook; signal hook
    render_model.rs                StreamEvent → ChatLine + ToolNode translation (the "Q2-A" core)
    resume.rs                      ConversationEvent log → initial pane state
    slash.rs                       in-process slash dispatch (no model roundtrip for parse errors)
    ui/
      mod.rs                       top-level layout (ratatui Layout)
      chat.rs                      chat scrollback widget + render(palette, lines)
      tools.rs                     tool-tree widget + render
      input.rs                     multi-line input wrapper around tui-textarea
      status.rs                    bottom status bar widget
      popup.rs                     /-command discovery popup
  tests/
    snapshot.rs                    TestBackend snapshot tests
    e2e.rs                         MockModelGateway end-to-end
    resume.rs                      replay-from-JSONL → initial pane state
    list_strategies.rs             --list-strategies parity with cogito-cli
    fixtures/
      test-strategy.md             canned strategy
      session-*.jsonl              canned session logs
```

### 4.3 Cargo.toml additions

The current stub depends on `cogito-core`, `ratatui`, `crossterm`, `tokio`,
`anyhow`. Additions required:

```toml
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
cogito-cli.workspace = true             # library exports: resolve_strategy etc.

clap = { workspace = true, features = ["derive"] }
tui-textarea = "0.7"                    # new workspace dep; well-maintained ratatui companion
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-appender = "0.2"                # new workspace dep; file rotation for --debug logs
tokio-stream.workspace = true
futures-util.workspace = true
```

`cogito-cli` becomes a library too: its existing `lib.rs` (8 lines) gets
populated with `pub mod chat_config; pub mod render;` (where useful)
plus re-exports of `resolve_strategy`, `build_runtime_config_and_registry`,
`synthesize_legacy_provider`, the slash parser, and the replay helper.
The CLI binary continues to work; the TUI gets a clean import path.
This is pragmatic Surface-layer sharing (no architectural violation) and
the right cleanup target for a future `cogito-surface-common` extraction
once the shared surface area grows.

**Workspace dep additions**: `tui-textarea = "0.7"` and
`tracing-appender = "0.2"`. Both go in `[workspace.dependencies]`; both
are widely used and Apache-2.0 / MIT licensed (compatible with the
existing license posture).

### 4.4 Async shape

Single-threaded tokio runtime hosts everything. The event loop is one
`tokio::select!` over three sources:

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = TuiArgs::parse();
    if args.list_strategies { return print_strategies_and_exit(args).await; }

    // Pre-raw-mode work: load config, build registry, build runtime, open session.
    // Errors here print to stderr and exit non-zero without entering raw mode.
    let (runtime, registry, mcp_failures, mcp_ok) = build_runtime(&args).await?;
    let handle = open_session(&runtime, &args).await?;
    let initial_state = resume::load_initial_state(&handle, &args).await?;

    // Now enter raw mode (RAII guard).
    let _term = terminal::TerminalGuard::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut app = App::new(handle, registry, initial_state, mcp_failures, mcp_ok);
    let mut crossterm_events = EventStream::new();
    let mut stream_events = BroadcastStream::new(app.handle.subscribe());
    let mut redraw_tick = tokio::time::interval(Duration::from_millis(33));  // ~30 FPS

    loop {
        tokio::select! {
            Some(Ok(ev)) = crossterm_events.next() => app.on_key(ev).await?,
            Some(ev) = stream_events.next() => app.on_stream(ev),
            _ = redraw_tick.tick() => { terminal.draw(|f| ui::render(f, &mut app))?; }
        }
        if app.should_quit { break; }
    }
    Ok(())
}
```

Key contracts:

- `app.on_key` and `app.on_stream` mutate state only; they never draw.
- Drawing only happens on tick. A `TextDelta` storm of 1000 events/sec
  produces 30 redraws/sec, not 1000.
- The 33ms tick keeps CPU usage bounded; perceived latency is ≤1 frame.

### 4.5 State model

```rust
pub struct App {
    // Session plumbing
    session: SessionHandle,
    registry: Arc<dyn StrategyRegistry>,         // for /strategy listing in popup, never mid-session swap
    store: Arc<dyn ConversationStore>,            // for lazy tool-result lookup
    cancel_seen_at: Option<Instant>,              // Ctrl-C double-tap timer
    turn_in_progress: bool,                       // set on TurnStarted, cleared on Turn{Completed,Failed,Cancelled,Paused}

    // Chat pane state
    chat: ChatModel,                              // Vec<ChatLine>, scroll_offset, in_text/in_thinking flags

    // Tool tree state
    tools: ToolTreeModel,                         // Vec<TurnGroup { nodes: Vec<ToolNode> }>
    selected: Option<TreePath>,                   // (turn_idx, node_idx) selection cursor
    expanded: HashSet<TreePath>,                  // toggled with Ctrl-Enter; result populates on first expand

    // Input state
    input: TextArea<'static>,                     // tui-textarea, capped at MAX_VISIBLE_INPUT_LINES = 8

    // Layout state
    show_tools: bool,                             // Ctrl-T toggle
    popup: Option<Popup>,                         // SlashMenu | None

    // Status bar / startup info
    strategy_name: String,
    model_id: String,
    session_id: SessionId,
    turn_count: u32,
    mcp_banner: Vec<Line<'static>>,               // prepended once to scrollback

    should_quit: bool,
}
```

```rust
pub enum ChatLine {
    UserPrompt(String),
    AssistantText(String),                        // accumulated across TextDelta
    AssistantThinking(String),                    // accumulated across ThinkingDelta
    ToolStartLine { tool: String, args_preview: String },
    ToolEndLine { tool: String, ok: bool, elapsed_ms: u128, error: Option<String> },
    SystemNotice(String),                         // [paused] / [resumed] / [cancelled] / [error] / mcp banner
}

pub struct ToolNode {
    call_id: String,
    tool_name: String,
    args: serde_json::Value,
    started_at: Instant,
    status: ToolStatus,                           // Running | Ok { elapsed } | Err { elapsed, message }
    result_preview: Option<String>,               // populated on first Ctrl-Enter expansion (α.1)
}
```

Both `ChatModel::on_event` and `ToolTreeModel::on_event` are sink-agnostic
pure functions of `(state, &StreamEvent) -> state`. This is the testable
heart of the surface.

### 4.6 State regeneration invariant

Mirrors AGENTS.md rule 3 ("State lives in Conversation Service, not
Harness memory"). The TUI's `App` must be reconstructible from the JSONL
log alone. On startup with `--session <id>`:

```text
1. open Runtime in SessionMode::Resume; runtime hands back SessionHandle + read store
2. read all ConversationEvents from store
3. resume::translate_to_stream_events(events) → Vec<StreamEvent>
4. for each StreamEvent: chat.on_event + tools.on_event
5. enter live event loop; new StreamEvents drive the same paths
```

Step 3 reuses (or mirrors) `cogito-cli::chat::replay_history` semantics
via the library export. The same translation logic runs in live and
replay modes.

## 5. Data flow

### 5.1 StreamEvent fan-out (live)

One `BroadcastStream<StreamEvent>` from `SessionHandle::subscribe()` is
consumed by `App::on_stream`. Each event mutates both models in sequence:

```rust
fn on_stream(&mut self, ev: StreamEvent) {
    self.chat.on_event(&ev);
    self.tools.on_event(&ev);
}
```

Models touch disjoint state. `TextDelta`/`ThinkingDelta` are no-ops for
`tools`; `ToolDispatchStarted/Ended` push `ChatLine::ToolStartLine`/
`ToolEndLine` into the scrollback (chat keeps the textual record) *and*
mutate the tool tree (structural index). Intentional duplication — a
user who hides the tools pane (`Ctrl-T`) still sees tool calls in the
chat scrollback.

### 5.2 Input → send path

```rust
on_key(Enter) without Shift:
    let text = input.lines().join("\n").trim().to_string();
    if text.is_empty() { return Ok(()); }
    if let Some(cmd) = slash::parse(&text) {
        slash::dispatch(self, cmd)?;        // in-process; no model roundtrip
    } else {
        self.chat.push_user_prompt(text.clone());
        self.session.submit_user_text(text).await?;
    }
    input.select_all(); input.delete_line();
```

`slash::parse` recognizes `/skill <name>` (v0.1 only). Unknown commands
push a `SystemNotice("[error] unknown command: /foo. Try /skill")` and
do not consume turn budget.

### 5.3 Tool-tree expansion (lazy result lookup, α.1)

```rust
on_key(Ctrl+Enter) with selected tool node finished:
    if !self.expanded.insert(path) {
        self.expanded.remove(&path);    // toggle off
        return;
    }
    let node = self.tools.get_mut(path);
    if node.status.is_finished() && node.result_preview.is_none() {
        let events = self.store.read_session(self.session_id).await?;
        if let Some(result) = events.iter()
            .find_map(|e| extract_tool_result(e, &node.call_id))
        {
            node.result_preview = Some(truncate(&result, RESULT_PREVIEW_MAX));
        }
    }
```

JSONL read is local-only file IO (sub-millisecond for sessions under a
few MB). Caching on the node avoids repeat reads.

`extract_tool_result` walks `ConversationEvent::ContentBlockEnd { block:
ContentBlock::ToolResult { call_id, content } }` and pulls out the text
or first text block from the content. Errors emit the indented
error-message string the CLI already shows; success previews the result
text capped at `RESULT_PREVIEW_MAX = 800` chars (mirrors the CLI's
`TOOL_ERROR_PREVIEW_MAX`).

### 5.4 Resume replay

```rust
async fn load_initial_state(handle: &SessionHandle, args: &TuiArgs) -> Result<InitialState> {
    if args.session_id.is_none() && !args.resume_latest { return Ok(InitialState::Fresh); }
    let events = handle.store().read_session(args.resolved_session_id()).await?;
    let stream_events = translate_to_stream_events(&events);     // shared with cogito-cli
    Ok(InitialState::Replayed { stream_events, last_seq: events.last().map(|e| e.seq) })
}
```

On startup, `App::new` drives `chat.on_event` + `tools.on_event` for
each translated `StreamEvent`. Tool-tree result previews populate
lazily — the same on-expand path as live mode. No special "replay vs
live" mode in the models.

## 6. Error handling

### 6.1 Terminal restoration (the ratatui foot-gun)

Three layers of defense, all must coexist:

```rust
// 1. RAII guard for normal exit
pub struct TerminalGuard;
impl TerminalGuard {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Self::install_panic_hook();
        Self::install_signal_hooks();
        Ok(Self)
    }
    fn install_panic_hook() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
            prev(info);
        }));
    }
    fn install_signal_hooks() { /* tokio::signal::unix SIGTERM/SIGHUP best-effort */ }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}
```

Layer 1 covers normal exit (`should_quit = true`); layer 2 covers
`unwrap!` and unexpected panics anywhere in the event loop; layer 3
covers `kill -TERM`. SIGKILL is documented as unhandleable.

### 6.2 Stream error surfaces

| Source | Manifestation | Handling |
|---|---|---|
| `StreamEvent::TurnFailed { reason }` | Model gateway error, tool-loop failure | Push `ChatLine::SystemNotice("[error] {reason}")`; session stays alive |
| `StreamEvent::TurnCancelled` | User Ctrl-C mid-turn | Push `[cancelled]` notice; reset cancel-double-tap timer |
| `BroadcastStream::Lagged(n)` | Slow consumer | Push `[warning] {n} events dropped`; continue. Do not resync — accept minor visual inconsistency; live view catches up on next user input |
| `SessionError::SessionClosed` | Runtime actor crashed | Render `[fatal] session closed unexpectedly`; pause 2s; restore terminal; exit code 2 |

### 6.3 Startup errors (before raw mode)

These print to stderr and `process::exit(1)` without entering raw mode:

- `ResolveError::{UnknownStrategy, UnknownProvider, MissingProvider, MissingModel, Strategy, LegacyBridge}` — same shape as CLI
- Config load failure (`cogito.toml` parse error)
- `--session <id>` not found in store

### 6.4 MCP banner

Reuse `cogito-cli::banner::render_banner` (sink-agnostic over `<W: Write>`).
Capture to `Vec<u8>`, decode UTF-8, split lines, prepend each as
`ChatLine::SystemNotice` so they render at the top of the chat
scrollback. Scroll out naturally as the session fills. Same content
shape as the CLI's stderr output.

### 6.5 Ctrl-C double-tap (matches CLI exit escalation)

```rust
on_key(Ctrl+C):
    if a turn is running:
        self.handle.cancel_turn().await?;
        self.cancel_seen_at = Some(Instant::now());
    else if self.cancel_seen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)):
        self.should_quit = true;
    else:
        self.cancel_seen_at = Some(Instant::now());
        self.chat.push_notice("[hint] Press Ctrl-C again to exit, or Ctrl-D on empty input");
```

### 6.6 Empty / whitespace input

`Enter` on empty buffer is a no-op (no notice, no turn). Same as CLI.

### 6.7 Tools pane empty state

When `tools.turns` is empty: render dim `(no tool calls yet)` placeholder
centered in the pane. Same when `show_tools = false` is toggled back on
with no history.

### 6.8 Debug logging (gated)

When `RUST_LOG` env or `--debug` flag set: install
`tracing_subscriber::fmt` with a `tracing_appender::rolling::daily`
writer pointed at `$XDG_STATE_HOME/cogito/tui.log` (or
`$HOME/.local/state/cogito/tui.log` fallback). 10MB size rotation,
7-day retention. Default: no logging (raw mode owns stderr; silent
logging is correct).

## 7. Testing

Five test surfaces, all idiomatic for ratatui apps. Target: ≥ 25 tests,
`make test CRATE=cogito-tui` runs in < 2 seconds.

### 7.1 TestBackend snapshot tests (`tests/snapshot.rs`)

`ratatui::backend::TestBackend` draws into an in-memory `Buffer` we
assert against. Cases:

- Empty initial state (fresh session, no events)
- Single text turn (user prompt + assistant text)
- Turn with one tool call (collapsed)
- Turn with tool call expanded (args + result preview shown)
- Popup open (`/` discovery menu)
- Tools pane hidden (`Ctrl-T` toggled off)
- Tools pane visible with multi-turn history
- Mid-stream rendering (turn in progress, partial text)
- Cancellation rendered (`[cancelled]` notice)
- Error rendered (`[error] foo` notice)
- MCP banner header present
- Resume-replay initial state matches expected snapshot

Twelve to fifteen snapshots covering visual states.

### 7.2 Model unit tests (`src/render_model.rs`, `src/ui/{chat,tools}.rs`)

`ChatModel::on_event` and `ToolTreeModel::on_event` are pure
state-transition functions. Feed `Vec<StreamEvent>`, assert resulting
state. ≥ 15 cases covering:

- TextDelta coalescing within one block
- ThinkingDelta coalescing within one block
- Text → Tool → Text re-emits agent label
- Multi-tool single turn produces multi-node TurnGroup
- TurnStarted pushes new TurnGroup
- TurnFailed produces SystemNotice without breaking subsequent turns
- TurnCancelled produces SystemNotice
- ToolDispatchEnded with error populates ToolNode error field
- Unknown future StreamEvent variant is a no-op (forward compat)

### 7.3 Resume-replay test (`tests/resume.rs`)

Use or adapt `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`.
Feed events through the replay adapter; assert final `ChatModel` +
`ToolTreeModel` state matches expected. Verifies state regeneration
invariant (§4.6) — the same chat + tree reconstructs from log alone.

### 7.4 End-to-end with MockModelGateway (`tests/e2e.rs`)

Build a `Runtime` with `MockModelGateway` (canned `StreamEvent`s) +
`MapStrategyRegistry` + no-op `JobManager`. Drive the event loop
synchronously by injecting `crossterm::Event`s into a `mpsc::Receiver`
substituted for the live `EventStream`. Assert state after key sequences:

- type `"hi\n"` + Enter → assistant text appears in scrollback
- Ctrl-T → tools pane disappears (state flag flipped)
- Ctrl-C during streaming → `[cancelled]` notice appears + buffer drains
- `/skill nonexistent` + Enter → `[error] unknown command` notice
- Ctrl-C twice within 2s → `should_quit` becomes true

### 7.5 --list-strategies parity (`tests/list_strategies.rs`)

Use `assert_cmd` to run `cogito-tui --list-strategies --config <fixture>`
and compare to `cogito-cli --list-strategies --config <fixture>`. Output
must be identical, no raw mode entered.

### 7.6 Edge cases (explicit, all four included)

- Terminal resize mid-stream (snapshot before + after `Event::Resize`)
- Extremely long single-line input (≥10k chars; buffer truncates display, accepts send)
- Unicode in tool args (CJK + emoji round-trip via `args_preview`)
- Deep tool tree (50+ calls in one turn; scrollable, no panic)

### 7.7 What's not tested

- Panic-recovery terminal restore (requires subprocess spawn; manual
  smoke test documented in `docs/components/` notes)
- Real VT100 sequences (trust ratatui's own coverage)
- `tui-textarea` internals (trust upstream)

## 8. Documentation propagation

Required after implementation lands:

- `crates/cogito-tui/README.md`: new (one-page overview + key reference)
- `docs/components/`: add `cogito-tui.md` (no H-number; Surface crate,
  not a Harness component) describing the panes, key bindings, and the
  Surface-layer position
- `AGENTS.md` §Workspace layout: bump `cogito-tui` from "v0.2" to "v0.1"
  status (it's shipping in v0.1 now)
- `ARCHITECTURE.md` workspace-layout table: same status bump
- `ROADMAP.md` §Sprint 9b: tick all three TUI check-boxes when done
- `CHANGELOG.md`: Sprint 9b entry under Unreleased
- `docs/configuration/overview.md`: add a line noting `cogito-tui`
  consumes the same `RuntimeConfig` + `FsStrategyRegistry`

No new ADR required for v0.1 — every architectural decision in this
spec already aligns with ADR-0004 (layer position), ADR-0017 (config),
ADR-0019 (reasoning rendering parity), ADR-0026 (strategy registry).
A future ADR may be warranted if a `cogito-surface-common` crate gets
extracted (currently deferred).

## 9. Migration

None. `cogito-tui` is presently a one-line `println!` stub with no
users. Replacing it is purely additive.

The CLI is untouched — no breaking change. The library-promotion of
`cogito-cli` adds re-exports in `lib.rs` but leaves the binary's `main.rs`
behavior identical.

## 10. Risks and open questions

### 10.1 Risks

- **R1 — `tui-textarea` integration**: new dep; if API surface
  changes, multi-line input breaks. Mitigation: pin `0.7.x`; the surface
  we use (`TextArea::new`, `input(Event)`, `lines()`, `select_all`,
  `delete_line`) is small and stable.
- **R2 — `crossterm` raw-mode on non-terminal stdout** (CI, piped):
  raw mode fails. Mitigation: detect `io::stdout().is_terminal()` before
  `enable_raw_mode`; print "TUI requires a terminal" + exit 1 if not.
- **R3 — broadcast lagging under heavy `TextDelta` storms**: ratatui
  redraws at 30 FPS but `BroadcastStream` channel default is small.
  Mitigation: configure `broadcast::channel(capacity = 1024)` at the
  Runtime side if the default is smaller. **Action item**: check the
  current Runtime broadcast capacity during implementation; raise to
  1024 if needed (small Runtime-side change, justified by TUI demand).
- **R4 — Library-promotion of `cogito-cli` creates a Surface →
  Surface dependency**: not architecturally wrong (both are Surface-layer)
  but unusual. Mitigation: document in the new `cogito-tui/README.md`
  and earmark `cogito-surface-common` extraction as a v0.2 cleanup
  task. Drift risk is low because both surfaces test the shared
  helpers via the existing CLI tests.
- **R5 — `tracing-appender` adds a runtime dep just for debug logging**:
  ~50KB binary size cost. Mitigation: runtime-gated via `RUST_LOG` /
  `--debug` (no log activity, no file IO when not requested). A
  compile-time feature gate is deliberately not added — the binary-size
  delta is small and the operational simplicity of "one binary, debug
  on demand" outweighs it.

### 10.2 Open questions (none blocking; for review)

- Should `--debug` log mode also write to a per-session log
  (`tui-<session_id>.log`)? Default plan: single rolling
  `tui.log`. Per-session is easy to add later.
- Should the popup support fuzzy match (`/sk` matches `/skill`) or only
  prefix? Default plan: prefix only for v0.1; fuzzy adds a search-rank
  layer not worth the complexity yet.
- Should `Ctrl-L` clear-screen be wired? Default plan: not in v0.1
  (clearing the scrollback discards state visible to the user; ratatui
  re-renders every tick, so a redraw isn't needed).

## 11. Acceptance criteria

A reasonable reviewer can tick all of these by hand:

1. `cogito-tui --strategy coder` opens a multi-pane terminal UI: chat
   on the left, empty tool-tree on the right, status bar at the bottom
   showing `strategy: coder | model: ...`.
2. Typing `hello\n` + Enter sends the message; assistant streaming
   text appears in the chat pane; on `TurnCompleted` the input bar is
   ready for the next message and `turn_in_progress` is false.
3. The same `--strategy` / `--list-strategies` / `--config` /
   `--session` / `--resume-latest` / `--model` flags behave identically
   to `cogito chat`.
4. A turn that invokes a tool produces a node in the tool-tree pane;
   `Ctrl-↑/↓` selects nodes; `Ctrl-Enter` expands to show args +
   result preview (loaded lazily from JSONL).
5. `Ctrl-T` toggles the tool-tree pane; chat pane grows to full width
   when hidden.
6. `Ctrl-C` cancels the current turn; a second `Ctrl-C` within 2s
   exits cleanly. `Ctrl-D` on empty input exits cleanly. Both paths
   restore the terminal (raw mode off, alternate screen left, cursor
   visible).
7. A panic during operation also restores the terminal (panic hook
   verified by manual smoke test).
8. `/skill <name>` works the same as in CLI; `/<other>` shows the
   discovery popup.
9. Resume via `--session <id>` reconstructs the chat scrollback and
   tool-tree from the JSONL log before the live event loop starts.
10. `--list-strategies` prints the same list as `cogito-cli --list-strategies`
    without entering raw mode.
11. `make ci` green; `make test CRATE=cogito-tui` runs in < 2s with
    ≥ 25 tests passing; no `#[ignore]`s.
12. ROADMAP Sprint 9b check-boxes ticked; CHANGELOG entry added;
    `docs/components/cogito-tui.md` exists.

## 12. References

- ROADMAP §Sprint 9b
- ADR-0004 (Brain/Hands/Session layer map)
- ADR-0017 (Runtime configuration model)
- ADR-0018 (MCP integration; banner contract)
- ADR-0019 (Reasoning content; thinking rendering parity)
- ADR-0026 (Strategy registry; the artifact the TUI reads)
- `docs/superpowers/specs/2026-05-21-dev-experience-cli-display-design.md`
  (the CLI renderer this design intentionally does not share)
- `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
  (Sprint 9a — sibling sprint providing `resolve_strategy` and the
  registry the TUI consumes)

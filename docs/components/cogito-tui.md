# cogito-tui — Surface

Single-column ratatui terminal UI with `▸` / `∴` role markers and
inline expandable tool blocks (no tools pane, no persistent status
bar). Peer to `cogito-cli` in the Surface layer (ADR-0004). Not a
Harness component (no H-number).

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
- `ui/` — widgets (chat, input, popup, spinner, banner) + top-level
  single-column `render`
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
  widgets paint at render time. A `ChatLine::ToolBlock { call_id }`
  carries no state — the chat renderer looks up the live `ToolNode` in
  `ToolTreeModel` by `call_id` at draw time and paints the lifecycle
  glyph (`⠋ ✓ ✗ ▸ ▾`).
- **Lazy tool-result lookup**: a tool block's result preview is
  populated on first expand (`Ctrl-Enter` / `Alt-N`) via
  `ConversationStore::read_session`.
- **Thinking spinner**: `App.current_turn_thinking` (set on
  `TurnStarted`, re-armed after `ToolDispatchEnded`, cleared by the
  next content event or any terminal event) drives a `∴ ⠋` line.
- **Modifier-gated commands**: all printable keys reach the input;
  tool commands use modifiers (`Ctrl-E` expand-all, `Ctrl-L`
  collapse-all, `Alt-1..9` quick-expand) so typing is never captured.
- **State regeneratable from JSONL**: `apply_stream_event` runs the
  same in live and replay modes.
- **Drawing only on tick**: 33ms interval bounds CPU; key/stream
  handlers mutate state but never `draw`.
- **Three-layer terminal restore**: RAII Drop + panic hook + SIGTERM
  handler. SIGKILL unhandleable.

## Where things live (other docs)

- Spec (current, v0.2 redesign):
  `docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`
- Plan (current, v0.2 redesign):
  `docs/superpowers/plans/2026-05-29-cogito-tui-redesign.md`
- Spec (original v0.1 multi-pane, superseded):
  `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`
- Plan (original v0.1, superseded):
  `docs/superpowers/plans/2026-05-28-sprint-9b-tui.md`
- ROADMAP entry: §"Sprint 9b · TUI"
- Strategy registry consumed by TUI: ADR-0026
- Runtime config TUI consumes: ADR-0017 §"Surface boundaries"
- MCP banner contract: ADR-0018 §3.5.3

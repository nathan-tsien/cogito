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
- `ratatui = 0.29`, `crossterm = 0.29`, `tui-textarea = 0.7`,
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

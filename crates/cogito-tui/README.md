# cogito-tui

Single-column terminal UI for the cogito runtime. Replicates `cogito
chat` in a ratatui frontend with `▸` / `∴` role markers and inline,
expandable tool blocks in the chat scrollback (no separate tools pane,
no persistent status bar).

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
┌──────────────────────────────────────┐
│ ∴∴∴                                  │
│ cogito  v0.2                         │
│ <model>  ·  <strategy>  ·  <session> │
│ ▸  who are you?                      │
│ ∴  I am cogito.                      │
│    ✓ read_file  12ms                 │
├──────────────────────────────────────┤
│ message (multi-line, Shift+Enter)    │
└──────────────────────────────────────┘
```

A single chat column with an input footer (separated by a dim
divider). `▸` marks user prompts, `∴` marks cogito replies. Tool calls
render inline as `⠋` (running) / `✓` (ok) / `✗` (error); `▸` when
selected, `▾` when expanded. A `∴ ⠋` spinner shows between turn
dispatch and the first content. The startup banner and any MCP banner
scroll away with history.

## Keymap

| Key | Action |
|---|---|
| Enter | Send message |
| Shift+Enter | Newline in message buffer |
| PgUp / PgDn | Scroll chat (5 rows) |
| Ctrl-↑ / Ctrl-↓ | Move tool-block selection |
| Ctrl-Enter | Expand / collapse selected tool block |
| Alt-1 … Alt-9 | Quick-expand the N-th most recent tool block |
| Ctrl-E | Expand all tool blocks in the latest cogito message |
| Ctrl-L | Collapse all tool blocks in the latest cogito message |
| Ctrl-C (during turn) | Cancel turn |
| Ctrl-C twice within 2s (idle) | Exit |
| Ctrl-D on empty buffer | Exit |
| / (start of buffer) | Open slash command popup |
| Esc | Dismiss popup |

All printable keys (including `e`, `c`, digits) always go to the input;
tool commands are modifier-gated so typing is never captured.

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

Result-preview text for a tool block is loaded lazily on first
expansion (`Ctrl-Enter` / `Alt-N`) via
`ConversationStore::read_session`. No new event broadcast was added.

See `docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`
for the current visual + interaction design, and
`docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md` for the
original v0.1 multi-pane design it superseded.

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

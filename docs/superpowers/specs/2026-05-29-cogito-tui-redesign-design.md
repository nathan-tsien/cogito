# Cogito TUI Redesign — Visual + Interaction (v0.2)

> Spec for the next iteration of `crates/cogito-tui`: drop the tools
> pane, drop the persistent status bar, move tool calls inline into
> chat, establish a coherent visual identity (`▸` / `∴`) tied to the
> project's "cogito" (thinking) ethos.
>
> Brainstorm session: 2026-05-29 (see "Decisions log" at the bottom).
>
> Predecessor: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`
> (cogito-tui v0.1, shipped via PR #27).

## Goal

Re-imagine the cogito-tui surface from a multi-pane layout
(70/30 chat + tools, persistent status bar) to a **minimal-flat
single-stream** design:

- Drop the tools pane. Tool calls live inline in chat as collapsible
  blocks (Claude Code style).
- Drop the persistent status bar. Identity info (project name, model,
  strategy, session id) appears as a startup banner that scrolls
  away naturally with chat history.
- Replace the text labels `> ` / `agent: ` with geometric role markers
  `▸` (user) and `∴` (cogito = "therefore", from *cogito ergo sum*).
- Spinner-based feedback for the wait between turn dispatch and
  first model token, and for running tools.

## Why

Sprint 9b shipped a working but visually unsatisfying TUI. User
feedback (2026-05-29):

- Tools pane occupies 30% of the chat row but adds little real value;
  the lazy-result-lookup feature is rarely used and tool calls already
  surface as `[tool]` lines in chat.
- Persistent `turn:` counter, `tools: on/off`, and key-hint strip in
  the status bar are noise after the first use.
- ASCII `Borders::ALL` line borders and generic "You / cogito" labels
  feel rough and dated.
- No visual identity tying the surface to the project's "cogito"
  (thinking) ethos — the symbols are arbitrary.

PR #27 review surfaced additional polish items (markdown rendering,
incremental projection, message animations); those are split into
separate sprints (see "Out of scope" below) so this redesign focuses
on visual language + interaction model.

---

## Visual language

### Role markers (locked)

| Role | Glyph | Color | Rationale |
|------|-------|-------|-----------|
| User | `▸` | cyan | Input chevron; conventional for "send" affordance. |
| Cogito | `∴` | green | "Therefore" operator from *cogito ergo sum*. Each agent reply is a reasoning conclusion. |

3-space content indent under both markers. Multi-line user / cogito
messages keep the marker on line 1 only; subsequent lines align under
the content column.

```
▸  who are you?

∴  I am cogito, an embedded
   agent runtime.
```

### Flat, borderless

No `Block::borders(Borders::ALL)` anywhere in the persistent UI.
The only persistent visual structure is a single dim horizontal rule
(`─────`) immediately above the input. No pane titles. No box-drawing.

### Color palette

| Token | `ratatui::Color` | Modifier | Usage |
|-------|------------------|----------|-------|
| `accent.user` | Cyan | — | `▸` user marker |
| `accent.cogito` | Green | — | `∴` cogito marker, `✓` success |
| `accent.tool_running` | Yellow | DIM | `⠋` spinner (running tool) |
| `accent.error` | Red | — | `✗` failure, `[error]` notices |
| `accent.selection` | Cyan / Blue | — | `▸` selected tool block |
| `dim.1` | default fg | DIM | secondary text, timestamps, args preview |
| `dim.0` | DarkGray | — | divider lines, placeholder text |
| `fg` | default fg | — | primary message text |

Theme/palette swap is a future concern; v0.2 ships with dark-default
only. All choices degrade gracefully on 16-color terminals.

---

## Chrome strategy: Ambient (no persistent header / status)

### Startup banner (first frame)

Pushed at App build time as 3 `SystemNotice` lines into `ChatModel`
**before** the live event loop starts. Scrolls away naturally with
chat history.

```
   ∴∴∴
   cogito  v0.2
   opus-4.7  ·  coder  ·  01abcdef
```

- Line 1: `   ∴∴∴` (green, indented 3 spaces — matches content column).
- Line 2: `   cogito  v0.2` (default fg + dim version).
- Line 3: `   <model_id>  ·  <strategy_name>  ·  <session_id[:8]>` (all dim).

No tool list, no MCP info — those are separate banner lines (below).
No user message yet.

### MCP banner

Existing per-server status lines from `cogito_cli::banner::render_banner`
push immediately after the startup banner as dim `SystemNotice`
entries. Same scrolling behavior. Format unchanged:

```
   [mcp] ✓ filesystem ready (4 tools)
   [mcp] ✓ search ready (2 tools)
   [mcp] ✗ kubernetes startup failed: timeout
```

### Input footer

Single dim horizontal rule above the multi-line input:

```
─────────────────────────────────────────────────────────────────
›  Type a message...
```

- No model name ghost text.
- No key hints (`/help` slash command can surface them later).
- Input grows from 1 to 8 visible lines (existing
  `MAX_VISIBLE_INPUT_LINES = 8` preserved).
- Placeholder shown when buffer is empty; cleared on first keystroke.

### What's GONE compared to v0.1

| Removed | Replacement |
|---------|-------------|
| `cogito chat` pane (70% width) | Single full-width chat column. |
| `tools` pane (30% width) | Inline expandable tool blocks within cogito messages. |
| Persistent bottom status bar | Startup banner (scrolls away). |
| `turn: N` counter | Nothing visible. Counter still maintained internally for telemetry. |
| `tools: on/off` indicator | Nothing (no pane to toggle). |
| Key-hint strip (`Ctrl-C cancel · …`) | Nothing in v0.2; future `/help` command. |

---

## Tool block lifecycle (T1 — Status Glyph)

Each tool dispatch is a single inline block rendered within the
cogito (`∴`) reply that triggered it. Block starts at the position
in stream order where `ToolDispatchStarted` arrived (not where it
completed — a long-running tool stays at its dispatch position even
if surrounding text streams past it).

### States and glyphs

| State | Leading glyph | Color | Full render |
|-------|---------------|-------|-------------|
| Running | `⠋` (animated) | yellow + DIM | `   ⠋ <tool_name>           <elapsed_s>` |
| Completed OK | `✓` | green | `   ✓ <tool_name>           <duration_ms>` |
| Completed Err | `✗` | red | `   ✗ <tool_name>           <duration_ms>`<br/>`     ↳ <error_message>` (red, dim) |
| Selected (mutex with state glyph) | `▸` | cyan | `   ▸ <tool_name>           <duration>` |
| Expanded | `▾` | green | `   ▾ <tool_name>           <duration>`<br/>`     args   <one-line args preview>`<br/>`     ↳ <result_preview lines...>` (all dim) |

Notes:

- 3-space indent matches the role-marker content indent so tool
  blocks visually belong to the parent cogito message.
- `<elapsed_s>` for running tools updates every redraw tick
  (`{elapsed:.1}s` formatting). For completed tools, `<duration_ms>`
  uses `{ms}ms` for <1s and `{s:.1}s` for ≥1s.
- Selection takes precedence over state glyph: a running + selected
  block renders as `▸ <name> <elapsed>` (yellow text untouched, blue
  glyph). Once selection clears (Esc), running blocks revert to `⠋`.
- Expansion takes precedence over selection: an expanded block renders
  as `▾`, not `▸`, even when also selected.

### Thinking spinner

Between turn dispatch (`StreamEvent::TurnStarted`) and the first
content event (any of `TextDelta` / `ThinkingDelta` /
`ToolDispatchStarted`), the latest `∴` glyph is followed by a spinner:

```
∴ ⠋
```

The spinner clears as soon as ANY content event arrives for that
turn. While tools run mid-turn, the `∴ ⠋` may reappear briefly
between a tool finishing and the next text token if cogito takes
non-zero time to decide its next action.

### Spinner animation source

Standard braille spinner sequence:

```
⠋  ⠙  ⠹  ⠸  ⠼  ⠴  ⠦  ⠧  ⠇  ⠏
```

Indexed by `(tick_counter / spinner_period_ticks) % frames.len()`
where `spinner_period_ticks` ≈ 3 ticks × 33ms = 99ms per frame
(comfortable for the eye, not too jittery).

---

## Interaction model

### Implicit focus (preserved from v0.1)

No mode toggle. Typing always goes to the input widget. Tool-block
navigation is keyboard-only via `Ctrl`-modified keys.

### Keymap

| Key | Action | Notes |
|-----|--------|-------|
| `Enter` | Submit non-empty input | Tool-block expansion is `Ctrl-Enter` (below), not bare `Enter` |
| `Shift+Enter` | Newline in input buffer | |
| `Ctrl-↑` / `Ctrl-↓` | Move tool-block selection across ALL tool blocks (including running) | Wraps within current visible chat, or across the full session — see "Open questions" |
| `Ctrl-Enter` | Expand/collapse the currently selected tool block | No-op when nothing is selected |
| `Alt-1` … `Alt-9` | Quick-expand the N-th most recent tool block in the entire session | `Alt-1` = most recent globally, `Alt-9` = 9th most recent. If fewer than N tool blocks exist, no-op. (`Ctrl`+digit is not reliably reported by terminals, hence `Alt`.) |
| `Ctrl-E` | Expand all tool blocks within the most recent cogito message | |
| `Ctrl-L` | Collapse all tool blocks within the most recent cogito message | `Ctrl-C` is taken by cancel/exit, so collapse uses `Ctrl-L`. |
| `PgUp` / `PgDn` | Scroll chat by 5 rows | Selection survives scroll |
| `Esc` | Dismiss slash popup | Clearing tool selection on `Esc` is planned but not yet implemented |
| `/` | Open slash command popup when typed at start of empty input | Behavior unchanged from v0.1 |
| `Ctrl-C` | Turn running: cancel turn. Idle + within 2s of prior Ctrl-C: exit. Idle + first press: arm 2s window + push hint notice | Unchanged from v0.1 |
| `Ctrl-D` | Exit when input buffer is empty | Unchanged from v0.1 |

**Removed from v0.1:** `Ctrl-T` (toggle tools pane).

### Selection rules

- Default state at startup: no selection; focus is the input.
- Selection set by: `Ctrl-↑/↓` (creates if none, moves if some) or `Alt-1`-`Alt-9` (creates pointing at the N-th most recent).
- Selection cleared by: quit. (Planned but not yet implemented in
  v0.2: explicit `Esc`, and any printable key typed into the input.)
- Selection survives chat scrolling (`PgUp` / `PgDn`). If the selected
  block is currently off-screen, no visible marker is rendered; the
  selection is restored when the user scrolls back. (No
  auto-scroll-to-selection on `Ctrl-↑/↓` in v0.2; future polish.)
- Running tool blocks ARE selectable (pre-select-then-wait pattern).

### Mouse: out of scope for this redesign.

### Slash popup

Behavior unchanged from v0.1. Popup appears centered above the input
when the first character of the buffer is `/`; lists prefix-matched
commands; `Esc` dismisses. v0.1 ships with only `/skill <name>`.

---

## Architecture impact

### Removed

| File | Replacement |
|------|-------------|
| `src/ui/tools.rs` | None — inline rendering inside `ui::chat`. |
| `src/ui/status.rs` | None — startup banner = SystemNotice lines pushed at App build. |
| `src/ui/mod.rs` `tools` + `status` submodule lines | Trimmed. |
| `App.show_tools` field | Removed. |
| `App.turn_count` rendered display | Field stays (telemetry); not rendered. |
| `Action::ToggleTools` (if any) / `Ctrl-T` dispatch | Removed. |

### Modified

| File | Change |
|------|--------|
| `src/ui/chat.rs` | Largest delta. Renders inline tool blocks (`⠋ ✓ ✗ ▸ ▾`) based on `ToolTreeModel` lookup keyed by the current cogito message. Role glyphs `▸` / `∴` replace text prefix. Reads spinner tick from a new `RenderInputs` field. |
| `src/ui/mod.rs` (top-level layout) | Single column: `chat | input_footer`. No horizontal split. No status row. Drop `show_tools` from `RenderInputs`. Add `spinner_tick: u64`. |
| `src/app.rs` | Drop `show_tools` field. Add `last_content_at: Option<Instant>` per turn for thinking-spinner gating (or derive from existing `turn_in_progress` + whether any chat lines yet for the current turn). |
| `src/keymap.rs` | Drop `Ctrl-T` branch. Add `1-9` quick-expand branch. Add `e` / `c` expand-all / collapse-all branch. |
| `src/runtime_build.rs` | Push startup-banner SystemNotices into ChatModel before returning the App. |
| `src/event_loop.rs` | Pass an incrementing `spinner_tick` into `RenderInputs`. Tiny change. |

### Added

| File | Purpose |
|------|---------|
| `src/ui/spinner.rs` (or inline helper) | `pub fn frame(tick: u64) -> &'static str` returning the next braille frame. Tiny module; ≤ 30 LOC. |
| `src/ui/banner.rs` (or inline in `runtime_build.rs`) | `pub fn startup_lines(model_id, strategy_name, session_id_str, version) -> Vec<String>` returning the 3 banner lines. |

### ChatLine refactor (minor)

`ChatLine::ToolStartLine { tool, args_preview }` and
`ChatLine::ToolEndLine { tool, ok, elapsed_ms, error }` are
**replaced** by a single `ChatLine::ToolBlock { call_id: String }`.
The renderer looks up current state (status, duration, args, result
preview) from `ToolTreeModel` via `call_id` at render time. This
keeps a single block per dispatch and lets selection / expansion
overlay the same logical row.

`ChatModel::on_event` for `ToolDispatchStarted` pushes one
`ToolBlock { call_id }`; `ToolDispatchEnded` becomes a no-op for
`ChatModel` (state is owned by `ToolTreeModel`). The
`TOOL_ARGS_PREVIEW_MAX` / `TOOL_ERROR_PREVIEW_MAX` / `tool_timers`
helpers move out of `ChatModel`; truncation happens in the renderer
or in `ToolTreeModel`'s existing fields.

### Preserved (no change)

- `ToolTreeModel` data structure (sink-agnostic).
- `StreamEvent` → tool-tree translation (`tree_tests` unaffected).
- Resume replay (`resume::translate_events` + `load_initial_state`)
  — the `synth_from_block` already emits paired
  `ToolDispatchStarted` + `ToolDispatchEnded`; ChatModel just stops
  emitting end lines on the second.
- Lazy tool-result lookup on first expand
  (`App::populate_result_preview`).
- Three-layer `TerminalGuard` (RAII + panic hook + SIGTERM/SIGHUP).
- Slash command parsing and dispatch (`slash::parse` + `slash::dispatch`).
- `runtime_build::build` overall structure (only adds banner push).

---

## Out of scope (deferred to subsequent sprints)

Captured from PR #27 review and brainstorm; explicitly NOT in this
redesign:

1. **Markdown rendering in chat.** Code blocks, bold/italic, inline
   code, lists. Needs `tui-markdown` or a custom span builder. Own
   sprint. *(Shipped — see
   `docs/superpowers/specs/2026-05-29-cogito-tui-markdown-design.md`
   and `docs/superpowers/plans/2026-05-29-cogito-tui-markdown.md`.)*
2. **Incremental ChatModel projection.** Current `Vec<ChatLine>` is
   reprojected from the event log every frame; long sessions cause
   frame drops. Cursor-based incremental projection. Needs a
   measurement pass first to size the work. Own sprint.
3. **Message animations.** Fade-in for new messages, typewriter
   cursor blink, smooth scroll. UX experiment. Own sprint.
4. **Theme system.** Palette swap, light/dark detection, custom
   themes. Future v0.3+.
5. **Mouse support.** `hover` + click-to-expand on tool blocks.
   Future polish.
6. **Markdown for tool args / results inside expansion blocks.**
   Covered by item 1. *(Still pending — item 1 shipped markdown for
   assistant text only; tool args / results remain raw text.)*

---

## Testing strategy

### Unit tests that survive unchanged

- `render_model::tree_tests::*` (ToolTreeModel + 8 cases) — unchanged.
- `slash::tests::*` (6 cases).
- `resume::tests::*` (translator) + `resume::extract_tests::*` (5+
  cases).

### Unit tests rewritten (`render_model::tests::*`)

`ChatModel` tests that assert on `ToolStartLine` / `ToolEndLine`
variants are rewritten against the new `ToolBlock { call_id }`
variant. The 9 existing cases become roughly:

- `text_delta_coalesces_within_block` — unchanged in shape.
- `thinking_delta_coalesces_within_block` — unchanged.
- `thinking_then_text_emits_two_lines` — unchanged.
- `tool_dispatch_emits_single_tool_block` (replaces
  `tool_dispatch_emits_start_and_end_lines`).
- (Drop `tool_args_preview_is_compact_json`, `tool_args_preview_truncates_at_limit`, `tool_error_message_truncates` — moved to renderer-side concerns.)
- `turn_paused_resumed_cancelled_failed_emit_notices` — unchanged.
- `user_prompt_breaks_text_coalescing` — unchanged.

### Unit tests rewritten

- `ui::chat::tests::*` — assertions updated to look for `▸` / `∴`
  glyphs + 3-space indent + inline tool block rendering (`⠋` / `✓` /
  `✗` / `▸` / `▾`).
- `keymap::tests::*` — remove `ctrl_t_toggles_show_tools`. Add tests:
  `digit_key_quick_expands_n_th_recent_tool` (3 cases), `e_expands_all_in_message`, `c_collapses_all_in_message`.
- `app::tests::*` — drop `show_tools` from the `App` test fixture; no other change expected.

### Unit tests deleted

- `ui::tools::tests::*` (8 cases) — pane gone.
- `ui::status::tests::*` (3 cases) — pane gone.
- `ui::tests::*` (top-level layout, 3 cases) — rewritten for new layout (count likely 2-3 new cases).

### New unit tests

- `ui::spinner::tests::frame_cycles_through_braille_sequence` (1 case).
- `ui::banner::tests::startup_lines_includes_model_strategy_session` (1 case).
- `ui::chat::tests::role_markers_render_with_correct_glyphs` (1 case covering `▸` and `∴`).
- `ui::chat::tests::tool_block_running_includes_spinner_frame` (1 case).
- `ui::chat::tests::tool_block_running_with_selection_shows_blue_marker` (1 case for mutex rule).
- `ui::chat::tests::tool_block_expanded_shows_args_and_result` (1 case).
- `keymap::tests::quick_expand_digit_keys` (3 cases for digits 1, 2, 9).
- `keymap::tests::e_and_c_expand_collapse_all_in_current_cogito_message` (2 cases).

### Integration tests

- `tests/e2e.rs` — existing `tool_lifecycle_renders_into_both_panes`
  test renames + updates assertion to look for `✓ read_file` instead
  of `[tool] read_file ok`. Other e2e tests unaffected.
- `tests/snapshot.rs` — rewrite all 5 cases. Add a banner-on-first-frame snapshot.
- `tests/resume.rs` — unchanged.
- `tests/list_strategies.rs` — unchanged.
- `tests/edges.rs` — resize / long-input / unicode tests rewritten
  against the new layout. Add a deep-tool-tree test verifying inline
  rendering of 50+ tool blocks in one cogito message.

### Total test count

V0.1 ships ~98 tests. Net change: ~−15 (pane deletions) + ~10 (new
cases) = **target ≥ 90 tests** for v0.2.

---

## Acceptance criteria

1. **Layout**: one chat column + input footer. No horizontal split.
   No `Borders::ALL` anywhere in persistent UI.
2. **No persistent status bar.** No `turn:` counter rendered. No
   `tools: on/off`. No key-hint strip.
3. **Startup banner** shows on first frame: `∴∴∴` (line 1) + project
   name + version (line 2) + `model · strategy · session_id[:8]`
   (line 3).
4. **MCP banner** lines appear immediately after the startup banner
   as dim notices (unchanged format from v0.1).
5. **Role markers**: `▸` (cyan) for user, `∴` (green) for cogito;
   3-space content indent.
6. **Tool block lifecycle glyphs**: `⠋` running (animated yellow),
   `✓` ok (green), `✗` err (red, with red `↳ message` line below),
   `▸` selected (cyan, mutex with state glyph), `▾` expanded (green,
   with `args` + `↳ result` lines indented).
7. **Thinking spinner** `∴ ⠋` between `TurnStarted` and the first
   content event of that turn; clears immediately on first content.
8. **Keymap**: `Ctrl-↑/↓` navigates tool selection across running
   + completed tools; `Enter` submits OR expands; `1-9` quick-expand;
   `e` / `c` expand-all / collapse-all in most recent cogito message;
   `Ctrl-T` removed.
9. **`make ci` green**; ≥ 90 cogito-tui tests pass.
10. **Manual smoke** confirms `∴∴∴` banner appears, role markers
    render, running tool spinner animates, thinking spinner clears
    on first token, all listed key bindings work, `Ctrl-D` on empty
    input exits cleanly with terminal restored.

---

## Risks and open questions

1. **Selection wrap-around scope.** Should `Ctrl-↑` from the
   most-recent tool wrap to the oldest tool in the SESSION, or only
   within the currently visible viewport? Lean toward
   "session-wide" since selection survives scroll. Confirm at
   implementation time.

2. **Tool-block placement when streaming overlaps.** Spec assumes
   blocks render at the position they were dispatched in stream
   order. A long-running tool dispatched early then text streamed
   past it stays at its dispatch position. Verify this matches user
   intuition during implementation; alternative is "float to bottom
   of cogito message until completion".

3. **Spinner CPU/redraw cost.** The 33ms redraw tick already exists;
   spinner just animates one cell per tick. No measurable regression
   expected. Confirm with `cargo bench` if a baseline exists.

4. **Color palette in 16-color terminals.** Existing v0.1 already
   uses `Color::Cyan` / `Green` / `Red` / `Yellow` which degrade
   gracefully. The `▸` / `∴` glyphs and braille spinner characters
   are valid Unicode in all major terminal fonts (JetBrains Mono,
   SF Mono, Consolas, etc.). Verified mentally; no font fallback work
   expected.

5. **Sprint slotting.** This redesign is not tagged to a numbered
   sprint. ROADMAP §"Sprint 10 (v0.1 hardening)" is the immediate
   next sprint. The redesign could:
   - (a) ride alongside Sprint 10 as a parallel work item (1-2 day
     overlap),
   - (b) become "Sprint 9c · TUI redesign" before Sprint 10, OR
   - (c) defer entirely to a "v0.2 polish" phase post-v0.1.0 tag.
   Implementation plan should pick one; my recommendation is **(b)**
   — small enough to land before v0.1 hardening, and the markdown +
   incremental-projection follow-ups naturally chain after it.

6. **Test count target.** ≥ 90 is a rough estimate. Actual count
   depends on how granular the new tests get. Final count surfaces
   in the implementation plan's testing section.

---

## Decisions log (brainstorm 2026-05-29)

| # | Question | Decision |
|---|----------|----------|
| 1 | Core dissatisfaction with v0.1 | (E) Visual layering + information architecture; tools pane has no real utility; turn counter shouldn't be persistent. |
| 2 | Where do tool calls go without the tools pane? | (A) Fully inline in chat as collapsible blocks (Claude Code style). |
| Visual direction (browser mockup) | A / B / C? | (A) Minimal Flat — no borders, dim horizontal separators. |
| Role marker style | A1 / A2 / A3 / A4? | (A1) Geometric symbols, zero text label. |
| Cogito symbol meaning | S1 / S2 / S3 / S4? | (S1) `▸` / `∴` — `∴` is the "therefore" of *cogito ergo sum*. |
| Chrome strategy | C1 / C2 / C3? | (C3) Ambient — startup banner scrolls away, no persistent chrome. User refinement: no tool list in banner, no model ghost text in input. |
| 3 | Thinking indicator | (B) `∴` followed by spinner during first-token wait; spinner clears on first text. |
| Tool lifecycle style | T1 / T2 / T3? | (T1) Status glyph (`⠋ ✓ ✗ ▸ ▾`). |
| 4a | Running tools selectable? | Yes — pre-select-then-wait pattern. |
| 4b | Selection on scroll | Survives scroll. |
| 4c | Mouse support | Out of scope for this redesign. |
| 4d | PR #27 follow-ups (markdown / perf / animations) | All deferred to subsequent sprints. This redesign is visual + interaction only. |

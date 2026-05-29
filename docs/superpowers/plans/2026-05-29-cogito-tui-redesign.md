# Cogito TUI Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Worktree:** before starting, use superpowers:using-git-worktrees to create an isolated branch (suggested name: `feat/tui-redesign`). All work happens in that worktree.

**Goal:** Land the cogito-tui v0.2 redesign from spec
`docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`:
single-column chat (no tools pane, no persistent status bar), `▸` / `∴`
role markers, inline expandable tool blocks with `⠋ ✓ ✗ ▸ ▾` glyphs,
`∴ ⠋` thinking spinner, ambient startup banner.

**Architecture:** Minor `ChatLine` refactor (one variant replaces
two), then a renderer rewrite that consults `ToolTreeModel` for tool
state, a layout simplification (drop horizontal split + status row),
new spinner + banner helpers, and keymap updates (drop Ctrl-T, add
`1-9` quick-expand, `e` / `c` expand-all / collapse-all).

**Tech Stack:** Rust 2024 (MSRV 1.85), `ratatui = 0.29`,
`crossterm = 0.29`, `tui-textarea = 0.7`. No new workspace deps.

---

## File structure

```
crates/cogito-tui/
  src/
    render_model.rs                MODIFY: ChatLine refactor (drop ToolStartLine + ToolEndLine, add ToolBlock { call_id })
    ui/
      mod.rs                       MODIFY: drop tools/status submodule decls; rewrite render() to single column; drop show_tools from RenderInputs; add spinner_tick
      chat.rs                      REWRITE: role markers ▸/∴, inline tool blocks (5 states), thinking spinner
      tools.rs                     DELETE
      status.rs                    DELETE
      spinner.rs                   CREATE: braille spinner frame helper
      banner.rs                    CREATE: startup banner SystemNotice lines
      input.rs                     unchanged
      popup.rs                     unchanged
    app.rs                         MODIFY: drop show_tools field, drop status_data StatusData dependency, add current_turn_thinking flag, drop StatusData import
    keymap.rs                      MODIFY: drop Ctrl-T branch; add 1-9 quick-expand; add e/c expand-all/collapse-all
    runtime_build.rs               MODIFY: push startup banner via ui::banner::startup_lines before returning App
    event_loop.rs                  MODIFY: maintain spinner_tick counter, pass to RenderInputs
  tests/
    e2e.rs                         REWRITE: assertions match new glyphs (✓ read_file replaces [tool] read_file ok); drop show_tools from helper
    snapshot.rs                    REWRITE: 5 cases against new layout; add banner-on-first-frame snapshot
    edges.rs                       REWRITE: resize / long-input / unicode / deep-tree tests against new layout
    resume.rs                      unchanged
    list_strategies.rs             unchanged
```

All assertions use the cell-symbol buffer collector pattern
(ratatui 0.29 `Buffer` has no `Display`); the helper already lives in
`tests/e2e.rs` and can be copied as-is.

---

## Phase 1: ChatLine refactor

### Task 1: Replace `ToolStartLine` + `ToolEndLine` with `ToolBlock { call_id }`

**Files:**
- Modify: `crates/cogito-tui/src/render_model.rs` (the `ChatLine` enum
  near line 20, `ChatModel::on_event` near line 130, the
  `TOOL_ARGS_PREVIEW_MAX` / `TOOL_ERROR_PREVIEW_MAX` / `tool_timers`
  helpers, and the `tests` module).

- [ ] **Step 1: Read the current file**

Run: `wc -l crates/cogito-tui/src/render_model.rs`
Expected: ~520 lines.

- [ ] **Step 2: Update the `ChatLine` enum**

Replace:

```rust
ToolStartLine {
    tool: String,
    args_preview: String,
},
ToolEndLine {
    tool: String,
    ok: bool,
    elapsed_ms: u128,
    error: Option<String>,
},
```

with:

```rust
/// Inline tool block. Renderer looks up current state in
/// `ToolTreeModel` via `call_id` at render time.
ToolBlock {
    /// `call_id` from the dispatcher; matches `StreamEvent` IDs and
    /// `ToolNode.call_id`.
    call_id: String,
},
```

- [ ] **Step 3: Remove the truncation constants and `ToolTimers`
  type alias**

Drop these items (they become render-side concerns; `ToolTreeModel`
already truncates the error message at `TOOL_ERROR_PREVIEW_MAX`):

```rust
pub const TOOL_ARGS_PREVIEW_MAX: usize = 200;
pub const TOOL_ERROR_PREVIEW_MAX: usize = 400;
type ToolTimers = HashMap<String, (Instant, String)>;
```

Also delete the `tool_timers: ToolTimers` field from `ChatModel`
and remove `use std::collections::HashMap;` / `use std::time::Instant;`
if no other code path uses them in this file (it doesn't).

- [ ] **Step 4: Rewrite `ChatModel::on_event` tool arms**

Replace the `ToolDispatchStarted { call_id, .. }` arm with:

```rust
StreamEvent::ToolDispatchStarted { call_id, .. } => {
    self.lines.push(ChatLine::ToolBlock {
        call_id: call_id.clone(),
    });
    self.in_text = false;
    self.in_thinking = false;
}
```

Replace the `ToolDispatchEnded { .. }` arm with:

```rust
StreamEvent::ToolDispatchEnded { .. } => {
    // State update lives in ToolTreeModel; ChatModel does nothing.
}
```

- [ ] **Step 5: Update the `tests` module**

Drop these tests entirely (their assertions are no longer meaningful):
- `tool_args_preview_is_compact_json`
- `tool_args_preview_truncates_at_limit`
- `tool_error_message_truncates`

Rewrite `tool_dispatch_emits_start_and_end_lines` →
`tool_dispatch_emits_single_tool_block`:

```rust
#[test]
fn tool_dispatch_emits_single_tool_block() {
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
    assert_eq!(m.lines.len(), 1);
    assert!(matches!(
        &m.lines[0],
        ChatLine::ToolBlock { call_id } if call_id == "c1"
    ));
}
```

Other `tests::*` cases (text coalescing, thinking coalescing, lifecycle
notices, user-prompt boundary) survive unchanged.

- [ ] **Step 6: Run render_model tests**

Run: `cargo nextest run -p cogito-tui render_model::tests`
Expected: 7 passed (was 9; we deleted 3 and added 1).

- [ ] **Step 7: Verify ToolTreeModel tests still pass**

Run: `cargo nextest run -p cogito-tui render_model::tree_tests`
Expected: 8 passed, unchanged.

- [ ] **Step 8: Whole-crate compile (some files now broken)**

Run: `cargo check -p cogito-tui 2>&1 | head -20`
Expected: errors in `src/ui/chat.rs`, `tests/e2e.rs`, `tests/edges.rs`,
`tests/snapshot.rs` referencing the removed variants. These are fixed
by later tasks (4, 11, 12, 13). **Do not commit yet** — we need at
least Task 4 to land before things compile again.

- [ ] **Step 9: Hold the commit; proceed to Task 2**

Reason: committing now leaves `cargo check` red, which violates
"frequent commits while keeping the tree green". The ChatLine refactor
+ chat renderer rewrite + test updates are a single logical change.
We'll commit at the end of Task 4.

---

## Phase 2: Building blocks

### Task 2: `ui::spinner` module

**Files:**
- Create: `crates/cogito-tui/src/ui/spinner.rs`
- Modify: `crates/cogito-tui/src/ui/mod.rs` (add `pub mod spinner;`)

- [ ] **Step 1: Write the file**

```rust
//! Braille spinner — animates running tools and the
//! between-content thinking marker (spec §"Spinner animation source").
//!
//! Frames cycle through the conventional 10-frame braille sequence.
//! Index by `tick / period`, where `tick` is the redraw counter and
//! `period` is chosen so visible frame rate is ~10 Hz at the 33ms
//! redraw cadence (period = 3 ticks ≈ 99ms per frame).

/// One frame per advance; advance once every `PERIOD_TICKS` redraw
/// ticks. Lower = faster (busier); higher = calmer.
pub const PERIOD_TICKS: u64 = 3;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Return the spinner glyph for the given redraw tick.
#[must_use]
pub fn frame(tick: u64) -> &'static str {
    let len = u64::try_from(FRAMES.len()).unwrap_or(1);
    let idx = (tick / PERIOD_TICKS) % len;
    FRAMES[usize::try_from(idx).unwrap_or(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_zero_is_first_frame() {
        assert_eq!(frame(0), "⠋");
    }

    #[test]
    fn frame_advances_every_period_ticks() {
        assert_eq!(frame(0), frame(PERIOD_TICKS - 1));
        assert_ne!(frame(0), frame(PERIOD_TICKS));
        assert_eq!(frame(PERIOD_TICKS), "⠙");
    }

    #[test]
    fn frame_cycles_through_all_braille_frames() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..(PERIOD_TICKS * 10) {
            seen.insert(frame(i));
        }
        assert_eq!(seen.len(), 10);
    }
}
```

- [ ] **Step 2: Register the module in `ui/mod.rs`**

Edit `crates/cogito-tui/src/ui/mod.rs`. In the submodule declarations
near the top, add `pub mod spinner;` (alphabetical between `popup`
and `status` — or just append; ordering is cosmetic):

Current block (top of file):

```rust
pub mod chat;
pub mod input;
pub mod popup;
pub mod status;
pub mod tools;
```

Add `spinner` (leave `status`/`tools` for now; they get deleted in
Task 6):

```rust
pub mod chat;
pub mod input;
pub mod popup;
pub mod spinner;
pub mod status;
pub mod tools;
```

- [ ] **Step 3: Run the spinner tests**

Run: `cargo nextest run -p cogito-tui ui::spinner`
Expected: 3 tests passed.

- [ ] **Step 4: Hold the commit**

Same reasoning as Task 1 — Phase 1 already left the tree red. We bundle
through to Task 4.

---

### Task 3: `ui::banner` module

**Files:**
- Create: `crates/cogito-tui/src/ui/banner.rs`
- Modify: `crates/cogito-tui/src/ui/mod.rs` (`pub mod banner;`)

- [ ] **Step 1: Write the file**

```rust
//! Startup banner — three SystemNotice lines pushed into
//! `ChatModel` at App build time. Scrolls away naturally with chat
//! history (spec §"Chrome strategy: Ambient (C3)").

/// Build the three banner lines.
///
/// Layout:
///
/// ```text
///    ∴∴∴
///    cogito  v0.2
///    <model_id>  ·  <strategy_name>  ·  <session_id[:8]>
/// ```
///
/// All lines have a 3-space leading indent so they align with the
/// chat content column.
#[must_use]
pub fn startup_lines(model_id: &str, strategy_name: &str, session_id: &str) -> Vec<String> {
    let version = env!("CARGO_PKG_VERSION");
    let short_session: String = session_id.chars().take(8).collect();
    vec![
        "   ∴∴∴".to_string(),
        format!("   cogito  v{version}"),
        format!("   {model_id}  ·  {strategy_name}  ·  {short_session}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_lines_contains_three_rows() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn startup_lines_first_row_is_sigil() {
        let v = startup_lines("m", "s", "sid");
        assert!(v[0].contains("∴∴∴"));
    }

    #[test]
    fn startup_lines_second_row_carries_version() {
        let v = startup_lines("m", "s", "sid");
        let version = env!("CARGO_PKG_VERSION");
        assert!(v[1].contains("cogito"));
        assert!(v[1].contains(version));
    }

    #[test]
    fn startup_lines_third_row_carries_identity() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        assert!(v[2].contains("opus-4.7"));
        assert!(v[2].contains("coder"));
        assert!(v[2].contains("01abcdef"));
        assert!(!v[2].contains("01abcdefghij"));
    }
}
```

- [ ] **Step 2: Register the module in `ui/mod.rs`**

Add `pub mod banner;` to the submodule list.

- [ ] **Step 3: Run banner tests**

Run: `cargo nextest run -p cogito-tui ui::banner`
Expected: 4 tests passed.

- [ ] **Step 4: Hold the commit; proceed to Task 4**

---

## Phase 3: Chat renderer rewrite (the central change)

### Task 4: `ui::chat::render` with role markers + inline tool blocks + thinking spinner

**Files:**
- Modify (essentially rewrite): `crates/cogito-tui/src/ui/chat.rs`

- [ ] **Step 1: Replace `crates/cogito-tui/src/ui/chat.rs` with the
  full rewrite below**

```rust
//! Chat pane widget — renders `ChatModel.lines` as ratatui `Line`s
//! with palette applied at draw time (lazy painting; spec §"Visual
//! language").
//!
//! Inline tool blocks: when a `ChatLine::ToolBlock { call_id }` is
//! encountered the renderer looks up the current `ToolNode` in
//! `ToolTreeModel` and paints the appropriate lifecycle glyph
//! (`⠋ ✓ ✗ ▸ ▾`) plus optional expanded args + result preview
//! (spec §"Tool block lifecycle (T1 — Status Glyph)").
//!
//! Thinking spinner: when `turn_in_progress = true` and no content
//! event has arrived for the current turn yet (`current_turn_thinking
//! = true`), render an extra `∴ ⠋` line at the end of the chat
//! scrollback (spec §"Thinking spinner").

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::render_model::{ChatLine, ChatModel, ToolNode, ToolStatus, ToolTreeModel, TreePath};
use crate::ui::spinner;

/// Max args-preview chars rendered next to an expanded block.
const EXPAND_ARGS_MAX: usize = 200;
/// Max result-preview lines rendered under an expanded block.
const EXPAND_RESULT_LINES: usize = 12;
/// 3-space content indent (matches role markers).
const INDENT: &str = "   ";

/// Borrowed inputs for `render`. Bundled in a struct because the
/// chat pane now consults the tool tree, the selection set, the
/// expanded set, the lifecycle flags, and the spinner tick.
pub struct ChatRenderInputs<'a> {
    /// Chat scrollback.
    pub chat: &'a ChatModel,
    /// Tool tree (state lookup by `call_id`).
    pub tools: &'a ToolTreeModel,
    /// Currently selected `(turn_idx_in_vec, node_idx)`.
    pub selected: Option<TreePath>,
    /// Set of expanded paths.
    pub expanded: &'a HashSet<TreePath>,
    /// `true` between `TurnStarted | ToolDispatchEnded` and the next
    /// content event for this turn.
    pub turn_thinking: bool,
    /// Redraw tick counter (drives spinner animation).
    pub spinner_tick: u64,
}

struct Palette {
    user: Style,
    cogito: Style,
    thinking: Style,
    error: Style,
    notice: Style,
    dim: Style,
    sel: Style,
    ok: Style,
    running: Style,
}

impl Palette {
    fn default_dark() -> Self {
        Self {
            user: Style::default().fg(Color::Cyan),
            cogito: Style::default().fg(Color::Green),
            thinking: Style::default().add_modifier(Modifier::DIM),
            error: Style::default().fg(Color::Red),
            notice: Style::default().add_modifier(Modifier::DIM),
            dim: Style::default().add_modifier(Modifier::DIM),
            sel: Style::default().fg(Color::Cyan),
            ok: Style::default().fg(Color::Green),
            running: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::DIM),
        }
    }
}

/// Render the chat pane into `area`.
pub fn render(f: &mut Frame, area: Rect, inputs: &ChatRenderInputs<'_>) {
    let p = Palette::default_dark();
    let mut out: Vec<Line<'static>> = Vec::new();
    for line in &inputs.chat.lines {
        match line {
            ChatLine::UserPrompt(text) => out.push(user_line(text, &p)),
            ChatLine::AssistantText(text) => out.push(cogito_line(text, &p)),
            ChatLine::AssistantThinking(text) => out.push(thinking_line(text, &p)),
            ChatLine::SystemNotice(text) => out.push(notice_line(text, &p)),
            ChatLine::ToolBlock { call_id } => {
                render_tool_block(&mut out, call_id, inputs, &p);
            }
        }
    }
    if inputs.turn_thinking {
        out.push(Line::from(vec![
            Span::styled("∴ ", p.cogito),
            Span::styled(spinner::frame(inputs.spinner_tick), p.running),
        ]));
    }
    let para = Paragraph::new(out)
        .wrap(Wrap { trim: false })
        .scroll((inputs.chat.scroll_offset, 0));
    f.render_widget(para, area);
}

fn user_line(text: &str, p: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled("▸  ", p.user),
        Span::raw(text.to_string()),
    ])
}

fn cogito_line(text: &str, p: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled("∴  ", p.cogito),
        Span::raw(text.to_string()),
    ])
}

fn thinking_line(text: &str, p: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled("∴  ", p.thinking),
        Span::styled(text.to_string(), p.thinking),
    ])
}

fn notice_line(text: &str, p: &Palette) -> Line<'static> {
    let style = if text.starts_with("[error]") {
        p.error
    } else {
        p.notice
    };
    Line::from(vec![Span::styled(text.to_string(), style)])
}

/// Resolve `call_id` to a `(TreePath, &ToolNode)` if present.
fn lookup(tools: &ToolTreeModel, call_id: &str) -> Option<(TreePath, ToolNode)> {
    for (t_idx, group) in tools.turns.iter().enumerate() {
        for (n_idx, node) in group.nodes.iter().enumerate() {
            if node.call_id == call_id {
                return Some(((t_idx, n_idx), node.clone()));
            }
        }
    }
    None
}

fn render_tool_block(
    out: &mut Vec<Line<'static>>,
    call_id: &str,
    inputs: &ChatRenderInputs<'_>,
    p: &Palette,
) {
    let Some((path, node)) = lookup(inputs.tools, call_id) else {
        // Tool tree hasn't ingested the event yet; defensively render
        // a dim placeholder.
        out.push(Line::from(vec![Span::styled(
            format!("{INDENT}? <unknown tool>"),
            p.dim,
        )]));
        return;
    };
    let is_selected = inputs.selected == Some(path);
    let is_expanded = inputs.expanded.contains(&path);
    out.push(tool_header_line(&node, is_selected, is_expanded, inputs, p));
    if matches!(node.status, ToolStatus::Err { .. }) {
        // Error message always shown inline under failed tools.
        if let ToolStatus::Err { message, .. } = &node.status {
            for msg_line in message.lines() {
                out.push(Line::from(vec![Span::styled(
                    format!("{INDENT}  ↳ {msg_line}"),
                    p.error,
                )]));
            }
        }
    }
    if is_expanded {
        // Args row.
        let args = serde_json::to_string(&node.args).unwrap_or_else(|_| "<unencodable>".into());
        let args_trim: String = args.chars().take(EXPAND_ARGS_MAX).collect();
        let args_suffix = if args.chars().count() > EXPAND_ARGS_MAX {
            "..."
        } else {
            ""
        };
        out.push(Line::from(vec![Span::styled(
            format!("{INDENT}  args   {args_trim}{args_suffix}"),
            p.dim,
        )]));
        // Result preview row (skipped when error already printed).
        if !matches!(node.status, ToolStatus::Err { .. }) {
            match &node.result_preview {
                Some(preview) => {
                    for (i, line) in preview.lines().enumerate() {
                        if i >= EXPAND_RESULT_LINES {
                            out.push(Line::from(vec![Span::styled(
                                format!("{INDENT}  ↳ ..."),
                                p.dim,
                            )]));
                            break;
                        }
                        out.push(Line::from(vec![Span::styled(
                            format!("{INDENT}  ↳ {line}"),
                            p.dim,
                        )]));
                    }
                }
                None => out.push(Line::from(vec![Span::styled(
                    format!("{INDENT}  (loading result...)"),
                    p.dim,
                )])),
            }
        }
    }
}

fn tool_header_line(
    node: &ToolNode,
    is_selected: bool,
    is_expanded: bool,
    inputs: &ChatRenderInputs<'_>,
    p: &Palette,
) -> Line<'static> {
    // Glyph priority: expanded > selected > state.
    let (glyph, glyph_style) = if is_expanded {
        ("▾", p.ok)
    } else if is_selected {
        ("▸", p.sel)
    } else {
        match &node.status {
            ToolStatus::Running => (spinner::frame(inputs.spinner_tick), p.running),
            ToolStatus::Ok { .. } => ("✓", p.ok),
            ToolStatus::Err { .. } => ("✗", p.error),
        }
    };
    let duration = match &node.status {
        ToolStatus::Running => format!("{:.1}s", node.started_at.elapsed().as_secs_f32()),
        ToolStatus::Ok { elapsed_ms } | ToolStatus::Err { elapsed_ms, .. } => {
            format_ms(*elapsed_ms)
        }
    };
    Line::from(vec![
        Span::raw(INDENT.to_string()),
        Span::styled(format!("{glyph} "), glyph_style),
        Span::raw(node.tool_name.clone()),
        Span::raw(" ".repeat(2)),
        Span::styled(duration, p.dim),
    ])
}

fn format_ms(ms: u128) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::stream::StreamEvent;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    fn draw(
        chat: &ChatModel,
        tools: &ToolTreeModel,
        selected: Option<TreePath>,
        expanded: &HashSet<TreePath>,
        turn_thinking: bool,
        tick: u64,
        w: u16,
        h: u16,
    ) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &ChatRenderInputs {
                        chat,
                        tools,
                        selected,
                        expanded,
                        turn_thinking,
                        spinner_tick: tick,
                    },
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let width = buf.area().width as usize;
        buf.content()
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn empty_tools() -> ToolTreeModel {
        ToolTreeModel::new()
    }

    #[test]
    fn empty_model_renders_nothing_substantive() {
        let chat = ChatModel::new();
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        assert!(!out.contains('▸'), "got: {out}");
        assert!(!out.contains('∴'), "got: {out}");
    }

    #[test]
    fn user_prompt_renders_with_marker_prefix() {
        let mut chat = ChatModel::new();
        chat.push_user_prompt("hello".into());
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        assert!(out.contains("▸  hello"), "got:\n{out}");
    }

    #[test]
    fn assistant_text_renders_with_marker_prefix() {
        let mut chat = ChatModel::new();
        chat.on_event(&StreamEvent::TextDelta {
            chunk: "I am cogito.".into(),
        });
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        assert!(out.contains("∴  I am cogito."), "got:\n{out}");
    }

    #[test]
    fn thinking_spinner_appears_when_turn_thinking() {
        let chat = ChatModel::new();
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), true, 0, 40, 5);
        assert!(out.contains("∴ ⠋"), "got:\n{out}");
    }

    #[test]
    fn thinking_spinner_absent_when_not_thinking() {
        let chat = ChatModel::new();
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        assert!(!out.contains('⠋'), "got:\n{out}");
    }

    fn build_tools_with_one(call_id: &str, name: &str) -> (ChatModel, ToolTreeModel) {
        let mut chat = ChatModel::new();
        let mut tools = ToolTreeModel::new();
        for ev in [
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: call_id.into(),
                tool_name: name.into(),
                args: json!({"path": "a.rs"}),
            },
        ] {
            chat.on_event(&ev);
            tools.on_event(&ev);
        }
        (chat, tools)
    }

    #[test]
    fn running_tool_renders_with_spinner_glyph() {
        let (chat, tools) = build_tools_with_one("c1", "read_file");
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 60, 8);
        assert!(out.contains("read_file"), "got:\n{out}");
        // First spinner frame is "⠋".
        assert!(out.contains('⠋'), "got:\n{out}");
    }

    #[test]
    fn completed_ok_tool_renders_with_check_glyph() {
        let (mut chat, mut tools) = build_tools_with_one("c1", "read_file");
        for ev in [StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        }] {
            chat.on_event(&ev);
            tools.on_event(&ev);
        }
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 60, 8);
        assert!(out.contains('✓'), "got:\n{out}");
    }

    #[test]
    fn failed_tool_renders_cross_and_error_message() {
        let (mut chat, mut tools) = build_tools_with_one("c1", "run_tests");
        for ev in [StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: false,
            error_message: Some("panicked: assertion failed".into()),
        }] {
            chat.on_event(&ev);
            tools.on_event(&ev);
        }
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 60, 8);
        assert!(out.contains('✗'), "got:\n{out}");
        assert!(out.contains("panicked"), "got:\n{out}");
    }

    #[test]
    fn selected_tool_renders_cyan_arrow_marker_overriding_state() {
        let (chat, tools) = build_tools_with_one("c1", "read_file");
        let out = draw(&chat, &tools, Some((0, 0)), &HashSet::new(), false, 0, 60, 8);
        assert!(out.contains("▸ read_file"), "got:\n{out}");
    }

    #[test]
    fn expanded_completed_tool_renders_args_and_result_placeholder() {
        let (mut chat, mut tools) = build_tools_with_one("c1", "read_file");
        for ev in [StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        }] {
            chat.on_event(&ev);
            tools.on_event(&ev);
        }
        let mut expanded = HashSet::new();
        expanded.insert((0, 0));
        let out = draw(&chat, &tools, Some((0, 0)), &expanded, false, 0, 80, 12);
        assert!(out.contains('▾'), "got:\n{out}");
        assert!(out.contains("args"), "got:\n{out}");
        assert!(out.contains("path"), "got:\n{out}");
        assert!(out.contains("(loading result"), "got:\n{out}");
    }
}
```

- [ ] **Step 2: Run chat tests**

Run: `cargo nextest run -p cogito-tui ui::chat`
Expected: 10 tests passed.

- [ ] **Step 3: Verify crate still has top-level breakage in
  `ui/mod.rs` (expected — fixed in Task 5)**

Run: `cargo check -p cogito-tui 2>&1 | head -10`
Expected: errors in `src/ui/mod.rs` (still uses old `chat::render`
signature + `tools::render` + `status::render`).

- [ ] **Step 4: Hold the commit; proceed to Task 5**

---

## Phase 4: Top-level layout

### Task 5: Rewrite `ui::mod.rs` for single-column layout

**Files:**
- Modify (rewrite): `crates/cogito-tui/src/ui/mod.rs`

- [ ] **Step 1: Replace the file with**

```rust
//! UI surface — top-level `render` orchestrates the single chat
//! column + input footer (spec §"Chrome strategy"). No persistent
//! status row; no tools pane.

pub mod banner;
pub mod chat;
pub mod input;
pub mod popup;
pub mod spinner;
pub mod status; // TODO Task 6: delete after dependents drop the reference
pub mod tools; // TODO Task 6: delete after dependents drop the reference

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::render_model::{ChatModel, ToolTreeModel, TreePath};
use crate::ui::chat::ChatRenderInputs;
use crate::ui::input::InputWidget;

/// Top-level render inputs. One chat column + an input footer. No
/// `show_tools`, no separate status pane.
pub struct RenderInputs<'a> {
    /// Chat scrollback.
    pub chat: &'a ChatModel,
    /// Tool tree (for inline tool block lookup).
    pub tools: &'a ToolTreeModel,
    /// Selected tool path.
    pub selected: Option<TreePath>,
    /// Expanded tool paths.
    pub expanded: &'a HashSet<TreePath>,
    /// Multi-line input widget.
    pub input: &'a InputWidget,
    /// Lifecycle: `true` between TurnStarted | ToolDispatchEnded and
    /// the next content event.
    pub turn_thinking: bool,
    /// Redraw counter for spinner animation.
    pub spinner_tick: u64,
    /// Slash popup prefix when `Some`.
    pub popup_prefix: Option<&'a str>,
}

/// Render one frame.
pub fn render(f: &mut Frame, inputs: &RenderInputs<'_>) {
    let area = f.area();
    let input_h = inputs.input.desired_height();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),         // chat
            Constraint::Length(1),      // dim divider
            Constraint::Length(input_h),
        ])
        .split(area);
    let chat_area = outer[0];
    let divider_area = outer[1];
    let input_area = outer[2];

    crate::ui::chat::render(
        f,
        chat_area,
        &ChatRenderInputs {
            chat: inputs.chat,
            tools: inputs.tools,
            selected: inputs.selected,
            expanded: inputs.expanded,
            turn_thinking: inputs.turn_thinking,
            spinner_tick: inputs.spinner_tick,
        },
    );

    // Single dim horizontal rule above the input.
    let rule = "─".repeat(divider_area.width as usize);
    let divider = Paragraph::new(Line::from(rule)).style(Style::default().fg(Color::DarkGray));
    f.render_widget(divider, divider_area);

    inputs.input.render(f, input_area);

    if let Some(prefix) = inputs.popup_prefix {
        crate::ui::popup::render(f, area, prefix);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::stream::StreamEvent;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw_buf(with_text: bool, popup_prefix: Option<&str>, w: u16, h: u16) -> String {
        let mut chat = ChatModel::new();
        if with_text {
            chat.push_user_prompt("hi".into());
            chat.on_event(&StreamEvent::TextDelta {
                chunk: "hello".into(),
            });
        }
        let tools = ToolTreeModel::new();
        let input = InputWidget::new();
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
                        turn_thinking: false,
                        spinner_tick: 0,
                        popup_prefix,
                    },
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let width = buf.area().width as usize;
        buf.content()
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn single_column_layout_has_no_tools_pane() {
        let out = draw_buf(true, None, 80, 20);
        assert!(out.contains("▸  hi"), "got:\n{out}");
        assert!(out.contains("∴  hello"), "got:\n{out}");
        // No "tools" pane title anywhere.
        assert!(!out.contains("tools "), "tools pane should be absent:\n{out}");
    }

    #[test]
    fn divider_row_renders_above_input() {
        let out = draw_buf(false, None, 40, 6);
        // The divider line uses '─' characters.
        assert!(out.contains('─'), "expected divider, got:\n{out}");
    }

    #[test]
    fn popup_overlays_above_input_when_prefix_set() {
        let out = draw_buf(false, Some("/sk"), 80, 20);
        assert!(out.contains("commands"), "popup title missing:\n{out}");
        assert!(out.contains("/skill"), "popup entry missing:\n{out}");
    }
}
```

- [ ] **Step 2: Run the layout tests**

Run: `cargo nextest run -p cogito-tui ui::tests`
Expected: 3 tests passed.

- [ ] **Step 3: Whole-crate compile (still red for status / tools)**

Run: `cargo check -p cogito-tui 2>&1 | head -10`
Expected: warnings or errors about unused `ui::status` /
`ui::tools` (since `RenderInputs` no longer carries `StatusData` or
toggles tools).  Plus errors in `app.rs` (`status_data()` still
references `StatusData`) and `runtime_build.rs` / `event_loop.rs` /
integration tests using the old `RenderInputs` shape. All addressed
in Tasks 6-13.

- [ ] **Step 4: Hold commit; proceed to Task 6**

---

## Phase 5: Drop the dead panes

### Task 6: Delete `ui/status.rs` and `ui/tools.rs`

**Files:**
- Delete: `crates/cogito-tui/src/ui/status.rs`
- Delete: `crates/cogito-tui/src/ui/tools.rs`
- Modify: `crates/cogito-tui/src/ui/mod.rs` (drop the two TODO submodule decls)

- [ ] **Step 1: Delete the files**

```bash
rm crates/cogito-tui/src/ui/status.rs
rm crates/cogito-tui/src/ui/tools.rs
```

- [ ] **Step 2: Drop the submodule declarations**

Edit `crates/cogito-tui/src/ui/mod.rs`. Remove the two lines (and the
`// TODO Task 6: …` comments):

```rust
pub mod status;
pub mod tools;
```

The final module declaration block should be:

```rust
pub mod banner;
pub mod chat;
pub mod input;
pub mod popup;
pub mod spinner;
```

- [ ] **Step 3: Compile check**

Run: `cargo check -p cogito-tui 2>&1 | head -30`
Expected: errors in `app.rs` (`use crate::ui::status::StatusData;`,
`status_data()` body) and possibly `runtime_build.rs` (banner write
path). Tasks 7-9 fix these.

- [ ] **Step 4: Hold commit; proceed to Task 7**

---

## Phase 6: App state + keymap

### Task 7: Trim `App` (drop `show_tools`, drop `StatusData`, add `current_turn_thinking`)

**Files:**
- Modify: `crates/cogito-tui/src/app.rs`

- [ ] **Step 1: Drop the `StatusData` import and `status_data()` method**

Edit `crates/cogito-tui/src/app.rs`. Remove:

```rust
use crate::ui::status::StatusData;
```

and remove the entire `status_data()` method (currently at lines ~107-117).

Drop the `show_tools` field from the struct (currently at line ~67-68):

```rust
/// Whether the tools pane is visible (`Ctrl-T` toggle).
pub show_tools: bool,
```

- [ ] **Step 2: Add `current_turn_thinking` field**

Insert (alphabetical/logical placement near `turn_in_progress`):

```rust
/// True between `TurnStarted | ToolDispatchEnded` and the next
/// content event for this turn. Drives the `∴ ⠋` thinking spinner
/// (spec §"Thinking spinner").
pub current_turn_thinking: bool,
```

- [ ] **Step 3: Update `apply_stream_event` to maintain
  `current_turn_thinking`**

Replace the match block in `apply_stream_event` with:

```rust
match ev {
    StreamEvent::TurnStarted => {
        self.turn_in_progress = true;
        self.current_turn_thinking = true;
    }
    StreamEvent::TurnCompleted => {
        self.turn_in_progress = false;
        self.current_turn_thinking = false;
        self.turn_count = self.turn_count.saturating_add(1);
    }
    StreamEvent::TurnFailed { .. }
    | StreamEvent::TurnCancelled
    | StreamEvent::TurnPaused => {
        self.turn_in_progress = false;
        self.current_turn_thinking = false;
    }
    StreamEvent::TextDelta { .. } | StreamEvent::ThinkingDelta { .. } => {
        self.current_turn_thinking = false;
    }
    StreamEvent::ToolDispatchStarted { .. } => {
        self.current_turn_thinking = false;
    }
    StreamEvent::ToolDispatchEnded { .. } => {
        // Spinner reappears between tool end and next content.
        if self.turn_in_progress {
            self.current_turn_thinking = true;
        }
    }
    _ => {}
}
```

(The two model `on_event` calls at the top of `apply_stream_event`
stay unchanged.)

- [ ] **Step 4: Update the `tests::app_for_pure_test` constructor**

Drop the `show_tools: true,` line. Add `current_turn_thinking: false,`
in the App literal.

- [ ] **Step 5: Drop the `status_data_mirrors_app_state` test**

It tested `status_data()` which no longer exists.

- [ ] **Step 6: Run app tests**

Run: `cargo nextest run -p cogito-tui app`
Expected: 5 tests passed (was 6; we dropped one).

- [ ] **Step 7: Compile check**

Run: `cargo check -p cogito-tui 2>&1 | head -20`
Expected: errors in `keymap.rs` (uses `app.show_tools`) and possibly
elsewhere. Task 8 fixes keymap.

- [ ] **Step 8: Hold commit; proceed to Task 8**

---

### Task 8: Rewrite keymap (drop Ctrl-T, add 1-9, add e/c)

**Files:**
- Modify: `crates/cogito-tui/src/keymap.rs`

- [ ] **Step 1: Update the `Action` enum**

Add a new variant near the others:

```rust
/// Quick-expand the N-th most recent tool block in the entire
/// session (N = 1..=9). Pushes the `(path, true)` analogue of
/// `ExpandNode` but separate to make it explicit at the action layer.
ExpandRecent {
    /// 1-based recency index (1 = most recent).
    n: u8,
},
/// Expand all tool blocks in the most recent cogito message.
ExpandAllInLatestMessage,
/// Collapse all tool blocks in the most recent cogito message.
CollapseAllInLatestMessage,
```

(`ExpandNode` stays.)

- [ ] **Step 2: Remove the Ctrl-T branch in `dispatch`**

Delete the block:

```rust
// Ctrl-T toggles tools pane.
if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
    app.show_tools = !app.show_tools;
    return Action::None;
}
```

Also update the module doc comment at the top: drop the
`Ctrl-T -> toggle tools pane visibility` bullet; add bullets for
`1-9 -> quick-expand`, `e -> expand all in latest message`,
`c -> collapse all in latest message`.

- [ ] **Step 3: Add digit + e/c branches**

Insert (after the Ctrl-Enter branch, before the default "route to
input" block):

```rust
// 1-9: quick-expand N-th most recent tool block (session-wide).
if let KeyCode::Char(ch) = key.code
    && key.modifiers.is_empty()
    && let Some(n) = digit_index(ch)
{
    return quick_expand(app, n);
}

// 'e' / 'c': expand-all / collapse-all in latest cogito message.
if key.modifiers.is_empty() {
    match key.code {
        KeyCode::Char('e') => return expand_all_latest(app),
        KeyCode::Char('c') => return collapse_all_latest(app),
        _ => {}
    }
}
```

- [ ] **Step 4: Add helper functions**

Append (next to `expand_selected`):

```rust
/// Map a key character `'1'..='9'` to a 1-based index, otherwise None.
fn digit_index(ch: char) -> Option<u8> {
    if ('1'..='9').contains(&ch) {
        let n = (ch as u32 - '0' as u32) as u8;
        Some(n)
    } else {
        None
    }
}

/// Find the `n`-th most recent tool (1 = most recent) across all
/// turns; toggle expansion; return the appropriate Action.
fn quick_expand(app: &mut App, n: u8) -> Action {
    // Flatten all (TreePath, &ToolNode) in reverse order; pick n-th.
    let mut flat: Vec<crate::render_model::TreePath> = Vec::new();
    for (t_idx, group) in app.tools.turns.iter().enumerate().rev() {
        for n_idx in (0..group.nodes.len()).rev() {
            flat.push((t_idx, n_idx));
        }
    }
    let Some(path) = flat.get(usize::from(n - 1)).copied() else {
        return Action::None;
    };
    // Same finished-only restriction as Ctrl-Enter expansion.
    let finished = app
        .tools
        .turns
        .get(path.0)
        .and_then(|g| g.nodes.get(path.1))
        .is_some_and(|node| node.status.is_finished());
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

/// Expand every finished node in the most recent turn group.
fn expand_all_latest(app: &mut App) -> Action {
    let Some(last_t) = app.tools.turns.len().checked_sub(1) else {
        return Action::None;
    };
    if let Some(group) = app.tools.turns.get(last_t) {
        for (n_idx, node) in group.nodes.iter().enumerate() {
            if node.status.is_finished() {
                app.expanded.insert((last_t, n_idx));
            }
        }
    }
    Action::ExpandAllInLatestMessage
}

fn collapse_all_latest(app: &mut App) -> Action {
    let Some(last_t) = app.tools.turns.len().checked_sub(1) else {
        return Action::None;
    };
    if let Some(group) = app.tools.turns.get(last_t) {
        for (n_idx, _) in group.nodes.iter().enumerate() {
            app.expanded.remove(&(last_t, n_idx));
        }
    }
    Action::CollapseAllInLatestMessage
}
```

- [ ] **Step 5: Update tests**

Drop `ctrl_t_toggles_show_tools` entirely.

Add new tests at the bottom of the `tests` module:

```rust
#[test]
fn digit_one_quick_expands_most_recent_finished_tool() {
    let (mut app, _td) = fresh_app();
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
    let a = dispatch(&mut app, k(KeyCode::Char('1'), KeyModifiers::NONE));
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
fn digit_for_n_greater_than_available_is_noop() {
    let (mut app, _td) = fresh_app();
    let a = dispatch(&mut app, k(KeyCode::Char('5'), KeyModifiers::NONE));
    assert_eq!(a, Action::None);
}

#[test]
fn digit_on_running_tool_is_noop() {
    let (mut app, _td) = fresh_app();
    app.tools.on_event(&StreamEvent::TurnStarted);
    app.tools.on_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c".into(),
        tool_name: "t".into(),
        args: json!({}),
    });
    let a = dispatch(&mut app, k(KeyCode::Char('1'), KeyModifiers::NONE));
    assert_eq!(a, Action::None);
    assert!(app.expanded.is_empty());
}

#[test]
fn e_expands_all_finished_in_latest_message() {
    let (mut app, _td) = fresh_app();
    app.tools.on_event(&StreamEvent::TurnStarted);
    for id in ["c1", "c2"] {
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: id.into(),
            tool_name: id.into(),
            args: json!({}),
        });
        app.tools.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: id.into(),
            ok: true,
            error_message: None,
        });
    }
    let a = dispatch(&mut app, k(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(a, Action::ExpandAllInLatestMessage);
    assert!(app.expanded.contains(&(0, 0)));
    assert!(app.expanded.contains(&(0, 1)));
}

#[test]
fn c_collapses_all_in_latest_message() {
    let (mut app, _td) = fresh_app();
    app.tools.on_event(&StreamEvent::TurnStarted);
    app.tools.on_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c1".into(),
        tool_name: "t".into(),
        args: json!({}),
    });
    app.tools.on_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c1".into(),
        ok: true,
        error_message: None,
    });
    app.expanded.insert((0, 0));
    let a = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(a, Action::CollapseAllInLatestMessage);
    assert!(!app.expanded.contains(&(0, 0)));
}
```

- [ ] **Step 6: Run keymap tests**

Run: `cargo nextest run -p cogito-tui keymap`
Expected: 16 tests passed (was 12; -1 dropped, +5 added).

- [ ] **Step 7: Compile check**

Run: `cargo check -p cogito-tui 2>&1 | head -10`
Expected: errors in `runtime_build.rs` (`App { show_tools: true, .. }`)
and `event_loop.rs` (uses `app.status_data()`). Tasks 9 + 10 fix these.

- [ ] **Step 8: Hold commit; proceed to Task 9**

---

## Phase 7: Runtime + event-loop wiring

### Task 9: `runtime_build` — push startup banner, drop `show_tools`

**Files:**
- Modify: `crates/cogito-tui/src/runtime_build.rs`

- [ ] **Step 1: Replace the App construction block**

Find the `App { ... }` literal near the bottom of `build()`. Drop the
`show_tools: true,` line. Add `current_turn_thinking: false,`.

- [ ] **Step 2: Push the startup banner**

Inside `build()`, right after the `let mut chat = ChatModel::new();`
line and BEFORE pushing the MCP banner lines, push the startup banner:

```rust
let startup = crate::ui::banner::startup_lines(
    &strategy.model_params.model,
    args.strategy
        .as_deref()
        .or(cfg.runtime.default_strategy.as_deref())
        .unwrap_or("default"),
    &session_id.to_string(),
);
for line in startup {
    chat.push_notice(line);
}
// Then the existing MCP banner push:
for line in &mcp_banner_lines {
    chat.push_notice(line.clone());
}
```

(The strategy-name selection mirrors the existing
`strategy_name: args.strategy.clone().or(cfg.runtime.default_strategy.clone()).unwrap_or_else(|| "<synthesized>".into())`
literal lower down; reuse the same idiom.)

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p cogito-tui 2>&1 | head -10`
Expected: errors only in `event_loop.rs` now.

---

### Task 10: `event_loop` — spinner tick + new `RenderInputs` shape

**Files:**
- Modify: `crates/cogito-tui/src/event_loop.rs`

- [ ] **Step 1: Add a tick counter and update the two `render` calls**

At the top of `run()`, before the `loop {`, add:

```rust
let mut spinner_tick: u64 = 0;
```

Replace both `render(f, &RenderInputs { ... })` blocks (the initial
draw and the redraw-tick draw) with the new shape:

```rust
render(
    f,
    &RenderInputs {
        chat: &app.chat,
        tools: &app.tools,
        selected: app.selected,
        expanded: &app.expanded,
        input: &app.input,
        turn_thinking: app.current_turn_thinking,
        spinner_tick,
        popup_prefix: popup_prefix(&app.popup).as_deref(),
    },
);
```

In the redraw branch (`_ = redraw_tick.tick() => { ... }`), bump the
counter just before drawing:

```rust
_ = redraw_tick.tick() => {
    spinner_tick = spinner_tick.wrapping_add(1);
    terminal.draw(|f| { /* new RenderInputs as above */ })
        .context("draw on tick")?;
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p cogito-tui`
Expected: clean compile of the library + binary. Integration tests
(`tests/e2e.rs`, `tests/snapshot.rs`, `tests/edges.rs`) still
reference the old shapes; addressed in Tasks 11-13.

- [ ] **Step 3: Run all unit tests**

Run: `cargo nextest run -p cogito-tui --lib 2>&1 | tail -3`
Expected: all library tests pass (no failures).

- [ ] **Step 4: Commit the bundle (Tasks 1-10)**

```bash
git add crates/cogito-tui/src/
git commit -m "$(cat <<'EOF'
feat(cogito-tui): visual + interaction redesign

Single-column chat (no tools pane, no persistent status bar) with
'▸' / '∴' role markers and inline expandable tool blocks
(spec docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md).

- ChatLine refactor: replace ToolStartLine + ToolEndLine with
  ToolBlock { call_id }; the renderer looks up tool state in
  ToolTreeModel via call_id at draw time.
- ui::chat rewrite: role markers, 5-state inline tool blocks
  (⠋ ✓ ✗ ▸ ▾), thinking spinner.
- ui::mod.rs single-column layout: chat + dim divider + input.
- ui::spinner: braille frame helper (~10 Hz).
- ui::banner: startup SystemNotice lines (∴∴∴ + version + identity).
- Drop ui::tools, ui::status, App.show_tools, status_data(), Ctrl-T.
- Keymap: add 1-9 quick-expand, e expand-all, c collapse-all.
- runtime_build pushes startup banner; event_loop maintains
  spinner_tick and the new RenderInputs shape.

Library tests: render_model (8) + render_model::tree_tests (8)
+ spinner (3) + banner (4) + chat (10) + ui (3) + app (5)
+ keymap (16) + slash (6) + resume (5) + resume::extract (3)
= 71 unit tests passing.

Integration tests (e2e, snapshot, edges) rewrite lands in
follow-up commits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 8: Integration test rewrites

### Task 11: Rewrite `tests/e2e.rs`

**Files:**
- Modify: `crates/cogito-tui/tests/e2e.rs`

- [ ] **Step 1: Replace the file**

The test file's `e2e_app()` helper already uses the
`JsonlStore::new(tempdir)` + `SessionHandle::test_stub()` pattern; just
drop `show_tools: true,` and add `current_turn_thinking: false,` to
the App literal.

The `draw()` helper currently takes the old `RenderInputs` shape; it
needs to switch to the new shape (no `show_tools`, no `status`, add
`turn_thinking` + `spinner_tick`). The cell-symbol collector at the
bottom stays as-is.

For each existing test, update the assertion strings:

| Old assertion | New assertion |
|---------------|---------------|
| `out.contains("> hi")` | `out.contains("▸  hi")` |
| `out.contains("agent: hello")` | `out.contains("∴  hello")` |
| `out.contains("[tool] read_file")` | `out.contains("read_file")` and `out.contains('✓')` |
| `out.contains("turn 1")` (tool pane) | DROP — tools pane is gone; the inline tool block already asserted via `read_file` + `✓` |

The test `ctrl_t_hides_tools_pane_in_render` becomes obsolete; replace
with a thematically equivalent test:

```rust
#[test]
fn layout_has_no_tools_pane() {
    let (app, _td) = e2e_app();
    let out = draw(&app);
    assert!(!out.contains("tools "), "tools pane should be absent:\n{out}");
}
```

The test `slash_unknown_command_renders_error_notice` survives once
the assertion `out.contains("unknown command")` is preserved.

The test `tool_lifecycle_renders_into_both_panes` becomes
`tool_lifecycle_renders_inline`:

```rust
#[test]
fn tool_lifecycle_renders_inline() {
    let (mut app, _td) = e2e_app();
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
    assert!(out.contains("read_file"), "chat lacking tool name:\n{out}");
    assert!(out.contains('✓'), "chat lacking completed glyph:\n{out}");
}
```

Add one new test:

```rust
#[test]
fn typing_thinking_response_shows_spinner_then_clears() {
    let (mut app, _td) = e2e_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    // No content yet -> spinner present.
    let out1 = draw(&app);
    assert!(out1.contains("∴ ⠋"), "spinner missing pre-content:\n{out1}");
    app.apply_stream_event(&StreamEvent::TextDelta { chunk: "hi".into() });
    let out2 = draw(&app);
    assert!(!out2.contains("∴ ⠋"), "spinner should clear on content:\n{out2}");
    assert!(out2.contains("∴  hi"), "content missing:\n{out2}");
}
```

- [ ] **Step 2: Run e2e tests**

Run: `cargo nextest run -p cogito-tui --test e2e`
Expected: 6 tests passed (was 5; -1 dropped, +2 added).

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/tests/e2e.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): rewrite e2e for redesigned layout

Update assertions for '▸' / '∴' role markers and inline tool
blocks. Replace ctrl_t_hides_tools_pane test with a
layout_has_no_tools_pane structural check. Add a
typing_thinking_response_shows_spinner_then_clears test exercising
the new '∴ ⠋' thinking-spinner gating.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Rewrite `tests/snapshot.rs`

**Files:**
- Modify: `crates/cogito-tui/tests/snapshot.rs`

- [ ] **Step 1: Replace the file**

Same scaffolding update as Task 11 — adopt new `RenderInputs` shape,
drop `show_tools`, add `turn_thinking` + `spinner_tick` to the helper.
The `app()` helper returns `(App, TempDir)`.

The 5 existing tests get rewritten / renamed:

1. `empty_state_shows_panes_and_status` →
   `empty_state_renders_no_tools_pane_and_no_status_bar`:
   ```rust
   let (app, _td) = app();
   let out = draw(&app, None);
   assert!(!out.contains("tools "), "tools pane should be absent:\n{out}");
   assert!(!out.contains("strategy:"), "status bar should be absent:\n{out}");
   assert!(!out.contains("turn:"), "turn counter should be absent:\n{out}");
   ```

2. `single_text_turn_renders_user_and_agent_lines`:
   ```rust
   let (mut app, _td) = app();
   app.chat.push_user_prompt("who are you?".into());
   app.apply_stream_event(&StreamEvent::TurnStarted);
   app.apply_stream_event(&StreamEvent::TextDelta {
       chunk: "I am cogito.".into(),
   });
   app.apply_stream_event(&StreamEvent::TurnCompleted);
   let out = draw(&app, None);
   assert!(out.contains("▸  who are you?"));
   assert!(out.contains("∴  I am cogito."));
   ```

3. `popup_overlays_when_prefix_set`:
   ```rust
   let (app, _td) = app();
   let out = draw(&app, Some("/"));
   assert!(out.contains("commands"));
   assert!(out.contains("/skill"));
   ```

4. `tools_hidden_grows_chat_width` →
   `chat_uses_full_width` (drop the toggle reference; assert chat
   column is the only horizontal pane):
   ```rust
   let (mut app, _td) = app();
   app.chat.push_user_prompt("test".into());
   let out = draw(&app, None);
   assert!(out.contains("▸  test"));
   // No tool pane separator at 30% mark.
   ```

5. `mcp_banner_lines_render_at_top_of_chat` → unchanged in intent,
   adapt to the new layout:
   ```rust
   let (mut app, _td) = app();
   app.chat.push_notice("[mcp] ✓ filesystem ready (4 tools)".to_string());
   let out = draw(&app, None);
   assert!(out.contains("filesystem ready"));
   ```

Add one new test for the startup banner:

```rust
#[test]
fn startup_banner_renders_three_lines_with_sigil_and_identity() {
    let (mut app, _td) = app();
    // Mimic what runtime_build does at build time.
    for line in cogito_tui::ui::banner::startup_lines("opus-4.7", "coder", "01abcdefghij") {
        app.chat.push_notice(line);
    }
    let out = draw(&app, None);
    assert!(out.contains("∴∴∴"), "sigil missing:\n{out}");
    assert!(out.contains("cogito"));
    assert!(out.contains("opus-4.7"));
    assert!(out.contains("coder"));
    assert!(out.contains("01abcdef"));
    assert!(!out.contains("01abcdefghij"));
}
```

- [ ] **Step 2: Run snapshot tests**

Run: `cargo nextest run -p cogito-tui --test snapshot`
Expected: 6 tests passed (was 5; +1 added).

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-tui/tests/snapshot.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): rewrite snapshots for redesigned layout

5 cases adapted to the new single-column layout (no tools pane, no
status bar). New 6th case asserts the startup banner sigil
'∴∴∴' + project name + identity line render in the first frame.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: Rewrite `tests/edges.rs`

**Files:**
- Modify: `crates/cogito-tui/tests/edges.rs`

- [ ] **Step 1: Adopt new `RenderInputs` shape in `fresh_app` / `draw`**

Same as Tasks 11 + 12 — drop `show_tools`, add `turn_thinking` +
`spinner_tick`, use cell-symbol collector.

- [ ] **Step 2: Update existing test assertions**

| Test | Change |
|------|--------|
| `resize_mid_stream_does_not_lose_content` | `out.contains("agent: before resize")` → `out.contains("∴  before resize")` |
| `extremely_long_input_does_not_panic` | unchanged in shape |
| `unicode_in_tool_args_renders_without_corruption` | tools now inline; expand the tool, assert args row contains the CJK / emoji characters. The plan's Task 21 from sprint 9b already broke `out.contains("深圳")` into individual chars; keep that style. Wait for a TurnCompleted before drawing so the tool is finished and expansion shows args. |
| `deep_tool_tree_renders_without_panic` | inline tools mean 60 tool blocks rendered as 60 lines in chat. Assert chat contains `t0` (first tool name) and `read_file`-style names. Drop "turn 1" assertion (no header any more). |

- [ ] **Step 3: Add new test**

```rust
#[test]
fn quick_expand_via_digit_one_works_after_tool_completes() {
    let (mut app, _td) = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c".into(),
        tool_name: "read_file".into(),
        args: serde_json::json!({"path": "x.rs"}),
    });
    app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c".into(),
        ok: true,
        error_message: None,
    });
    use cogito_tui::keymap::dispatch;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    let out = draw(&app, 80, 24);
    assert!(out.contains('▾'), "expansion glyph missing:\n{out}");
    assert!(out.contains("args"), "args row missing:\n{out}");
    assert!(out.contains("path"), "args content missing:\n{out}");
}
```

- [ ] **Step 4: Run edge tests**

Run: `cargo nextest run -p cogito-tui --test edges`
Expected: 5 tests passed (was 4; +1 added).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-tui/tests/edges.rs
git commit -m "$(cat <<'EOF'
test(cogito-tui): rewrite edges for redesigned layout

resize / long-input / unicode / deep-tree tests adapted to the
new single-column inline-tool layout. New quick_expand_via_digit_one
test exercises the keymap → render path end-to-end through the new
'1' quick-expand action.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 9: Final validation

### Task 14: `make ci` green + manual smoke + PR

**Files:** (whatever lint surfaces; expect minimal)

- [ ] **Step 1: Format**

Run: `make fmt`
Expected: exit 0; files may be reflowed.

- [ ] **Step 2: Clippy on the redesigned crate**

Run: `make fix CRATE=cogito-tui`
Expected: exit 0; auto-fixable lints applied. Review the diff
manually before committing — auto-fix is over-eager in places.

- [ ] **Step 3: Full CI**

Run: `make ci`
Expected: fmt-check + clippy + layer-check + test all green.

Likely surprises (mostly handled in prior tasks but worth knowing):

- `clippy::doc_markdown` on TUI / JSON / JSONL / OpenAI in doc
  comments — backtick them.
- `clippy::missing_errors_doc` on `pub fn` returning `Result` — add a
  `# Errors` section.
- `clippy::cast_possible_truncation` on `as u16` / `as usize` casts —
  prefer `u16::try_from(...).unwrap_or(u16::MAX)`.
- `clippy::match_same_arms` — collapse identical arms.

- [ ] **Step 4: Verify the test count**

Run: `cargo nextest run -p cogito-tui 2>&1 | tail -3`
Expected: `Summary [...] N tests run: N passed, 0 skipped` with
N ≥ 90 (spec §11 acceptance criterion 9). Realistically ~90-95.

- [ ] **Step 5: Smoke test the binary (manual)**

```bash
make chat  # CLI: confirm cogito-cli still works
cargo run -p cogito-tui -- --list-strategies
# Should print the available strategies (no regression).
```

If an Anthropic / OpenAI key is set, run a short interactive session
to verify visually:

```bash
cargo run -p cogito-tui
# Verify:
# - ∴∴∴ banner appears as first 3 lines
# - MCP banner appears immediately below
# - type "say hello in three words" + Enter
# - ▸ prefix on your message
# - ∴ ⠋ spinner briefly between send and first token
# - ∴ prefix on streaming response
# - if a tool is called, ⠋ during it, ✓ after, name + ms visible
# - Ctrl-↑ selects the tool block; Enter expands; Esc clears
# - press '1' to quick-expand the most recent tool
# - press 'e' / 'c' to expand-all / collapse-all
# - press Ctrl-D on empty input → exits cleanly
```

Document any visual oddities. Fix in place if trivial, or open a
follow-up issue.

- [ ] **Step 6: Final commit (only if lint fix-ups landed)**

```bash
git add -u
git commit -m "$(cat <<'EOF'
chore(cogito-tui): final make ci green for redesign

Resolves clippy lints surfaced by full workspace check
(doc_markdown / missing_errors_doc / cast_possible_truncation
where applicable).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If nothing changed, skip this commit.

- [ ] **Step 7: Push and open PR**

```bash
git push -u github feat/tui-redesign
gh pr create --title "cogito-tui: visual + interaction redesign (v0.2)" --body "$(cat <<'EOF'
## Summary

Reimagines the cogito-tui surface per
`docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`:

- Single-column chat — drop the tools pane, drop the persistent
  status bar.
- `▸` / `∴` role markers replace `> ` / `agent: ` text labels.
  `∴` is the *therefore* operator from *cogito ergo sum* — every
  agent reply is a reasoning conclusion.
- Inline tool blocks with `⠋ ✓ ✗ ▸ ▾` lifecycle glyphs (running →
  ok / err, selected, expanded).
- `∴ ⠋` thinking spinner between turn dispatch and first content,
  re-appears between tool end and next decision.
- Ambient startup banner (`∴∴∴` + version + identity line) +
  unchanged MCP banner; both scroll away naturally.
- Keymap: `Ctrl-T` removed; `1-9` quick-expand most recent tools;
  `e` / `c` expand-all / collapse-all in latest cogito message.

## Out of scope (deferred, captured in spec)

Markdown rendering, incremental ChatModel projection, message
animations — separate sprints.

## Test plan

- [ ] `make ci` green
- [ ] `cargo nextest run -p cogito-tui` — ≥ 90 tests pass
- [ ] `cogito-tui --list-strategies` parity preserved
- [ ] Manual smoke: ∴∴∴ banner, role markers, tool spinner, thinking
  spinner, `Ctrl-↑/↓` selection, `1` quick-expand, `e`/`c` bulk,
  `Ctrl-D` on empty input exits cleanly

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review (controller checklist)

This is the controller's pass — not a subagent dispatch. Run before
declaring the plan complete.

**1. Spec coverage:**

| Spec section | Plan task(s) |
|---|---|
| §"Visual language" (role markers, palette, flat borderless) | Task 4 |
| §"Chrome strategy" (startup banner) | Task 3 + Task 9 |
| §"Chrome strategy" (input footer, dim divider) | Task 5 |
| §"Tool block lifecycle (T1)" — 5 glyphs | Task 4 |
| §"Thinking spinner" | Task 4 + Task 7 (`current_turn_thinking`) |
| §"Spinner animation source" | Task 2 |
| §"Interaction model" — implicit focus + keymap | Task 8 |
| §"Architecture impact" — removed (tools, status, show_tools, Ctrl-T) | Tasks 6 + 7 + 8 |
| §"ChatLine refactor" | Task 1 |
| §"Testing strategy" | Tasks 1, 2, 3, 4, 5, 7, 8, 11, 12, 13 (per-section unit + integration) |
| §"Acceptance criteria" (10 items) | All addressed across the 14 tasks; criterion 10 (manual smoke) is Task 14 |

No gaps.

**2. Placeholder scan:** Grep the plan for `TBD`, `TODO`, `implement
later`, `fill in details`, `Similar to Task`. The only `TODO` strings
are inside `// TODO Task 6: …` source-code comments that mark
intentionally short-lived placeholders, removed by Task 6. No real
plan-level placeholders.

**3. Type consistency:** Names used across tasks:

- `ChatLine::ToolBlock { call_id: String }` — Tasks 1, 4, 11, 12, 13.
- `ChatRenderInputs` — Tasks 4 + 5.
- `RenderInputs` — Tasks 5, 10, 11, 12, 13.
- `Action::ExpandRecent / ExpandAllInLatestMessage /
  CollapseAllInLatestMessage` — Task 8.
- `ChatModel`, `ToolTreeModel`, `App`, `TerminalGuard` — preserved
  spelling.
- `current_turn_thinking` — Tasks 7, 4, 10.
- `spinner::frame(tick)` / `spinner::PERIOD_TICKS` — Tasks 2, 4, 10.
- `banner::startup_lines(model, strategy, session)` — Tasks 3, 9, 12.

No name drift.

The plan totals 14 tasks across 9 phases. Subagents can execute them
in order; review gates (per subagent-driven-development skill) apply
between tasks.

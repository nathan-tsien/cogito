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
use crate::ui::markdown::{self, MdStyles};
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
            ChatLine::AssistantText(text) => out.extend(assistant_lines(text, &p)),
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

/// Markdown styles derived from the chat palette.
fn md_styles(p: &Palette) -> MdStyles {
    MdStyles {
        bold: Style::default().add_modifier(Modifier::BOLD),
        italic: Style::default().add_modifier(Modifier::ITALIC),
        code_inline: Style::default().fg(Color::Yellow),
        code_block: p.dim,
        list_marker: p.cogito,
    }
}

/// Render an assistant message as markdown body lines, prefixed with the
/// `∴ ` marker on the first visual line and a 3-space gutter on the rest.
fn assistant_lines(text: &str, p: &Palette) -> Vec<Line<'static>> {
    let mut body = markdown::render(text, &md_styles(p));
    if body.is_empty() {
        // Message started but no content yet: bare marker line.
        return vec![Line::from(vec![Span::styled("∴  ", p.cogito)])];
    }
    for (i, line) in body.iter_mut().enumerate() {
        let prefix = if i == 0 {
            Span::styled("∴  ", p.cogito)
        } else {
            Span::raw(INDENT.to_string())
        };
        line.spans.insert(0, prefix);
    }
    body
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
fn lookup<'a>(tools: &'a ToolTreeModel, call_id: &str) -> Option<(TreePath, &'a ToolNode)> {
    for (t_idx, group) in tools.turns.iter().enumerate() {
        for (n_idx, node) in group.nodes.iter().enumerate() {
            if node.call_id == call_id {
                return Some(((t_idx, n_idx), node));
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
    out.push(tool_header_line(node, is_selected, is_expanded, inputs, p));
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
        // Integer math avoids the u128 -> f64 precision-loss lint; we only
        // render one decimal place of seconds.
        format!("{}.{}s", ms / 1000, (ms % 1000) / 100)
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

    #[allow(clippy::too_many_arguments)]
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
        {
            let ev = StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            };
            chat.on_event(&ev);
            tools.on_event(&ev);
        }
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 60, 8);
        assert!(out.contains('✓'), "got:\n{out}");
    }

    #[test]
    fn failed_tool_renders_cross_and_error_message() {
        let (mut chat, mut tools) = build_tools_with_one("c1", "run_tests");
        {
            let ev = StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some("panicked: assertion failed".into()),
            };
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
        let out = draw(
            &chat,
            &tools,
            Some((0, 0)),
            &HashSet::new(),
            false,
            0,
            60,
            8,
        );
        assert!(out.contains("▸ read_file"), "got:\n{out}");
    }

    #[test]
    fn assistant_markdown_bold_renders_marker_and_styles() {
        let mut chat = ChatModel::new();
        chat.on_event(&StreamEvent::TextDelta {
            chunk: "see **bold** here".into(),
        });
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        // marker present, asterisks gone (parsed as emphasis)
        assert!(out.contains("∴  see bold here"), "got:\n{out}");
        assert!(!out.contains("**"), "got:\n{out}");
    }

    #[test]
    fn assistant_multiline_markdown_indents_continuation() {
        let mut chat = ChatModel::new();
        chat.on_event(&StreamEvent::TextDelta {
            chunk: "first line\nsecond line".into(),
        });
        let tools = empty_tools();
        let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 5);
        // first line carries the marker, second carries the 3-space gutter
        assert!(out.contains("∴  first line"), "got:\n{out}");
        assert!(out.contains("   second line"), "got:\n{out}");
    }

    #[test]
    fn expanded_completed_tool_renders_args_and_result_placeholder() {
        let (mut chat, mut tools) = build_tools_with_one("c1", "read_file");
        {
            let ev = StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            };
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

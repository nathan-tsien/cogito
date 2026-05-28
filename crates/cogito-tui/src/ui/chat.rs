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

    use crate::render_model::ChatModel;
    use cogito_protocol::stream::StreamEvent;

    /// Render `model` into a `w x h` `TestBackend` terminal and return the
    /// buffer contents as a flat string (one row per line, separated by
    /// newlines). ratatui 0.28 `Buffer` implements `Debug` but not
    /// `Display`, so we read cells directly via `Buffer::content()`.
    fn draw(model: &ChatModel, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, model);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let width = buf.area().width as usize;
        let cells = buf.content();
        cells
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

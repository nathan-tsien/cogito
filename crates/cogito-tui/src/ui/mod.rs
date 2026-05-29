//! UI surface — top-level `render` orchestrates the single chat
//! column + input footer (spec §"Chrome strategy"). No persistent
//! status row; no tools pane.

pub mod banner;
pub mod chat;
pub mod input;
pub mod popup;
pub mod spinner;

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
    /// Lifecycle: `true` between `TurnStarted` | `ToolDispatchEnded` and
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
            Constraint::Min(3),    // chat
            Constraint::Length(1), // dim divider
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
        assert!(
            !out.contains("tools "),
            "tools pane should be absent:\n{out}"
        );
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

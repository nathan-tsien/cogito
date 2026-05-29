//! UI widgets — top-level `render` lands in Phase 8; submodules below
//! populate progressively across Phases 4–7.

pub mod chat;
pub mod input;
pub mod popup;
pub mod status;
pub mod tools;

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
            Constraint::Min(3), // chat + tools row
            Constraint::Length(input_h),
            Constraint::Length(1), // status bar
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
        crate::ui::tools::render(f, split[1], inputs.tools, inputs.selected, inputs.expanded);
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

    /// Render all panes into a `w x h` `TestBackend` terminal and return the
    /// buffer contents as a flat string (one row per line, separated by
    /// newlines). `Buffer` does not implement `Display` in ratatui 0.29, so
    /// we collect cell symbols directly.
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
        let out = draw_buf(false, true, 120, 20);
        assert!(out.contains("chat"), "got:\n{out}");
        assert!(
            !out.contains("tools "),
            "tools pane should be hidden, got:\n{out}"
        );
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
        let buf = terminal.backend().buffer().clone();
        let width = buf.area().width as usize;
        let cells = buf.content();
        let out = cells
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(out.contains("commands"), "popup title missing:\n{out}");
        assert!(out.contains("/skill"), "popup entry missing:\n{out}");
    }
}

//! Bottom status bar — single line. Renders strategy / model /
//! session / turn count on the left and key hints on the right.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Status data — pure value type, no widgets.
#[derive(Debug, Clone)]
pub struct StatusData {
    /// Strategy name in effect (e.g. `coder`).
    pub strategy: String,
    /// Model id in effect.
    pub model: String,
    /// Session id (truncated to 8 chars in display).
    pub session_id: String,
    /// Number of completed turns so far.
    pub turn_count: u32,
    /// Whether the tools pane is currently visible (for hint text).
    pub tools_visible: bool,
}

const HINT_TEXT: &str = "Ctrl-C cancel/exit · Ctrl-D exit · Ctrl-T tools · / commands";

/// Render the status bar into `area` (typically one row tall).
pub fn render(f: &mut Frame, area: Rect, data: &StatusData) {
    let truncated_session = data.session_id.chars().take(8).collect::<String>();
    let left = format!(
        "strategy: {s} · model: {m} · session: {sid} · turn: {t}",
        s = data.strategy,
        m = data.model,
        sid = truncated_session,
        t = data.turn_count
    );
    let tools_label = if data.tools_visible { "on" } else { "off" };
    let right = format!("tools: {tools_label} · {HINT_TEXT}");
    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(right, Style::default().add_modifier(Modifier::DIM)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render `data` into a `w x 1` terminal and return the buffer as a flat
    /// string. `Buffer` does not implement `Display` in ratatui 0.29, so we
    /// collect cell symbols directly.
    fn draw(data: &StatusData, w: u16) -> String {
        let backend = TestBackend::new(w, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, data);
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
    fn renders_strategy_model_session_turn() {
        let data = StatusData {
            strategy: "coder".into(),
            model: "claude-opus-4-7".into(),
            session_id: "01jaaaaaa0000".into(),
            turn_count: 3,
            tools_visible: true,
        };
        let out = draw(&data, 200);
        assert!(out.contains("strategy: coder"), "got:\n{out}");
        assert!(out.contains("model: claude-opus-4-7"), "got:\n{out}");
        assert!(out.contains("session: 01jaaaaa"), "got:\n{out}");
        assert!(out.contains("turn: 3"), "got:\n{out}");
    }

    #[test]
    fn renders_tools_on_when_visible() {
        let data = StatusData {
            strategy: "x".into(),
            model: "y".into(),
            session_id: "z".into(),
            turn_count: 0,
            tools_visible: true,
        };
        let out = draw(&data, 200);
        assert!(out.contains("tools: on"), "got:\n{out}");
    }

    #[test]
    fn renders_tools_off_when_hidden() {
        let data = StatusData {
            strategy: "x".into(),
            model: "y".into(),
            session_id: "z".into(),
            turn_count: 0,
            tools_visible: false,
        };
        let out = draw(&data, 200);
        assert!(out.contains("tools: off"), "got:\n{out}");
    }
}

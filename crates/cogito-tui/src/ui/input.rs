//! Multi-line input widget — thin wrapper around `tui_textarea::TextArea`.
//!
//! Discriminates `Enter` (send) from `Shift+Enter` (newline) ourselves
//! instead of letting `TextArea::input` consume the key, because the
//! upstream default semantics treat both the same.
//!
//! Visible height is capped at `MAX_VISIBLE_INPUT_LINES`; longer
//! buffers scroll within (`TextArea` handles its own internal scroll).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tui_textarea::TextArea;

/// Maximum visible input lines (the input bar grows up to this, then
/// scrolls internally).
pub const MAX_VISIBLE_INPUT_LINES: u16 = 8;

/// Result of a key dispatch from the App to the input widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    /// Key was consumed by the textarea (or ignored); nothing for
    /// the app to do beyond redrawing.
    Consumed,
    /// User pressed Enter on a non-empty buffer — App should treat
    /// the returned text as a message to send.
    Submit(String),
}

/// Multi-line input state — a wrapper around `tui_textarea::TextArea`.
pub struct InputWidget {
    textarea: TextArea<'static>,
}

impl Default for InputWidget {
    fn default() -> Self {
        let mut ta = TextArea::default();
        ta.set_cursor_line_style(Style::default());
        ta.set_placeholder_text("Type a message \u{2014} Enter to send, Shift+Enter for newline");
        ta.set_placeholder_style(Style::default().fg(Color::DarkGray));
        Self { textarea: ta }
    }
}

impl InputWidget {
    /// Fresh empty input.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle a key event. Returns `Submit(text)` if the user pressed
    /// Enter on a non-empty buffer; `Consumed` otherwise.
    pub fn on_key(&mut self, key: KeyEvent) -> InputOutcome {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, mods) if !mods.contains(KeyModifiers::SHIFT) => {
                let text = self.textarea.lines().join("\n").trim().to_string();
                if text.is_empty() {
                    return InputOutcome::Consumed;
                }
                self.clear();
                InputOutcome::Submit(text)
            }
            (KeyCode::Enter, _) => {
                // Shift+Enter: explicit newline. tui-textarea's
                // `input` would do this for plain Enter too; we force
                // it through the explicit newline insert API.
                self.textarea.insert_newline();
                InputOutcome::Consumed
            }
            _ => {
                let event = tui_textarea::Input::from(key);
                self.textarea.input(event);
                InputOutcome::Consumed
            }
        }
    }

    /// First character of the buffer, or None if empty. Used by the
    /// App to decide whether to show the `/`-popup.
    #[must_use]
    pub fn first_char(&self) -> Option<char> {
        self.textarea.lines().first().and_then(|l| l.chars().next())
    }

    /// Total visible height needed for the input, clamped to
    /// `MAX_VISIBLE_INPUT_LINES`. No border rows — the layout draws a
    /// divider above the input and a `▸` prompt marker sits in a left
    /// gutter, so the input itself is borderless.
    #[must_use]
    pub fn desired_height(&self) -> u16 {
        let content = u16::try_from(self.textarea.lines().len()).unwrap_or(u16::MAX);
        content.clamp(1, MAX_VISIBLE_INPUT_LINES)
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
    }

    /// Render into `area`: a 3-column `▸  ` prompt marker gutter (cyan,
    /// matching the user role marker in scrollback) plus the textarea.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);
        let marker = Paragraph::new(Line::from(Span::styled(
            "▸  ",
            Style::default().fg(Color::Cyan),
        )));
        f.render_widget(marker, cols[0]);
        f.render_widget(&self.textarea, cols[1]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn enter_with_empty_buffer_is_consumed_not_submitted() {
        let mut w = InputWidget::new();
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(r, InputOutcome::Consumed);
    }

    #[test]
    fn enter_with_text_returns_submit() {
        let mut w = InputWidget::new();
        for ch in "hi".chars() {
            w.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(r, InputOutcome::Submit("hi".into()));
    }

    #[test]
    fn shift_enter_inserts_newline_does_not_submit() {
        let mut w = InputWidget::new();
        w.on_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
        let r = w.on_key(key(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(r, InputOutcome::Consumed);
        w.on_key(key(KeyCode::Char('b'), KeyModifiers::NONE));
        let submit = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(submit, InputOutcome::Submit("a\nb".into()));
    }

    #[test]
    fn submit_clears_the_buffer() {
        let mut w = InputWidget::new();
        for ch in "hi".chars() {
            w.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let _ = w.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(w.first_char(), None);
        assert_eq!(w.desired_height(), 1); // 1 line, no border
    }

    #[test]
    fn desired_height_grows_with_lines_then_caps() {
        let mut w = InputWidget::new();
        for _ in 0..10 {
            w.on_key(key(KeyCode::Enter, KeyModifiers::SHIFT));
        }
        assert_eq!(w.desired_height(), MAX_VISIBLE_INPUT_LINES);
    }

    #[test]
    fn first_char_detects_leading_slash() {
        let mut w = InputWidget::new();
        w.on_key(key(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(w.first_char(), Some('/'));
    }
}

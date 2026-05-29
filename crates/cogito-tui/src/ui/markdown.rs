//! Markdown -> ratatui line translator. Pure, IO-free, palette-injected.
//!
//! Scope (spec `2026-05-29-cogito-tui-markdown-design.md`): bold,
//! italic, inline code, fenced/indented code blocks, and bullet /
//! numbered lists. Headings, block quotes, and links degrade to plain
//! text. This module knows nothing about the chat `∴` marker — the
//! chat widget prepends the marker / gutter (see `ui::chat`).

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Styles the markdown builder applies, injected from the chat palette.
#[derive(Clone, Copy)]
pub struct MdStyles {
    /// Style for `**bold**` runs.
    pub bold: Style,
    /// Style for `*italic*` runs.
    pub italic: Style,
    /// Style for `` `inline code` `` runs.
    pub code_inline: Style,
    /// Style for fenced / indented code block contents.
    pub code_block: Style,
    /// Style for list item markers (`- `, `1. `).
    pub list_marker: Style,
}

/// Parse `src` as `CommonMark` and emit body lines (see module docs).
#[must_use]
pub fn render(src: &str, styles: &MdStyles) -> Vec<Line<'static>> {
    let mut b = Builder::new(styles);
    for event in Parser::new(src) {
        b.handle(event);
    }
    b.finish()
}

/// Accumulates spans into lines while walking the pulldown event stream.
struct Builder<'s> {
    styles: &'s MdStyles,
    lines: Vec<Line<'static>>,
    /// Spans for the line currently under construction.
    pending_spans: Vec<Span<'static>>,
    /// Nesting depth of `**bold**` we are inside.
    bold: u32,
    /// Nesting depth of `*italic*` we are inside.
    italic: u32,
    /// Whether a previous block has been fully emitted (drives inter-block spacing).
    prev_block_emitted: bool,
}

impl<'s> Builder<'s> {
    fn new(styles: &'s MdStyles) -> Self {
        Self {
            styles,
            lines: Vec::new(),
            pending_spans: Vec::new(),
            bold: 0,
            italic: 0,
            prev_block_emitted: false,
        }
    }

    /// Inline style implied by the active bold/italic depth.
    fn inline_style(&self) -> Style {
        let mut s = Style::default();
        if self.bold > 0 {
            s = s.patch(self.styles.bold);
        }
        if self.italic > 0 {
            s = s.patch(self.styles.italic);
        }
        s
    }

    /// Append text to the current line in the active inline style.
    fn push_text(&mut self, text: &str) {
        self.pending_spans
            .push(Span::styled(text.to_string(), self.inline_style()));
    }

    /// Flush the current spans as a finished line (even if empty).
    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.pending_spans);
        self.lines.push(Line::from(spans));
    }

    /// Emit a blank separator line before a block when a previous block
    /// has already been emitted. Block-start arms call this so paragraphs,
    /// code blocks, and lists are spaced apart consistently.
    fn emit_block_separator(&mut self) {
        if self.prev_block_emitted {
            self.lines.push(Line::from(Vec::new()));
        }
    }

    /// Dispatch one pulldown-cmark event: update inline/block state or
    /// accumulate spans onto the current line.
    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Paragraph) => self.emit_block_separator(),
            Event::End(TagEnd::Paragraph) => {
                self.flush_line();
                self.prev_block_emitted = true;
            }
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Code(text) => {
                // Inline code uses its own style standalone; surrounding
                // bold/italic does not compose onto it (code color wins).
                self.pending_spans
                    .push(Span::styled(text.to_string(), self.styles.code_inline));
            }
            Event::Text(text) => self.push_text(&text),
            _ => {}
        }
    }

    /// Flush any trailing partial line and return all lines.
    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.pending_spans.is_empty() {
            self.flush_line();
        }
        self.lines
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn styles() -> MdStyles {
        MdStyles {
            bold: Style::default().add_modifier(Modifier::BOLD),
            italic: Style::default().add_modifier(Modifier::ITALIC),
            code_inline: Style::default().fg(Color::Yellow),
            code_block: Style::default().add_modifier(Modifier::DIM),
            list_marker: Style::default().fg(Color::Green),
        }
    }

    /// Flatten a Line's spans into (content, style) pairs.
    fn spans(line: &Line<'static>) -> Vec<(String, Style)> {
        line.spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    /// Concatenated text content of a line.
    fn text_of(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(render("", &styles()).is_empty());
    }

    #[test]
    fn plain_paragraph_is_one_line() {
        let out = render("hello world", &styles());
        assert_eq!(out.len(), 1);
        assert_eq!(text_of(&out[0]), "hello world");
    }

    #[test]
    fn bold_run_carries_bold_modifier() {
        let out = render("a **b** c", &styles());
        let pairs = spans(&out[0]);
        let bold_span = pairs.iter().find(|(t, _)| t == "b").unwrap();
        assert!(bold_span.1.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn italic_run_carries_italic_modifier() {
        let out = render("a *b* c", &styles());
        let pairs = spans(&out[0]);
        let it = pairs.iter().find(|(t, _)| t == "b").unwrap();
        assert!(it.1.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn nested_bold_italic_composes_both() {
        let out = render("**b _i_**", &styles());
        let pairs = spans(&out[0]);
        let inner = pairs.iter().find(|(t, _)| t == "i").unwrap();
        assert!(inner.1.add_modifier.contains(Modifier::BOLD));
        assert!(inner.1.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code_uses_code_inline_style() {
        let out = render("call `foo()` now", &styles());
        let pairs = spans(&out[0]);
        let code = pairs.iter().find(|(t, _)| t == "foo()").unwrap();
        assert_eq!(code.1.fg, Some(Color::Yellow));
    }

    #[test]
    fn two_paragraphs_separated_by_blank_line() {
        let out = render("one\n\ntwo", &styles());
        // para "one", blank separator, para "two"
        assert_eq!(out.len(), 3);
        assert_eq!(text_of(&out[0]), "one");
        assert_eq!(text_of(&out[1]), "");
        assert_eq!(text_of(&out[2]), "two");
    }

    #[test]
    fn text_around_bold_run_is_not_bold() {
        let out = render("a **b** c", &styles());
        let pairs = spans(&out[0]);
        for (t, st) in &pairs {
            if t != "b" {
                assert!(
                    !st.add_modifier.contains(Modifier::BOLD),
                    "span {t:?} should not be bold"
                );
            }
        }
    }

    #[test]
    fn inline_code_inside_bold_keeps_only_code_style() {
        let out = render("**a `c` b**", &styles());
        let pairs = spans(&out[0]);
        let code = pairs.iter().find(|(t, _)| t == "c").unwrap();
        assert_eq!(code.1.fg, Some(Color::Yellow));
        assert!(!code.1.add_modifier.contains(Modifier::BOLD));
    }
}

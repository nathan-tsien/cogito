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

/// Spaces added per indent level (code blocks, nested list content).
const NEST_INDENT: usize = 2;

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
    /// `true` while inside a fenced/indented code block.
    in_code_block: bool,
    /// Active list levels. `Some(n)` = ordered (next number), `None` = bullet.
    lists: Vec<Option<u64>>,
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
            in_code_block: false,
            lists: Vec::new(),
        }
    }

    /// Leading indent (spaces) implied by current list nesting.
    fn list_indent(&self) -> usize {
        self.lists.len().saturating_sub(1) * NEST_INDENT
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
            Event::Start(Tag::Paragraph) => {
                if self.lists.is_empty() {
                    self.emit_block_separator();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                // Inside a list item, End(Item) flushes the line instead.
                if self.lists.is_empty() {
                    self.flush_line();
                    self.prev_block_emitted = true;
                }
            }
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => {
                self.emit_block_separator();
                self.in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                self.in_code_block = false;
                self.prev_block_emitted = true;
            }
            Event::Code(text) => {
                // Inline code uses its own style standalone; surrounding
                // bold/italic does not compose onto it (code color wins).
                self.pending_spans
                    .push(Span::styled(text.to_string(), self.styles.code_inline));
            }
            Event::Text(text) => {
                if self.in_code_block {
                    // pulldown emits code-block content as one Text with a trailing
                    // newline. Strip only the trailing newline(s) and split on '\n'
                    // so blank lines in the MIDDLE of the block are preserved.
                    let indent = " ".repeat(NEST_INDENT);
                    for raw in text.trim_end_matches('\n').split('\n') {
                        self.lines.push(Line::from(vec![
                            Span::raw(indent.clone()),
                            Span::styled(raw.to_string(), self.styles.code_block),
                        ]));
                    }
                } else {
                    self.push_text(&text);
                }
            }
            Event::Start(Tag::List(first)) => {
                if self.lists.is_empty() {
                    self.emit_block_separator();
                } else {
                    // Nested list: flush the parent item's partial line so the
                    // parent text and nested items don't merge onto one line.
                    if !self.pending_spans.is_empty() {
                        self.flush_line();
                    }
                }
                self.lists.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
                self.prev_block_emitted = true;
            }
            Event::Start(Tag::Item) => {
                // Start a fresh line; build the marker from the innermost list.
                let indent = " ".repeat(self.list_indent());
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "- ".to_string(),
                };
                if !indent.is_empty() {
                    self.pending_spans.push(Span::raw(indent));
                }
                self.pending_spans
                    .push(Span::styled(marker, self.styles.list_marker));
            }
            Event::End(TagEnd::Item) => {
                // Flush the completed item line. End(Paragraph) is suppressed
                // inside lists, so this is the authoritative flush point. Guard
                // against an empty flush: when an item wraps only a nested list,
                // its line was already flushed at the nested List start, leaving
                // pending_spans empty.
                if !self.pending_spans.is_empty() {
                    self.flush_line();
                }
            }
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

    #[test]
    fn code_block_lines_use_code_block_style_and_indent() {
        let src = "```\nlet x = 1;\nlet y = 2;\n```";
        let out = render(src, &styles());
        // two code lines
        let code_lines: Vec<_> = out.iter().filter(|l| text_of(l).contains("let ")).collect();
        assert_eq!(code_lines.len(), 2);
        // every non-blank code span carries the code_block style (DIM)
        for l in &code_lines {
            let styled = l.spans.iter().find(|s| s.content.contains("let")).unwrap();
            assert!(styled.style.add_modifier.contains(Modifier::DIM));
        }
        // indented (leading-space span present)
        assert!(text_of(code_lines[0]).starts_with(' '));
    }

    #[test]
    fn code_block_preserves_blank_middle_line() {
        let src = "```\na\n\nb\n```";
        let out = render(src, &styles());
        // a, blank code line, b — three code lines (the middle one empty).
        let code: Vec<_> = out
            .iter()
            .filter(|l| {
                // code lines start with the 2-space indent and use code_block style
                l.spans.first().map(|s| s.content.as_ref()) == Some("  ")
            })
            .collect();
        assert_eq!(code.len(), 3, "expected 3 code lines, got {}", code.len());
        assert_eq!(text_of(code[0]).trim_end(), "  a");
        assert_eq!(text_of(code[1]), "  ");
        assert_eq!(text_of(code[2]).trim_end(), "  b");
    }

    #[test]
    fn asterisks_inside_code_block_stay_literal() {
        let src = "```\n**not bold**\n```";
        let out = render(src, &styles());
        let line = out
            .iter()
            .find(|l| text_of(l).contains("not bold"))
            .unwrap();
        assert!(text_of(line).contains("**not bold**"));
        // no span carries BOLD
        assert!(
            line.spans
                .iter()
                .all(|s| !s.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn unordered_list_items_get_dash_markers() {
        let out = render("- alpha\n- beta", &styles());
        let items: Vec<_> = out
            .iter()
            .filter(|l| {
                let t = text_of(l);
                t.contains("alpha") || t.contains("beta")
            })
            .collect();
        assert_eq!(items.len(), 2);
        assert!(text_of(items[0]).contains("- "));
        assert!(text_of(items[0]).contains("alpha"));
        // marker span carries list_marker style (green fg)
        let marker = items[0]
            .spans
            .iter()
            .find(|s| s.content.contains('-'))
            .unwrap();
        assert_eq!(marker.style.fg, Some(ratatui::style::Color::Green));
    }

    #[test]
    fn ordered_list_items_get_numbered_markers() {
        let out = render("1. first\n2. second", &styles());
        let first = out.iter().find(|l| text_of(l).contains("first")).unwrap();
        let second = out.iter().find(|l| text_of(l).contains("second")).unwrap();
        assert!(text_of(first).contains("1. "));
        assert!(text_of(second).contains("2. "));
    }

    #[test]
    fn nested_list_item_is_indented() {
        let out = render("- top\n  - nested", &styles());
        let nested = out.iter().find(|l| text_of(l).contains("nested")).unwrap();
        let top = out.iter().find(|l| text_of(l).contains("top")).unwrap();
        // nested item starts with more leading whitespace than the top item
        let lead = |l: &Line<'static>| text_of(l).len() - text_of(l).trim_start().len();
        assert!(lead(nested) > lead(top));
    }

    #[test]
    fn nested_list_has_no_stray_blank_between_siblings() {
        let out = render("- a\n  - b\n- c", &styles());
        let texts: Vec<String> = out.iter().map(text_of).collect();
        let pos_b = texts.iter().position(|t| t.contains("- b")).unwrap();
        let pos_c = texts.iter().position(|t| t.contains("- c")).unwrap();
        // sibling "- c" immediately follows nested "- b": no blank between them.
        assert_eq!(pos_c, pos_b + 1, "stray blank line in: {texts:?}");
    }
}

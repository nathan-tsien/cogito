//! Markdown -> ratatui line translator. Pure, IO-free, palette-injected.
//!
//! Scope (spec `2026-05-29-cogito-tui-markdown-design.md`): bold,
//! italic, inline code, fenced/indented code blocks, and bullet /
//! numbered lists. Headings, block quotes, and links degrade to plain
//! text. This module knows nothing about the chat `∴` marker — the
//! chat widget prepends the marker / gutter (see `ui::chat`).

use ratatui::style::Style;
use ratatui::text::Line;

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

/// Parse `src` as CommonMark and emit body lines with no outer
/// gutter / marker. Nested-list and code-block indentation are encoded
/// as leading-space spans on each line. Empty input yields an empty
/// `Vec`.
#[must_use]
pub fn render(src: &str, styles: &MdStyles) -> Vec<Line<'static>> {
    let _ = (src, styles);
    Vec::new()
}

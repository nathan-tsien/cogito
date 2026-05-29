# cogito-tui Markdown Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render assistant chat replies in cogito-tui as markdown (bold/italic, inline code, fenced code blocks, bullet/numbered lists) instead of flat raw text.

**Architecture:** A new pure `crate::ui::markdown` module parses an assistant message with `pulldown-cmark` and emits `Vec<Line<'static>>` body lines (no marker/gutter). `crate::ui::chat` prepends the `∴ ` marker to line 0 and a 3-space gutter to continuation lines, building `MdStyles` from its existing `Palette`. Parsing happens at draw time; `render_model` is untouched.

**Tech Stack:** Rust 2024 (MSRV 1.85), `pulldown-cmark 0.13` (CommonMark parser, `default-features = false`), `ratatui` (`Line`/`Span`/`Style`), `cargo nextest`.

**Spec:** `docs/superpowers/specs/2026-05-29-cogito-tui-markdown-design.md`

**API note:** Code below targets `pulldown-cmark 0.13`. Confirm exact
enum variants against docs.rs if a name drifts: `Tag::List(Option<u64>)`
(Some = ordered start number), `TagEnd::List(bool)`, `Tag::Item`,
`Tag::CodeBlock(CodeBlockKind)`, `CodeBlockKind::{Fenced, Indented}`,
`Tag::{Strong, Emphasis}`, `Event::{Text, Code, SoftBreak, HardBreak,
Start, End}`.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` (workspace root) | Add `pulldown-cmark` to `[workspace.dependencies]`. |
| `crates/cogito-tui/Cargo.toml` | Declare `pulldown-cmark.workspace = true`. |
| `crates/cogito-tui/src/ui/markdown.rs` | **New.** Pure markdown -> `Vec<Line>` builder + `MdStyles`. All markdown unit tests live here. |
| `crates/cogito-tui/src/ui/mod.rs` | Add `pub mod markdown;`. |
| `crates/cogito-tui/src/ui/chat.rs` | Replace the `AssistantText` arm with `assistant_lines` (markdown + marker/gutter). Add chat-level tests. |
| `crates/cogito-tui/tests/*` | Audit/refresh snapshot + e2e fixtures whose assistant text now renders differently. |
| `docs/components/cogito-tui.md`, `crates/cogito-tui/README.md` | Document markdown rendering. |

---

## Task 1: Add dependency and module skeleton

**Files:**
- Modify: `Cargo.toml` (workspace root, `[workspace.dependencies]`)
- Modify: `crates/cogito-tui/Cargo.toml` (`[dependencies]`)
- Create: `crates/cogito-tui/src/ui/markdown.rs`
- Modify: `crates/cogito-tui/src/ui/mod.rs`

- [ ] **Step 1: Add the workspace dependency**

In the root `Cargo.toml`, under `[workspace.dependencies]`, add (keep the list alphabetical if it already is):

```toml
pulldown-cmark = { version = "0.13", default-features = false }
```

- [ ] **Step 2: Declare it in cogito-tui**

In `crates/cogito-tui/Cargo.toml`, under `[dependencies]`, in the `# UI` group (next to `ratatui.workspace = true`):

```toml
pulldown-cmark.workspace = true
```

- [ ] **Step 3: Create the module skeleton**

Create `crates/cogito-tui/src/ui/markdown.rs`:

```rust
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
```

- [ ] **Step 4: Register the module**

In `crates/cogito-tui/src/ui/mod.rs`, add to the module declarations (near `pub mod chat;` / `pub mod banner;`):

```rust
pub mod markdown;
```

- [ ] **Step 5: Verify it compiles**

Run: `make fmt && cargo build -p cogito-tui`
Expected: builds clean (the `let _ =` discard avoids the unused-param warning under `-Dwarnings`).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/cogito-tui/Cargo.toml \
        crates/cogito-tui/src/ui/markdown.rs crates/cogito-tui/src/ui/mod.rs
git commit -m "feat(cogito-tui): markdown module skeleton + pulldown-cmark dep"
```

---

## Task 2: Inline emphasis + inline code + paragraphs

Builds the core `Builder` that turns paragraph text and inline
`Strong`/`Emphasis`/`Code` into styled spans.

**Files:**
- Modify: `crates/cogito-tui/src/ui/markdown.rs`

- [ ] **Step 1: Write the failing tests**

Add a test module at the bottom of `markdown.rs`:

```rust
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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `make test CRATE=cogito-tui`
Expected: the new tests FAIL (`render` returns empty `Vec`).

- [ ] **Step 3: Implement the core Builder**

Replace the `render` stub (and the imports block) in `markdown.rs` with:

```rust
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

// ... keep the MdStyles definition above ...

/// Parse `src` as CommonMark and emit body lines (see module docs).
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
    cur: Vec<Span<'static>>,
    /// Nesting depth of `**bold**` we are inside.
    bold: u32,
    /// Nesting depth of `*italic*` we are inside.
    italic: u32,
    /// `true` once any block has been emitted (drives paragraph spacing).
    emitted_block: bool,
}

impl<'s> Builder<'s> {
    fn new(styles: &'s MdStyles) -> Self {
        Self {
            styles,
            lines: Vec::new(),
            cur: Vec::new(),
            bold: 0,
            italic: 0,
            emitted_block: false,
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
        self.cur
            .push(Span::styled(text.to_string(), self.inline_style()));
    }

    /// Flush the current spans as a finished line (even if empty).
    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.cur);
        self.lines.push(Line::from(spans));
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Paragraph) => {
                // Blank separator between consecutive blocks.
                if self.emitted_block {
                    self.lines.push(Line::from(Vec::new()));
                }
            }
            Event::End(TagEnd::Paragraph) => {
                self.flush_line();
                self.emitted_block = true;
            }
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Code(text) => {
                self.cur
                    .push(Span::styled(text.to_string(), self.styles.code_inline));
            }
            Event::Text(text) => self.push_text(&text),
            _ => {}
        }
    }

    /// Flush any trailing partial line and return all lines.
    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.cur.is_empty() {
            self.flush_line();
        }
        self.lines
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make test CRATE=cogito-tui`
Expected: all Task 2 tests PASS.

- [ ] **Step 5: Lint**

Run: `make fix CRATE=cogito-tui`
Expected: clean (no clippy warnings).

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-tui/src/ui/markdown.rs
git commit -m "feat(cogito-tui): markdown inline emphasis, code, paragraphs"
```

---

## Task 3: Fenced and indented code blocks

Code-block contents render one styled line per source line, indented,
with no inline parsing inside.

**Files:**
- Modify: `crates/cogito-tui/src/ui/markdown.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
#[test]
fn code_block_lines_use_code_block_style_and_indent() {
    let src = "```\nlet x = 1;\nlet y = 2;\n```";
    let out = render(src, &styles());
    // two code lines
    let code_lines: Vec<_> = out
        .iter()
        .filter(|l| text_of(l).contains("let "))
        .collect();
    assert_eq!(code_lines.len(), 2);
    // every non-blank span on a code line carries the code_block style
    for l in &code_lines {
        let styled = l
            .spans
            .iter()
            .find(|s| s.content.contains("let"))
            .unwrap();
        assert!(styled.style.add_modifier.contains(Modifier::DIM));
    }
    // indented (leading-space span present)
    assert!(text_of(code_lines[0]).starts_with(' '));
}

#[test]
fn asterisks_inside_code_block_stay_literal() {
    let src = "```\n**not bold**\n```";
    let out = render(src, &styles());
    let line = out.iter().find(|l| text_of(l).contains("not bold")).unwrap();
    assert!(text_of(line).contains("**not bold**"));
    // no span carries BOLD
    assert!(line.spans.iter().all(|s| !s.style.add_modifier.contains(Modifier::BOLD)));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `make test CRATE=cogito-tui`
Expected: the two new tests FAIL (code block text currently flows
through `Event::Text` as plain paragraph text or is mis-indented).

- [ ] **Step 3: Implement code-block handling**

Add a `NEST_INDENT` const near the top of the file (after imports):

```rust
/// Spaces added per indent level (code blocks, nested list content).
const NEST_INDENT: usize = 2;
```

Add an `in_code_block: bool` field to `Builder` (init `false` in `new`).
Extend `handle` — add these arms (before the catch-all `_ => {}`). Reuse
the `emit_block_separator()` helper and the `prev_block_emitted` flag
introduced in the Task 2 review refactor:

```rust
Event::Start(Tag::CodeBlock(_)) => {
    self.emit_block_separator();
    self.in_code_block = true;
}
Event::End(TagEnd::CodeBlock) => {
    self.in_code_block = false;
    self.prev_block_emitted = true;
}
```

And change the `Event::Text` arm to special-case code blocks:

```rust
Event::Text(text) => {
    if self.in_code_block {
        // pulldown emits code-block text with trailing newlines;
        // render each source line as its own indented, styled line.
        for raw in text.lines() {
            let indent = " ".repeat(NEST_INDENT);
            self.lines.push(Line::from(vec![
                Span::raw(indent),
                Span::styled(raw.to_string(), self.styles.code_block),
            ]));
        }
    } else {
        self.push_text(&text);
    }
}
```

(Remove the old standalone `Event::Text` arm so there is exactly one.)

- [ ] **Step 4: Run to verify they pass**

Run: `make test CRATE=cogito-tui`
Expected: all tests PASS, including Task 2's.

- [ ] **Step 5: Lint + commit**

```bash
make fix CRATE=cogito-tui
git add crates/cogito-tui/src/ui/markdown.rs
git commit -m "feat(cogito-tui): markdown fenced code blocks"
```

---

## Task 4: Bullet and numbered lists

List items render as `- text` (unordered) or `1. text` (ordered) with
the marker styled `list_marker`; one nesting level indents by
`NEST_INDENT`.

**Files:**
- Modify: `crates/cogito-tui/src/ui/markdown.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
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
    let marker = items[0].spans.iter().find(|s| s.content.contains('-')).unwrap();
    assert_eq!(marker.style.fg, Some(Color::Green));
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
    let lead = |l: &Line| text_of(l).len() - text_of(l).trim_start().len();
    assert!(lead(nested) > lead(top));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `make test CRATE=cogito-tui`
Expected: the three list tests FAIL.

- [ ] **Step 3: Implement list handling**

Add a list-context stack to `Builder`. Add field:

```rust
/// Active list levels. `Some(n)` = ordered (next number), `None` = bullet.
lists: Vec<Option<u64>>,
```

Init `lists: Vec::new()` in `new`. Add a helper:

```rust
/// Leading indent (spaces) implied by current list nesting.
fn list_indent(&self) -> usize {
    self.lists.len().saturating_sub(1) * NEST_INDENT
}
```

Add these arms to `handle` (before `_ => {}`):

```rust
Event::Start(Tag::List(first)) => {
    if self.lists.is_empty() {
        self.emit_block_separator();
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
        self.cur.push(Span::raw(indent));
    }
    self.cur.push(Span::styled(marker, self.styles.list_marker));
}
Event::End(TagEnd::Item) => {
    self.flush_line();
}
```

Note: inside list items pulldown wraps text in a `Paragraph`. The
existing `Start(Tag::Paragraph)` arm pushes a blank separator when
`emitted_block` is true, which would break list items apart. Guard it:
change the `Start(Tag::Paragraph)` and `End(TagEnd::Paragraph)` arms to
no-op while inside a list (the `Item` start/end manage list lines):

```rust
Event::Start(Tag::Paragraph) => {
    if self.lists.is_empty() {
        self.emit_block_separator();
    }
}
Event::End(TagEnd::Paragraph) => {
    // Inside a list item, the Item end flushes the line instead.
    if self.lists.is_empty() {
        self.flush_line();
        self.prev_block_emitted = true;
    }
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `make test CRATE=cogito-tui`
Expected: all tests PASS (lists + earlier tasks).

- [ ] **Step 5: Lint + commit**

```bash
make fix CRATE=cogito-tui
git add crates/cogito-tui/src/ui/markdown.rs
git commit -m "feat(cogito-tui): markdown bullet and numbered lists"
```

---

## Task 5: Line breaks, plain-text degradation, robustness

Soft/hard breaks split lines; headings, block quotes, and links emit
their inner text in the default style; unterminated markup never panics.

**Files:**
- Modify: `crates/cogito-tui/src/ui/markdown.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
#[test]
fn soft_break_starts_a_new_line() {
    let out = render("line one\nline two", &styles());
    assert_eq!(text_of(&out[0]), "line one");
    assert_eq!(text_of(&out[1]), "line two");
}

#[test]
fn heading_text_degrades_to_plain() {
    let out = render("# Title", &styles());
    let line = out.iter().find(|l| text_of(l).contains("Title")).unwrap();
    // no '#' rendered; text present and unstyled (no bold modifier)
    assert!(!text_of(line).contains('#'));
    assert!(line.spans.iter().all(|s| !s.style.add_modifier.contains(Modifier::BOLD)));
}

#[test]
fn link_renders_text_and_drops_url() {
    let out = render("see [docs](https://example.com) here", &styles());
    let joined: String = out.iter().map(text_of).collect();
    assert!(joined.contains("docs"));
    assert!(!joined.contains("example.com"));
}

#[test]
fn unterminated_bold_does_not_panic() {
    let out = render("**oops no close", &styles());
    let joined: String = out.iter().map(text_of).collect();
    assert!(joined.contains("oops"));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `make test CRATE=cogito-tui`
Expected: `soft_break_starts_a_new_line` and possibly `heading_text_degrades_to_plain` FAIL (soft breaks currently ignored; headings flow as paragraph text but may include a stray separator). `link` and `unterminated` may already pass — confirm.

- [ ] **Step 3: Implement breaks + heading/blockquote handling**

Add these arms to `handle` (before `_ => {}`):

```rust
Event::SoftBreak | Event::HardBreak => {
    if !self.in_code_block {
        self.flush_line();
    }
}
Event::Start(Tag::Heading { .. }) => self.emit_block_separator(),
Event::End(TagEnd::Heading(_)) => {
    self.flush_line();
    self.prev_block_emitted = true;
}
```

`Link` and `BlockQuote` need no explicit arm: their `Start`/`End` fall
through the catch-all `_ => {}`, while the inner `Event::Text` (the link
label / quote body) is emitted in the default inline style and the URL
(carried only in the `Tag::Link` payload, never as a `Text` event) is
dropped. The catch-all already covers `Image`, `Html`, `Rule`,
`TaskListMarker`, etc.

- [ ] **Step 4: Run to verify they pass**

Run: `make test CRATE=cogito-tui`
Expected: all `markdown.rs` tests PASS.

- [ ] **Step 5: Lint + commit**

```bash
make fix CRATE=cogito-tui
git add crates/cogito-tui/src/ui/markdown.rs
git commit -m "feat(cogito-tui): markdown line breaks + plain-text degradation"
```

---

## Task 6: Wire markdown into the chat widget

Replace the single-`Line` `AssistantText` rendering with markdown body
lines plus the `∴ ` marker / 3-space gutter prefix.

**Files:**
- Modify: `crates/cogito-tui/src/ui/chat.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `chat.rs`:

```rust
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
```

Keep the existing `assistant_text_renders_with_marker_prefix` test
(plain "I am cogito.") — it must still pass unchanged.

- [ ] **Step 2: Run to verify the new tests fail**

Run: `make test CRATE=cogito-tui`
Expected: the two new chat tests FAIL (`**` still literal; newline not split).

- [ ] **Step 3: Implement `assistant_lines` and rewire the match arm**

In `chat.rs`, add the markdown import at the top:

```rust
use crate::ui::markdown::{self, MdStyles};
```

Add `Modifier`-based style fields are already imported. Add a helper to
build `MdStyles` from the palette and a function producing the lines:

```rust
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
        // Message started but no content yet — bare marker line.
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
```

Replace the match arm in `render`:

```rust
ChatLine::AssistantText(text) => out.extend(assistant_lines(text, &p)),
```

Delete the now-unused `cogito_line` function (and its lone caller above)
to avoid a dead-code warning under `-Dwarnings`. (`thinking_line`,
`user_line`, `notice_line` stay.)

- [ ] **Step 4: Run to verify they pass**

Run: `make test CRATE=cogito-tui`
Expected: new chat tests PASS; `assistant_text_renders_with_marker_prefix` still PASS (plain text -> single `∴  I am cogito.` line).

- [ ] **Step 5: Lint + commit**

```bash
make fix CRATE=cogito-tui
git add crates/cogito-tui/src/ui/chat.rs
git commit -m "feat(cogito-tui): render assistant markdown in chat pane"
```

---

## Task 7: Refresh integration snapshots + add a representative snapshot

**Files:**
- Modify: `crates/cogito-tui/src/ui/chat.rs` (one snapshot test)
- Audit/modify: `crates/cogito-tui/tests/*` (e2e + snapshot fixtures)

- [ ] **Step 1: Add a representative composite snapshot test**

In `chat.rs` tests, add:

```rust
#[test]
fn assistant_composite_markdown_snapshot() {
    let mut chat = ChatModel::new();
    chat.on_event(&StreamEvent::TextDelta {
        chunk: "Plan:\n\n- step **one**\n- step `two`\n\n```\ncode\n```".into(),
    });
    let tools = empty_tools();
    let out = draw(&chat, &tools, None, &HashSet::new(), false, 0, 40, 12);
    // structural assertions (not a brittle full-buffer match)
    assert!(out.contains("∴  Plan:"), "got:\n{out}");
    assert!(out.contains("- step one"), "got:\n{out}");
    assert!(out.contains("- step two"), "got:\n{out}");
    assert!(out.contains("code"), "got:\n{out}");
    assert!(!out.contains("```"), "got:\n{out}");
    assert!(!out.contains("**"), "got:\n{out}");
}
```

- [ ] **Step 2: Find integration fixtures that render assistant text**

Run: `grep -rln "AssistantText\|TextDelta\|∴" crates/cogito-tui/tests`
Then, for each hit, inspect whether its assistant text contains markdown
characters (`*`, `` ` ``, `-`, `#`, newlines) that now render
differently.

- [ ] **Step 3: Run the full crate test suite**

Run: `make test CRATE=cogito-tui`
Expected: the new snapshot test passes. If any `tests/*` snapshot now
mismatches, the failure output shows the new buffer.

- [ ] **Step 4: Update mismatched snapshots to the new expected output**

For each failing snapshot, verify the new rendering is correct by
reading the diff (markdown now styled/split), then update the expected
string/file to match. Do NOT `#[ignore]` any test. If a fixture used
literal `*`/`` ` `` in assistant text that was incidental, prefer
changing the fixture text to plain words so the snapshot stays about
what it was testing.

- [ ] **Step 5: Re-run to confirm green**

Run: `make test CRATE=cogito-tui`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-tui/src/ui/chat.rs crates/cogito-tui/tests
git commit -m "test(cogito-tui): markdown snapshot + refreshed fixtures"
```

---

## Task 8: Documentation + full CI

**Files:**
- Modify: `docs/components/cogito-tui.md`
- Modify: `crates/cogito-tui/README.md`

- [ ] **Step 1: Document markdown rendering in the component doc**

In `docs/components/cogito-tui.md`, add a "Markdown rendering" subsection
describing: assistant replies are parsed with `pulldown-cmark` at draw
time into styled lines; supported = bold/italic/inline-code/code-blocks/
lists; headings/blockquotes/links degrade to plain text; the `ui::markdown`
module is pure and the `∴` marker is added by `ui::chat`. Note that
per-frame re-parse is intentional pending the incremental-projection
sprint.

- [ ] **Step 2: Mention markdown in the crate README**

In `crates/cogito-tui/README.md`, add markdown rendering to the feature
list for the chat pane.

- [ ] **Step 3: Run full CI**

Run: `make ci`
Expected: fmt-check + clippy (`-D warnings`) + layer-check + full
workspace test all GREEN. (Layer-check confirms cogito-tui adding a
Surface-layer dep does not violate ADR-0004.)

- [ ] **Step 4: Commit**

```bash
git add docs/components/cogito-tui.md crates/cogito-tui/README.md
git commit -m "docs(cogito-tui): document markdown rendering"
```

- [ ] **Step 5: Update the deferred-item tracking**

In `docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`,
the "Out of scope" item 1 (markdown rendering) is now done — leave the
spec as historical record but add a one-line note pointing to this
plan/spec, or tick it in the ROADMAP note if tracked there. Commit:

```bash
git add docs
git commit -m "docs: mark cogito-tui markdown rendering shipped"
```

---

## Self-Review

**Spec coverage:**
- Code blocks -> Task 3. Bold/italic/inline code -> Task 2. Lists -> Task 4. (spec scope) ✓
- Applied to `AssistantText` only -> Task 6 (only that arm changes). ✓
- markdown module emits body lines, chat owns marker -> Task 1 (API) + Task 6 (prefix). ✓
- Headings/blockquotes/links degrade to plain text -> Task 5. ✓
- Soft break -> line break -> Task 5. ✓
- Inline code = yellow, code block = dim, list marker = green -> Task 6 `md_styles`. ✓
- `pulldown-cmark`, `default-features = false`, workspace dep -> Task 1. ✓
- Streaming tolerance (unterminated markup) -> Task 5 test. ✓
- Per-frame parse accepted; `render_model` untouched -> no task modifies `render_model`. ✓
- Tests: markdown unit tests (Tasks 2-5), chat tests + snapshot (Tasks 6-7), fixture audit (Task 7), existing plain-text test preserved (Task 6). ✓
- Docs: component doc + README + deferred-item note -> Task 8. ✓

**Type consistency:** `MdStyles` fields (`bold`/`italic`/`code_inline`/`code_block`/`list_marker`) are identical across Task 1 definition, Task 2 test helper, and Task 6 `md_styles`. `render(src, &MdStyles) -> Vec<Line<'static>>` signature stable across all tasks. `assistant_lines`/`md_styles` names consistent in Task 6.

**Placeholder scan:** No TBD/TODO; every code step shows complete code; commands have expected output.

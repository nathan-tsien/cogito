# cogito-tui — markdown rendering in chat (design)

Date: 2026-05-29
Status: approved (brainstorm), pending implementation plan
Follows: `docs/superpowers/specs/2026-05-29-cogito-tui-redesign-design.md`
(PR #28) — this is the deferred "Markdown rendering in chat" item from
that spec's "Out of scope" list.

## Problem

After the v0.2 TUI redesign (PR #28), assistant replies still render as
flat raw text. `ChatModel` stores each assistant message as
`ChatLine::AssistantText(String)` — the whole accumulated reply,
newlines included — and `ui::chat::cogito_line` paints it as a single
`Line` with a `∴  ` marker prefix and one `Span::raw(text)`. No
emphasis, no code blocks, no list structure; embedded newlines are not
even split into separate visual lines.

Real agent replies are markdown. The redesign spec explicitly deferred
markdown rendering to its own sprint (spec §"Out of scope", item 1:
"Code blocks, bold/italic, inline code, lists. Needs `tui-markdown` or
a custom span builder. Own sprint."). This is that sprint.

## Scope

In scope (locked during brainstorm):

- Fenced/indented code blocks
- Bold (`**`), italic (`*` / `_`), inline code (`` ` ``)
- Bullet and numbered lists (with nesting)
- Applied to `ChatLine::AssistantText` only

Explicitly out of scope (still deferred):

- Markdown inside tool args / result previews (redesign spec item 6)
- Markdown for thinking blocks, user prompts, system notices
- Styling of headings / blockquotes / links — these degrade to plain
  text (see event table)
- Incremental `ChatModel` projection (redesign spec item 2) — text is
  still re-parsed every frame; accepted cost, see "Performance"
- Message animations, theme system, mouse support

## Approach

Add `pulldown-cmark` (the de-facto Rust CommonMark parser) and write a
custom span builder that translates its event stream into
`ratatui::text::Line`s. Chosen over the `tui-markdown` crate (emits its
own fixed styling we'd have to fight to graft on our marker / indent /
palette) and over a hand-rolled parser (would reimplement a slice of
CommonMark and risk emphasis edge cases). pulldown-cmark is pure Rust,
low MSRV (fits MSRV 1.85 / edition 2024), and battle-tested. The custom
builder keeps full control of marker, indent, and palette, matching the
existing lazy-palette-painting design in `ui::chat`.

## Architecture

### New module: `crates/cogito-tui/src/ui/markdown.rs`

Pure, IO-free, palette-injected. Knows nothing about the `∴` marker or
the chat gutter — it emits body lines only. `ui::chat` owns the marker.

```rust
/// Styles the markdown builder applies. Built from the chat Palette.
pub struct MdStyles {
    pub bold: Style,
    pub italic: Style,
    pub code_inline: Style,
    pub code_block: Style,
    pub list_marker: Style,
}

/// Parse `src` as CommonMark and emit body lines with NO outer
/// gutter/marker. Nested-list and code-block indentation are encoded
/// as leading-space spans on each line. Empty input -> empty Vec.
pub fn render(src: &str, styles: &MdStyles) -> Vec<Line<'static>>;
```

Internally it walks `pulldown_cmark::Parser`, maintaining:

- the current line's span vector,
- an inline style stack (bold / italic / inline-code compose by
  pushing/popping),
- a list-context stack (ordered vs unordered, item counter, depth),
- a code-block-mode flag.

### Event handling (scope = spec list only)

| pulldown event | rendering |
|---|---|
| `Start(Strong)` / `End` | push / pop BOLD on the inline style stack |
| `Start(Emphasis)` / `End` | push / pop ITALIC |
| `Code(text)` (inline) | span styled `code_inline`; not recursively parsed |
| `Start(CodeBlock)` .. `Text` .. `End` | each inner line -> extra-indented span styled `code_block`; no inline parsing inside |
| `Start(List(first))` / `End` | push/pop list context (ordered if `Some(n)`) |
| `Start(Item)` / `End` | begin a new line with `- ` (unordered) or `N. ` (ordered) marker styled `list_marker`, indented by depth |
| `SoftBreak` / `HardBreak` | start a new line (preserve the author's line breaks rather than collapsing soft breaks to spaces) |
| `Start(Paragraph)` / `End` | paragraph boundary -> blank separator line between paragraphs (no leading blank) |
| `Text(t)` | append a span styled by the current inline stack |
| headings / blockquotes / links | degrade to plain text: emit inner text in default style; links drop the URL |

`pulldown_cmark::Options` left at defaults (no tables / footnotes /
strikethrough extensions — out of scope; they degrade to plain text if
present).

### Integration in `crate::ui::chat`

The `AssistantText` match arm changes from a single `cogito_line` to:

```rust
ChatLine::AssistantText(text) => out.extend(assistant_lines(text, &p)),
```

`assistant_lines(text, &p)`:

1. Build `MdStyles` from the `Palette`.
2. `let mut body = markdown::render(text, &styles);`
3. If `body` is empty (message started, no delta yet), return a single
   bare `∴  ` marker line.
4. Prepend a prefix span to each line: a green `Span::styled("∴  ",
   p.cogito)` on line 0, a `Span::raw("   ")` 3-space gutter on every
   other line. Both are width 3, preserving today's column alignment.

Style defaults (built from `Palette`, tunable):

- base text — default terminal fg (`Span::raw`), matching today; only
  the marker is green
- `bold` — `Modifier::BOLD`
- `italic` — `Modifier::ITALIC`
- `code_inline` — `Color::Yellow`
- `code_block` — dim / gray
- `list_marker` — `p.cogito` (subtle green accent)

## Data flow

Unchanged upstream: `StreamEvent::TextDelta` still coalesces into
`ChatLine::AssistantText` in `render_model`. The markdown parse happens
only at draw time inside `ui::chat`, so `render_model` needs no change.
This keeps the "state model is sink-agnostic, palette/layout applied at
render time" separation intact.

## Streaming / partial input

`ChatModel` is reprojected from the event log every frame, so the
accumulated (possibly mid-token) assistant text is re-parsed every
frame. pulldown-cmark tolerates incomplete input: an unterminated `**`
renders as a literal asterisk run, an unterminated fence renders as a
code block to end-of-text. Mid-stream frames therefore render sanely
with no special handling. A test asserts an unterminated `**bold` does
not panic.

## Error handling

The builder never panics and never returns `Result`: malformed or
exotic markdown degrades to plain text. `serde`/IO are not involved.
No `unwrap`/`expect`/`panic` (workspace clippy denies them).

## Performance

Re-parsing every frame is O(message length) per assistant message per
frame. Acceptable for v0.2; the redesign spec already carved out
"incremental ChatModel projection" as a separate sprint that will fix
per-frame reprojection wholesale (markdown parse included). Noted, not
optimized here.

## Dependency

Root `[workspace.dependencies]`:

```toml
pulldown-cmark = { version = "0.13", default-features = false }
```

`default-features = false` drops the unused `html` render feature,
keeping the build lean; the `Parser`/`Event` API stays available.
`cogito-tui/Cargo.toml` declares `pulldown-cmark.workspace = true`.
Surface-layer (cogito-tui) dependency only — no Brain/Hands/Session
layer-boundary impact (ADR-0004).

## Testing strategy

`markdown.rs` unit tests (pure, no terminal):

- bold / italic / inline-code spans carry the right `Modifier`/`Style`
- composed `**bold _and italic_**` yields a BOLD+ITALIC span
- fenced code block: inner lines styled `code_block`, `**` inside a
  code block stays literal (not parsed)
- unordered list -> `- ` markers; ordered list -> `1. 2. ...` markers;
  one nesting level indents
- soft break and hard break each start a new line
- unterminated `**bold` does not panic and emits a line
- empty input -> empty `Vec`
- heading text and link text degrade to plain (default) style; link URL
  dropped

`ui::chat` tests (TestBackend):

- assistant markdown message: `∴  ` on the first visual line, 3-space
  gutter on continuation lines
- one representative snapshot (bold + inline code + a fenced block + a
  list)
- existing `assistant_text_renders_with_marker_prefix` (plain text "I
  am cogito.") still passes: no markdown -> one `∴  I am cogito.` line

Integration (`crates/cogito-tui/tests/`): audit existing e2e / snapshot
fixtures for multiline assistant text whose rendering changes, and
update snapshots accordingly. Never `#[ignore]` to dodge a diff.

## Docs to update on completion

- `docs/components/cogito-tui.md` — add a markdown-rendering section
- `crates/cogito-tui/README.md` — mention markdown in chat
- Tick the deferred "markdown rendering" item where it is tracked
  (redesign spec "Out of scope" / ROADMAP note)

## Decisions locked during brainstorm

| Question | Decision |
|---|---|
| Feature scope | Spec list only: code blocks, bold/italic, inline code, lists. Headings/blockquotes/links degrade to plain text. |
| Where applied | `AssistantText` only. Thinking / user / notices / tool previews stay raw. |
| Implementation | `pulldown-cmark` + custom span builder (approach B). |
| Marker coupling | markdown module emits body lines only; `ui::chat` prepends `∴`/gutter. |
| Inline code color | `Color::Yellow`. |
| Soft break | renders as a line break (preserve author line breaks). |

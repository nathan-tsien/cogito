# Dev Experience — Compact Model Debug Log + REPL Role Coloring — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace pretty-printed multi-line debug JSON in both model adapters with a single compact `tracing::debug!` line, and add an ANSI-colored REPL renderer to `cogito chat` that distinguishes user / agent / tool messages.

**Architecture:** Two independent changes touching two crates. (1) `cogito-model` — change `serde_json::to_string_pretty` → `serde_json::to_string`, drop ASCII frame, add parity in Anthropic adapter (currently missing). (2) `cogito-cli` — new `render.rs` module owning all REPL output via a `Renderer<W: Write>` driven by `StreamEvent`, with TTY auto-detect for color fallback.

**Tech Stack:** Rust 2024, `tracing`, `tokio`, `cogito-protocol::stream::StreamEvent`, `std::io::IsTerminal`. No new workspace dependencies.

**Spec:** `docs/superpowers/specs/2026-05-21-dev-experience-cli-display-design.md` (commit `e5d327a`)

---

## File Structure

| Path | Action | Responsibility |
| --- | --- | --- |
| `crates/cogito-model/src/openai_compat/mod.rs` | Modify (lines 109-118) | Compact request-body debug log |
| `crates/cogito-model/src/anthropic/mod.rs` | Modify (insert after line 76) | Add parity compact request-body debug log |
| `crates/cogito-cli/src/render.rs` | Create | `Renderer<W>` — translate `StreamEvent` to ANSI-colored stdout |
| `crates/cogito-cli/src/main.rs` | Modify (add `mod render;`) | Register the new module |
| `crates/cogito-cli/src/chat.rs` | Modify (lines 191-228) | Use `Renderer` instead of raw `print!` |

---

## Task 1: Model-side — Compact OpenAI-compat debug log

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/mod.rs:109-118`

- [ ] **Step 1: Replace the pretty-print debug block**

Open `crates/cogito-model/src/openai_compat/mod.rs`. Find lines 109-118:

```rust
        if tracing::enabled!(tracing::Level::DEBUG) {
            match serde_json::to_string_pretty(&body) {
                Ok(json) => {
                    tracing::debug!(target: "cogito::prompt", url = %url, "\n── request body ──\n{json}\n──────────────────");
                }
                Err(e) => {
                    tracing::debug!(target: "cogito::prompt", "request body serialization failed: {e}");
                }
            }
        }
```

Replace with:

```rust
        if tracing::enabled!(tracing::Level::DEBUG) {
            match serde_json::to_string(&body) {
                Ok(json) => {
                    tracing::debug!(target: "cogito::prompt", url = %url, "request: {json}");
                }
                Err(e) => {
                    tracing::debug!(target: "cogito::prompt", "request body serialization failed: {e}");
                }
            }
        }
```

Changes: `to_string_pretty` → `to_string`; message format `"\n── request body ──\n{json}\n──────────────────"` → `"request: {json}"`.

- [ ] **Step 2: Run fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-model`
Expected: no output (or only "Compiling cogito-model …" then exit 0).

- [ ] **Step 3: Run tests**

Run: `make test CRATE=cogito-model`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-model/src/openai_compat/mod.rs
git commit -m "$(cat <<'EOF'
chore(model): compact openai-compat prompt debug log to one line

Drops pretty-print + ASCII separator frame. Long request bodies no
longer scroll the terminal off-screen. Single compact `tracing::debug!`
line per turn. Addresses GitLab #2.
EOF
)"
```

---

## Task 2: Model-side — Add parity debug log in Anthropic adapter

**Files:**
- Modify: `crates/cogito-model/src/anthropic/mod.rs` (insert after line 76)

- [ ] **Step 1: Insert the debug-log block after URL construction**

Open `crates/cogito-model/src/anthropic/mod.rs`. After line 76 (`let url = format!(...)`), and before line 77 (`let response = self`), insert:

```rust

        if tracing::enabled!(tracing::Level::DEBUG) {
            match serde_json::to_string(&body) {
                Ok(json) => {
                    tracing::debug!(target: "cogito::prompt", url = %url, "request: {json}");
                }
                Err(e) => {
                    tracing::debug!(target: "cogito::prompt", "request body serialization failed: {e}");
                }
            }
        }

```

(Leading and trailing blank lines so it visually separates from the surrounding code.)

- [ ] **Step 2: Run fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-model`
Expected: clean.

- [ ] **Step 3: Run tests**

Run: `make test CRATE=cogito-model`
Expected: all pass.

- [ ] **Step 4: Manual sanity (optional but recommended)**

If you have an Anthropic key configured, run a quick chat turn with `RUST_LOG=debug make chat`. You should see one `request: {…}` line per turn from `cogito::prompt`. If you don't have a key, skip — the unit-test green is enough.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/anthropic/mod.rs
git commit -m "$(cat <<'EOF'
chore(model): add compact prompt debug log to anthropic adapter

The openai-compat adapter has emitted request-body debug logs since
Sprint 2; anthropic had none. Brings them to parity with the same
one-line compact JSON format. Addresses GitLab #2.
EOF
)"
```

---

## Task 3: Create `render.rs` skeleton + first failing test (plain text)

**Files:**
- Create: `crates/cogito-cli/src/render.rs`
- Modify: `crates/cogito-cli/src/main.rs` (add `mod render;`)

- [ ] **Step 1: Register the module**

Open `crates/cogito-cli/src/main.rs`. Find the existing module declarations:

```rust
mod banner;
mod chat;
```

Add `mod render;` alphabetically:

```rust
mod banner;
mod chat;
mod render;
```

- [ ] **Step 2: Create the renderer skeleton**

Create `crates/cogito-cli/src/render.rs` with the following contents:

```rust
//! ANSI-colored REPL renderer for `cogito chat`.
//!
//! Translates `StreamEvent`s into role-tagged stdout output. TTY
//! detection (`std::io::IsTerminal`) degrades the color path to plain
//! text when stdout is not a terminal (e.g. piped to `cat` or
//! redirected to a file).
//!
//! See `docs/superpowers/specs/2026-05-21-dev-experience-cli-display-design.md`.

use std::collections::HashMap;
use std::io::{IsTerminal, Result as IoResult, Write};
use std::time::Instant;

use cogito_protocol::stream::StreamEvent;

const CYAN: &str = "36";
const GREEN: &str = "32";
const RED: &str = "31";
const DIM: &str = "2";
const DIM_YELLOW: &str = "2;33";

/// REPL renderer driven by `StreamEvent`s. Generic over the output
/// sink so tests can drive it with a `Vec<u8>` buffer.
pub struct Renderer<W: Write> {
    out: W,
    color: bool,
    in_text: bool,
    tool_timers: HashMap<String, (Instant, String)>,
}

impl<W: Write> Renderer<W> {
    /// Construct a renderer over an explicit writer. `color` controls
    /// whether ANSI escape sequences are emitted.
    #[must_use]
    pub fn new(out: W, color: bool) -> Self {
        Self {
            out,
            color,
            in_text: false,
            tool_timers: HashMap::new(),
        }
    }

    /// Print the user-input prompt `> ` and flush so the cursor sits
    /// to the right of the prompt before `stdin` reads.
    pub fn prompt_user(&mut self) -> IoResult<()> {
        let prompt = self.paint(CYAN, "> ");
        write!(self.out, "{prompt}")?;
        self.out.flush()
    }

    /// Render one `StreamEvent` to the output sink.
    pub fn on_stream_event(&mut self, ev: &StreamEvent) -> IoResult<()> {
        // Fully implemented in Tasks 4-6.
        let _ = ev;
        Ok(())
    }

    fn paint(&self, code: &str, body: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{body}\x1b[0m")
        } else {
            body.to_string()
        }
    }
}

impl Renderer<std::io::Stdout> {
    /// Convenience: construct a renderer over `std::io::stdout()` with
    /// color enabled iff stdout is a terminal.
    #[must_use]
    pub fn for_stdout() -> Self {
        let out = std::io::stdout();
        let color = out.is_terminal();
        Self::new(out, color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_events(events: &[StreamEvent]) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = Renderer::new(&mut buf, false);
            for e in events {
                r.on_stream_event(e).unwrap();
            }
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn plain_text_sequence_no_color() {
        let out = render_events(&[
            StreamEvent::TurnStarted,
            StreamEvent::TextDelta { chunk: "hi".into() },
            StreamEvent::TextDelta { chunk: " there".into() },
            StreamEvent::TurnCompleted,
        ]);
        assert_eq!(out, "\nagent: hi there\n");
    }
}
```

- [ ] **Step 3: Run the test — expect FAIL**

Run: `cargo nextest run -p cogito-cli render::tests::plain_text_sequence_no_color`
Expected: FAIL with `assertion `left == right` failed` — left is `""`, right is `"\nagent: hi there\n"`. (`on_stream_event` is a stub, so nothing is written.)

- [ ] **Step 4: Implement `TurnStarted`, `TextDelta`, `TurnCompleted` in `on_stream_event`**

Replace the body of `on_stream_event` with:

```rust
    pub fn on_stream_event(&mut self, ev: &StreamEvent) -> IoResult<()> {
        match ev {
            StreamEvent::TurnStarted => {
                self.in_text = false;
            }
            StreamEvent::TextDelta { chunk } => {
                if !self.in_text {
                    let label = self.paint(GREEN, "agent: ");
                    write!(self.out, "\n{label}")?;
                    self.in_text = true;
                }
                write!(self.out, "{chunk}")?;
                self.out.flush()?;
            }
            StreamEvent::TurnCompleted => {
                writeln!(self.out)?;
                self.in_text = false;
            }
            // Other variants land in subsequent tasks.
            _ => {}
        }
        Ok(())
    }
```

- [ ] **Step 5: Run the test — expect PASS**

Run: `cargo nextest run -p cogito-cli render::tests::plain_text_sequence_no_color`
Expected: PASS.

- [ ] **Step 6: fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-cli`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/cogito-cli/src/main.rs crates/cogito-cli/src/render.rs
git commit -m "$(cat <<'EOF'
feat(cli): introduce render::Renderer skeleton with text-delta path

New `crates/cogito-cli/src/render.rs` owns REPL output. First behavior
implemented: agent text delta sequences print a single `agent: `
prefix per text block; `TurnCompleted` writes a turn boundary
newline. Tool / lifecycle variants land in the next tasks.

Color path is in place but disabled in this commit; ANSI codes will
be exercised by the color-balance test in a follow-up commit.
EOF
)"
```

---

## Task 4: Tool dispatch lifecycle (happy path + error path)

**Files:**
- Modify: `crates/cogito-cli/src/render.rs`

- [ ] **Step 1: Add failing tool-lifecycle tests**

Append to the existing `mod tests` block inside `crates/cogito-cli/src/render.rs`:

```rust
    #[test]
    fn tool_lifecycle_ok_no_color() {
        let out = render_events(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
            },
            StreamEvent::TurnCompleted,
        ]);
        assert!(out.starts_with("\n[tool] read_file …\n[tool] read_file ok ("),
                "unexpected start: {out:?}");
        assert!(out.ends_with("ms)\n"),
                "unexpected end: {out:?}");
    }

    #[test]
    fn tool_lifecycle_err_no_color() {
        let out = render_events(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "bad_tool".into(),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c2".into(),
                ok: false,
            },
        ]);
        assert!(out.contains("[tool] bad_tool err ("),
                "expected 'err' marker, got: {out:?}");
    }

    #[test]
    fn text_after_tool_reprints_agent_prefix() {
        let out = render_events(&[
            StreamEvent::TextDelta { chunk: "a".into() },
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
            },
            StreamEvent::TextDelta { chunk: "b".into() },
        ]);
        let count = out.matches("agent: ").count();
        assert_eq!(count, 2, "expected two `agent: ` prefixes, got: {out:?}");
    }
```

- [ ] **Step 2: Run the new tests — expect FAIL**

Run: `cargo nextest run -p cogito-cli render::tests`
Expected: `tool_lifecycle_ok_no_color`, `tool_lifecycle_err_no_color`, `text_after_tool_reprints_agent_prefix` all FAIL (`on_stream_event` doesn't handle tool variants). `plain_text_sequence_no_color` still passes.

- [ ] **Step 3: Implement tool-dispatch variants**

In `on_stream_event`, replace the trailing `// Other variants land in subsequent tasks.` arm with explicit handling. The full match should become:

```rust
    pub fn on_stream_event(&mut self, ev: &StreamEvent) -> IoResult<()> {
        match ev {
            StreamEvent::TurnStarted => {
                self.in_text = false;
            }
            StreamEvent::TextDelta { chunk } => {
                if !self.in_text {
                    let label = self.paint(GREEN, "agent: ");
                    write!(self.out, "\n{label}")?;
                    self.in_text = true;
                }
                write!(self.out, "{chunk}")?;
                self.out.flush()?;
            }
            StreamEvent::ToolDispatchStarted { call_id, tool_name } => {
                self.tool_timers
                    .insert(call_id.clone(), (Instant::now(), tool_name.clone()));
                let line = self.paint(DIM_YELLOW, &format!("[tool] {tool_name} …"));
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::ToolDispatchEnded { call_id, ok } => {
                let (name, ms) = match self.tool_timers.remove(call_id) {
                    Some((started, name)) => (name, started.elapsed().as_millis()),
                    None => ("?".to_string(), 0u128),
                };
                let status = if *ok { "ok" } else { "err" };
                let body = format!("[tool] {name} {status} ({ms}ms)");
                let line = if *ok {
                    self.paint(DIM_YELLOW, &body)
                } else {
                    self.paint(RED, &body)
                };
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::TurnCompleted => {
                writeln!(self.out)?;
                self.in_text = false;
            }
            // Remaining variants land in Task 5.
            _ => {}
        }
        Ok(())
    }
```

- [ ] **Step 4: Run all render tests — expect PASS**

Run: `cargo nextest run -p cogito-cli render::tests`
Expected: 4 tests pass.

- [ ] **Step 5: fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-cli`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-cli/src/render.rs
git commit -m "$(cat <<'EOF'
feat(cli): render tool-dispatch lifecycle with name + duration

`ToolDispatchStarted` prints `[tool] <name> …` and records start
time; `ToolDispatchEnded` removes the timer, prints elapsed ms and
ok/err marker. After a tool, the next text delta re-emits the
`agent: ` prefix because `in_text` is reset on every tool boundary.
EOF
)"
```

---

## Task 5: Turn lifecycle (paused / resumed / cancelled / failed)

**Files:**
- Modify: `crates/cogito-cli/src/render.rs`

- [ ] **Step 1: Add failing lifecycle tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn turn_failed_prints_reason() {
        let out = render_events(&[StreamEvent::TurnFailed {
            reason: "boom".into(),
        }]);
        assert_eq!(out, "\n[error] boom");
    }

    #[test]
    fn turn_cancelled_prints_marker() {
        let out = render_events(&[StreamEvent::TurnCancelled]);
        assert_eq!(out, "\n[cancelled]");
    }

    #[test]
    fn turn_paused_resumed_print_markers() {
        let out = render_events(&[
            StreamEvent::TurnPaused,
            StreamEvent::TurnResumed,
        ]);
        assert_eq!(out, "\n[paused]\n[resumed]");
    }
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run -p cogito-cli render::tests`
Expected: three new tests FAIL with empty output (the wildcard arm swallows them).

- [ ] **Step 3: Implement the four lifecycle variants**

In `on_stream_event`, replace the `// Remaining variants land in Task 5.` + `_ => {}` arms with:

```rust
            StreamEvent::TurnPaused => {
                let line = self.paint(DIM, "[paused]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::TurnResumed => {
                let line = self.paint(DIM, "[resumed]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::TurnCancelled => {
                let line = self.paint(DIM_YELLOW, "[cancelled]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::TurnFailed { reason } => {
                let body = format!("[error] {reason}");
                let line = self.paint(RED, &body);
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            // `StreamEvent` is `#[non_exhaustive]`: future variants render as a no-op.
            _ => {}
```

(The wildcard is retained because `StreamEvent` is marked `#[non_exhaustive]` upstream.)

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run -p cogito-cli render::tests`
Expected: 7 tests pass.

- [ ] **Step 5: fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-cli`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-cli/src/render.rs
git commit -m "$(cat <<'EOF'
feat(cli): render turn-lifecycle markers (paused/resumed/cancelled/failed)

Each lifecycle variant becomes a single labeled line: `[paused]`,
`[resumed]`, `[cancelled]`, `[error] <reason>`. `TurnFailed` carries
the human-readable reason from the upstream stream event.
EOF
)"
```

---

## Task 6: Color path + ANSI balance test

**Files:**
- Modify: `crates/cogito-cli/src/render.rs`

The color path already exists (Task 3 wrote `paint()` with the `if self.color` branch). This task adds the test that verifies ANSI sequences are well-formed.

- [ ] **Step 1: Add the balance test**

Append to `mod tests`:

```rust
    fn render_events_color(events: &[StreamEvent]) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = Renderer::new(&mut buf, true);
            for e in events {
                r.on_stream_event(e).unwrap();
            }
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn color_codes_balanced() {
        let out = render_events_color(&[
            StreamEvent::TurnStarted,
            StreamEvent::TextDelta { chunk: "hi".into() },
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
            },
            StreamEvent::TurnFailed { reason: "x".into() },
        ]);
        // Every ESC-bracketed open code must be paired with a reset (ESC[0m).
        let resets = out.matches("\x1b[0m").count();
        let opens = out.matches("\x1b[").count() - resets;
        assert_eq!(
            opens, resets,
            "unbalanced ANSI sequences (opens={opens}, resets={resets}): {out:?}"
        );
        assert!(resets > 0, "expected at least one ANSI sequence: {out:?}");
    }

    #[test]
    fn no_color_path_emits_no_escape_sequences() {
        let out = render_events(&[
            StreamEvent::TextDelta { chunk: "hi".into() },
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
            },
            StreamEvent::TurnFailed { reason: "x".into() },
        ]);
        assert!(!out.contains('\x1b'), "no_color path leaked ESC byte: {out:?}");
    }
```

- [ ] **Step 2: Run — expect PASS**

Run: `cargo nextest run -p cogito-cli render::tests`
Expected: 9 tests pass. (Both new tests should pass without changing `paint()` since the color branch was implemented in Task 3.)

- [ ] **Step 3: fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-cli`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-cli/src/render.rs
git commit -m "$(cat <<'EOF'
test(cli): verify ANSI balance and no-color escape suppression

`color_codes_balanced` asserts every ANSI open code has a matching
reset; `no_color_path_emits_no_escape_sequences` ensures piped
output (non-TTY) is plain text. Locks the IsTerminal fallback
contract.
EOF
)"
```

---

## Task 7: Wire `Renderer` into `chat.rs`

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs` (lines 191-228)

- [ ] **Step 1: Add the `render` import**

Open `crates/cogito-cli/src/chat.rs`. At the top, the existing imports include `use cogito_protocol::stream::StreamEvent;`. Just before `use cogito_store_jsonl::JsonlStore;` add:

```rust
use crate::render::Renderer;
```

- [ ] **Step 2: Replace the event-loop body**

Find the existing loop block (lines 191-228 — starting `let mut stdin = BufReader::new(io::stdin());` and ending with the closing `}` of `loop { tokio::select! { … } }`).

Replace it with:

```rust
    // `AsyncBufReadExt::lines()` rejects non-UTF-8 bytes and aborts the REPL with
    // "stream did not contain valid UTF-8" (common with GBK terminals or pasted
    // binary). Read raw bytes and decode lossily instead.
    let mut stdin = BufReader::new(io::stdin());
    let mut line_buf = Vec::new();
    let mut sub = handle.subscribe();
    let mut renderer = Renderer::for_stdout();

    renderer.prompt_user()?;

    loop {
        tokio::select! {
            read = stdin.read_until(b'\n', &mut line_buf) => match read {
                Ok(0) => break,
                Ok(_) => {
                    while matches!(line_buf.last(), Some(b'\n' | b'\r')) {
                        line_buf.pop();
                    }
                    let l = String::from_utf8_lossy(&line_buf).into_owned();
                    line_buf.clear();

                    if l.trim() == "/quit" {
                        break;
                    }
                    if l.trim().is_empty() {
                        renderer.prompt_user()?;
                        continue;
                    }
                    handle.submit_user_text(l).await.context("submit_user_text")?;
                }
                Err(e) => return Err(e).context("stdin read"),
            },
            evt = sub.recv() => match evt {
                Ok(e) => {
                    let terminal = matches!(
                        &e,
                        StreamEvent::TurnCompleted
                            | StreamEvent::TurnFailed { .. }
                            | StreamEvent::TurnCancelled
                            | StreamEvent::TurnPaused
                    );
                    renderer.on_stream_event(&e)?;
                    if terminal {
                        renderer.prompt_user()?;
                    }
                }
                // Broadcast channel lagged or closed — treat as session end.
                Err(_) => break,
            },
        }
    }
```

Key differences from the original:
- `Renderer::for_stdout()` constructed once before the loop.
- `prompt_user()` called once before the loop and after every empty line / terminal turn event.
- Every `Ok(e)` event goes through `renderer.on_stream_event(&e)`. The `Ok(StreamEvent::TextDelta { chunk })` special case is gone — the renderer owns the print.
- The trailing `Ok(_) => {}` wildcard arm is gone (renderer handles all variants, including unknown future ones via its own `_` wildcard).

- [ ] **Step 3: fmt + clippy**

Run: `make fmt && make fix CRATE=cogito-cli`
Expected: clean.

- [ ] **Step 4: Run all cogito-cli tests**

Run: `make test CRATE=cogito-cli`
Expected: all pass — both the new `render::tests` module and the existing integration tests (`config_cli_overrides`, `config_file_only`, etc.).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-cli/src/chat.rs
git commit -m "$(cat <<'EOF'
feat(cli): drive `cogito chat` REPL through render::Renderer

Replaces the inline `print!("{chunk}")` event handler with the
`StreamEvent`-driven renderer. The REPL now shows `> ` user prompts
in cyan, `agent: ` in green for each assistant text block, dim-yellow
`[tool] <name> …`/`ok|err (Nms)` markers around tool dispatches,
and dim/red markers for paused/resumed/cancelled/failed turns. When
stdout is not a TTY (`cogito chat | cat`), the renderer falls back
to plain text via `IsTerminal`.
EOF
)"
```

---

## Task 8: Manual verification

This task does not write code — it is the human / agent runtime smoke-test step required by `CLAUDE.md` for UI / frontend changes ("If you can't test the UI, say so explicitly rather than claiming success.").

**Prerequisite:** a working `cogito.toml` with at least one provider configured. If running without a real API key, you can still execute steps 2/4/5 by typing `/quit` immediately; only step 1 needs the model to respond.

- [ ] **Step 1: Compact prompt log appears once per turn**

Run: `RUST_LOG=debug make chat 2>&1 | grep "cogito::prompt"`

Type a short message (e.g. `hello`) and `/quit` after the reply.

Expected: at least one line matching `DEBUG cogito::prompt: request: {…JSON…}` per turn. Verify the JSON is **one line**, no `── request body ──` frame.

If you have both Anthropic and an OpenAI-compat provider configured, repeat with each (`make chat --provider anthropic`, `make chat --provider openai_compat` or whatever your `cogito.toml` defines).

- [ ] **Step 2: REPL colors render in a TTY**

Run: `make chat` (in a real terminal, not piped).

Type `hello` (or whatever invokes a model + tool — `请读取 src/main.rs` if `read_file` is enabled).

Expected:
- `> ` prompt is cyan.
- `agent: ` prefix on assistant text is green.
- If a tool is invoked, `[tool] <name> …` (dim yellow) and `[tool] <name> ok (Nms)` lines appear around it.
- After the turn ends, a new `> ` prompt appears on its own line.

- [ ] **Step 3: REPL falls back to plain text when piped**

Run: `make chat | cat`. Type `hello` and `/quit`.

Expected: same content as step 2 but **no ANSI escape sequences** in the output. (Pipe through `cat -A` if you want to be paranoid: no `^[[` should appear.)

- [ ] **Step 4: Ctrl-C produces `[cancelled]`**

Run: `make chat`.

Type a long prompt that will cause the model to stream for a while. Mid-stream, press Ctrl-C **once**.

Expected: a dim-yellow `[cancelled]` line appears, then a fresh `> ` prompt. The REPL keeps running — Ctrl-C does **not** kill the process (per the existing Ctrl-C handler in `chat.rs`).

- [ ] **Step 5: Full CI pass**

Run: `make ci`
Expected: green.

- [ ] **Step 6: If anything in steps 1-5 misbehaved, do NOT mark this task complete.** File a follow-up note in the spec's §8 decision log and return to the offending Task. Do not paper over a regression.

---

## Self-Review Notes

**Spec coverage:**
- §2 (Model-side) → Tasks 1 + 2 ✓
- §3.3 (`Renderer` module structure) → Task 3 ✓
- §3.4 (Event → output mapping, all 9 rows) → Tasks 3 (text/completed/started), 4 (tool start/end), 5 (paused/resumed/cancelled/failed) ✓
- §3.5 (`chat.rs` integration) → Task 7 ✓
- §3.6 (ADR-0004 layering) → enforced by `make ci`'s `layer-check`, run in Task 8 step 5 ✓
- §4.1 (Unit tests, 6 listed) → Task 3 (plain_text_sequence_no_color), Task 4 (tool_lifecycle_ok/err_no_color + text_after_tool_reprints_agent_prefix), Task 5 (turn_failed/cancelled/paused_resumed), Task 6 (color_codes_balanced + no_color_path_emits_no_escape_sequences) — 9 tests total, exceeding the 6 the spec called for ✓
- §4.2 (Manual / integration) → Task 8 ✓
- §5 (Acceptance) → Task 8 step 5 (full `make ci`) + Task 8 steps 2-3 (TTY / pipe behavior) ✓

**Placeholder scan:** no TBDs, no "add error handling", no "similar to Task N" forward references. Every code step shows the actual code.

**Type consistency:** `Renderer<W: Write>` is the type across all tasks. `on_stream_event(&self, ev: &StreamEvent) -> IoResult<()>` signature is consistent. `tool_timers: HashMap<String, (Instant, String)>` is the map type referenced in Tasks 3, 4, and the spec §3.4.

**Constants:** `CYAN`, `GREEN`, `RED`, `DIM`, `DIM_YELLOW` — all defined in Task 3 and used in subsequent tasks. No unused constants (the workspace's `-Dwarnings` policy would fail the build).

---

## Done

After Task 8 step 5 is green, the branch is shippable. Spec §5 acceptance is satisfied. Push to remote and open a PR titled along the lines of:

> `feat(cli, model): compact debug log + REPL role coloring (GitLab #2)`

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

/// Maximum characters to print from a compact tool-args JSON before
/// truncating with an ellipsis. Picked so a one-line call rarely wraps
/// in an 80-col terminal once the `[tool] <name> ` prefix is included.
const TOOL_ARGS_PREVIEW_MAX: usize = 200;

/// Maximum characters to print from a tool error message before
/// truncating. Errors are usually short tracebacks or one-liners; this
/// keeps a misbehaving tool from flooding the terminal scrollback.
const TOOL_ERROR_PREVIEW_MAX: usize = 400;

/// Truncate a UTF-8 string to at most `max` chars (not bytes), appending
/// `...` when truncation happened. Returns the original string when it
/// already fits within `max`.
fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    // Reserve 3 chars for the ellipsis.
    let keep = max.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

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
            StreamEvent::ToolDispatchStarted {
                call_id,
                tool_name,
                args,
            } => {
                self.tool_timers
                    .insert(call_id.clone(), (Instant::now(), tool_name.clone()));
                // Compact one-line JSON; falls back to `{}` when the
                // value somehow fails to serialize (shouldn't happen
                // for valid serde_json::Value but stay defensive).
                let args_preview = serde_json::to_string(args).map_or_else(
                    |_| "{}".to_string(),
                    |s| truncate_chars(&s, TOOL_ARGS_PREVIEW_MAX),
                );
                let line = self.paint(
                    DIM_YELLOW,
                    &format!("[tool] {tool_name} {args_preview} \u{2026}"),
                );
                write!(self.out, "\n{line}")?;
                self.in_text = false;
            }
            StreamEvent::ToolDispatchEnded {
                call_id,
                ok,
                error_message,
            } => {
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
                writeln!(self.out, "\n{line}")?;
                if let Some(msg) = error_message.as_deref() {
                    let truncated = truncate_chars(msg, TOOL_ERROR_PREVIEW_MAX);
                    // 8-space indent so the message visually attaches
                    // to the `[tool] … err (…)` line above without
                    // being mistaken for a new agent / tool line.
                    let indented = self.paint(RED, &format!("        {truncated}"));
                    writeln!(self.out, "{indented}")?;
                }
                self.in_text = false;
            }
            StreamEvent::TurnCompleted => {
                // Only emit a trailing newline when the previous event
                // didn't already terminate its line (i.e. mid-stream
                // text). Tool / lifecycle events use `writeln!` and
                // are self-terminating, so a second newline here would
                // produce a stray blank line.
                if self.in_text {
                    writeln!(self.out)?;
                }
                self.in_text = false;
            }
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
        }
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
#[allow(clippy::unwrap_used)]
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
            StreamEvent::TextDelta {
                chunk: " there".into(),
            },
            StreamEvent::TurnCompleted,
        ]);
        assert_eq!(out, "\nagent: hi there\n");
    }

    #[test]
    fn tool_lifecycle_ok_no_color() {
        let out = render_events(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({"path": "src/main.rs"}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
            StreamEvent::TurnCompleted,
        ]);
        assert!(
            out.starts_with(
                "\n[tool] read_file {\"path\":\"src/main.rs\"} …\n[tool] read_file ok ("
            ),
            "unexpected start: {out:?}"
        );
        assert!(out.ends_with("ms)\n"), "unexpected end: {out:?}");
    }

    #[test]
    fn tool_lifecycle_err_no_color() {
        let out = render_events(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "bad_tool".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c2".into(),
                ok: false,
                error_message: None,
            },
        ]);
        assert!(
            out.contains("[tool] bad_tool err ("),
            "expected 'err' marker, got: {out:?}"
        );
    }

    #[test]
    fn tool_args_preview_no_color() {
        let out = render_events(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "query_cameras".into(),
            args: serde_json::json!({"fuzzy_keyword": "深圳"}),
        }]);
        assert!(
            out.contains(r#"{"fuzzy_keyword":"深圳"}"#),
            "expected args JSON in output: {out:?}"
        );
        assert!(
            out.contains("[tool] query_cameras"),
            "expected tool name prefix: {out:?}"
        );
    }

    #[test]
    fn tool_args_truncated_when_long() {
        // Build args that, when serialized, exceed TOOL_ARGS_PREVIEW_MAX.
        let long: String = "x".repeat(500);
        let out = render_events(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: serde_json::json!({"blob": long}),
        }]);
        // The output line is `\n[tool] t <preview> …`. Locate the
        // preview substring between the tool name and the trailing
        // ellipsis (U+2026 \u{2026}) and assert it ends with `...`
        // and is within the budget.
        let line = out.lines().find(|l| l.contains("[tool] t")).unwrap_or("");
        let preview = line
            .strip_prefix("[tool] t ")
            .and_then(|s| s.strip_suffix(" \u{2026}"))
            .unwrap_or(line);
        assert!(
            preview.ends_with("..."),
            "expected truncation marker, got preview: {preview:?}"
        );
        assert!(
            preview.chars().count() <= TOOL_ARGS_PREVIEW_MAX,
            "preview exceeded TOOL_ARGS_PREVIEW_MAX: {preview:?}"
        );
    }

    #[test]
    fn tool_error_message_indented_after_err_line() {
        let out = render_events(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some("boom".into()),
            },
        ]);
        assert!(
            out.contains("err ("),
            "expected err marker before message: {out:?}"
        );
        assert!(
            out.contains("\n        boom\n"),
            "expected 8-space-indented error message line: {out:?}"
        );
    }

    #[test]
    fn tool_error_message_truncated_when_long() {
        let long: String = "x".repeat(800);
        let out = render_events(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some(long),
            },
        ]);
        let indented_line = out
            .lines()
            .find(|l| l.starts_with("        x"))
            .unwrap_or("");
        let msg = indented_line.trim_start();
        assert!(
            msg.ends_with("..."),
            "expected truncation marker on error message: {msg:?}"
        );
        assert!(
            msg.chars().count() <= TOOL_ERROR_PREVIEW_MAX,
            "error message exceeded TOOL_ERROR_PREVIEW_MAX: {msg:?}"
        );
    }

    #[test]
    fn tool_ok_does_not_print_error_line() {
        let out = render_events(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert!(
            !out.contains("\n        "),
            "ok path should not emit an indented error line: {out:?}"
        );
    }

    #[test]
    fn text_after_tool_reprints_agent_prefix() {
        let out = render_events(&[
            StreamEvent::TextDelta { chunk: "a".into() },
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
            StreamEvent::TextDelta { chunk: "b".into() },
        ]);
        let count = out.matches("agent: ").count();
        assert_eq!(count, 2, "expected two `agent: ` prefixes, got: {out:?}");
    }

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
        let out = render_events(&[StreamEvent::TurnPaused, StreamEvent::TurnResumed]);
        assert_eq!(out, "\n[paused]\n[resumed]");
    }

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
                args: serde_json::json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some("oops".into()),
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
                args: serde_json::json!({}),
            },
            StreamEvent::TurnFailed { reason: "x".into() },
        ]);
        assert!(
            !out.contains('\x1b'),
            "no_color path leaked ESC byte: {out:?}"
        );
    }
}

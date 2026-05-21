//! ANSI-colored REPL renderer for `cogito chat`.
//!
//! Translates `StreamEvent`s into role-tagged stdout output. TTY
//! detection (`std::io::IsTerminal`) degrades the color path to plain
//! text when stdout is not a terminal (e.g. piped to `cat` or
//! redirected to a file).
//!
//! See `docs/superpowers/specs/2026-05-21-dev-experience-cli-display-design.md`.

// SCAFFOLDING: this entire module is built incrementally across Tasks 3-7
// of the dev-experience plan. Removed in Task 7 when `chat.rs` wires up
// `Renderer::for_stdout()`. By that point every constant and method below
// has at least one real call site outside of `#[cfg(test)]`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{IsTerminal, Result as IoResult, Write};
use std::time::Instant;

use cogito_protocol::stream::StreamEvent;

// Color/style codes used by `paint`; some are referenced only in later tasks.
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
                let line = self.paint(DIM_YELLOW, &format!("[tool] {tool_name} \u{2026}"));
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
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
            },
            StreamEvent::TurnCompleted,
        ]);
        assert!(
            out.starts_with("\n[tool] read_file …\n[tool] read_file ok ("),
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
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c2".into(),
                ok: false,
            },
        ]);
        assert!(
            out.contains("[tool] bad_tool err ("),
            "expected 'err' marker, got: {out:?}"
        );
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
        assert!(
            !out.contains('\x1b'),
            "no_color path leaked ESC byte: {out:?}"
        );
    }
}

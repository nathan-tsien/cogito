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
            StreamEvent::TurnCompleted => {
                writeln!(self.out)?;
                self.in_text = false;
            }
            // Other variants land in subsequent tasks.
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
}

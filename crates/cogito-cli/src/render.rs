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
    in_thinking: bool,
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
            in_thinking: false,
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
            StreamEvent::TurnStarted { .. } => {
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::ThinkingDelta { chunk } => {
                if !self.in_thinking {
                    // If a thinking block starts after text/tools, the
                    // previous line is already terminated by our helpers,
                    // so `\nthinking: ` lays cleanly. Reset in_text first
                    // so a subsequent TextDelta repaints `agent:`.
                    self.in_text = false;
                    self.write_thinking_label()?;
                    self.in_thinking = true;
                }
                let painted = self.paint(DIM, chunk);
                write!(self.out, "{painted}")?;
                self.out.flush()?;
            }
            StreamEvent::TextDelta { chunk, .. } => {
                if !self.in_text {
                    self.write_agent_label()?;
                    self.in_text = true;
                }
                self.in_thinking = false;
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
                self.write_tool_start_line(tool_name, args)?;
                self.in_text = false;
                self.in_thinking = false;
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
                self.write_tool_end_line(&name, *ok, ms, error_message.as_deref())?;
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnCompleted { .. } => {
                // Only emit a trailing newline when the previous event
                // didn't already terminate its line (i.e. mid-stream
                // text or thinking). Tool / lifecycle events use
                // `writeln!` and are self-terminating, so a second
                // newline here would produce a stray blank line.
                if self.in_text || self.in_thinking {
                    writeln!(self.out)?;
                }
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnPaused => {
                let line = self.paint(DIM, "[paused]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnResumed => {
                let line = self.paint(DIM, "[resumed]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnCancelled => {
                let line = self.paint(DIM_YELLOW, "[cancelled]");
                write!(self.out, "\n{line}")?;
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnFailed { reason, .. } => {
                self.write_error_line(reason)?;
                self.in_text = false;
                self.in_thinking = false;
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

    // -- Shared block formatters --------------------------------------
    //
    // Each helper writes one visual block (agent label, tool start,
    // tool end + optional error indent, `[error] …` line). Both the
    // live `on_stream_event` path and the `replay_*` API call these so
    // the formatting of tool args, elapsed-ms rendering, and error
    // truncation lives in exactly one place. Helpers only deal with
    // writing — they do NOT touch `self.in_text` or `tool_timers`; the
    // caller decides those based on whether it's live or replay.

    /// `\n` + green `agent: ` label. The caller writes the actual
    /// text content immediately after.
    fn write_agent_label(&mut self) -> IoResult<()> {
        let label = self.paint(GREEN, "agent: ");
        write!(self.out, "\n{label}")
    }

    /// `\n` + dim `thinking: ` label. The caller writes the reasoning
    /// content immediately after (already dim-painted so the whole
    /// block reads as muted text). See ADR-0019 §3 for the broadcast
    /// flow this surfaces.
    fn write_thinking_label(&mut self) -> IoResult<()> {
        let label = self.paint(DIM, "thinking: ");
        write!(self.out, "\n{label}")
    }

    /// `\n` + dim-yellow `[tool] <name> <args-preview> …` line, no
    /// trailing newline so the next event (typically the matching
    /// end line) can lay its own `\n`-prefixed line directly below.
    fn write_tool_start_line(&mut self, tool_name: &str, args: &serde_json::Value) -> IoResult<()> {
        // Compact one-line JSON; falls back to `{}` when the value
        // somehow fails to serialize (shouldn't happen for a valid
        // `serde_json::Value`, but stay defensive).
        let args_preview = serde_json::to_string(args).map_or_else(
            |_| "{}".to_string(),
            |s| truncate_chars(&s, TOOL_ARGS_PREVIEW_MAX),
        );
        let line = self.paint(
            DIM_YELLOW,
            &format!("[tool] {tool_name} {args_preview} \u{2026}"),
        );
        write!(self.out, "\n{line}")
    }

    /// `\n` + colored `[tool] <name> ok|err (<ms>ms)` line, then —
    /// when `error_message` is present — an 8-space-indented red
    /// message line. Both lines are `writeln!`-terminated so the next
    /// stdout writer (tracing log, REPL prompt) starts on a fresh
    /// line; see spec §9.2 for the rationale.
    fn write_tool_end_line(
        &mut self,
        tool_name: &str,
        ok: bool,
        elapsed_ms: u128,
        error_message: Option<&str>,
    ) -> IoResult<()> {
        let status = if ok { "ok" } else { "err" };
        let body = format!("[tool] {tool_name} {status} ({elapsed_ms}ms)");
        let line = if ok {
            self.paint(DIM_YELLOW, &body)
        } else {
            self.paint(RED, &body)
        };
        writeln!(self.out, "\n{line}")?;
        if let Some(msg) = error_message {
            let truncated = truncate_chars(msg, TOOL_ERROR_PREVIEW_MAX);
            // 8-space indent so the message visually attaches to the
            // `[tool] … err (…)` line above without being mistaken
            // for a new agent / tool line.
            let indented = self.paint(RED, &format!("        {truncated}"));
            writeln!(self.out, "{indented}")?;
        }
        Ok(())
    }

    /// `\n` + red `[error] <reason>` line. No trailing newline; the
    /// caller appends one when the line must stand alone (replay),
    /// or leaves it implicit when something else follows (live →
    /// REPL prompt).
    fn write_error_line(&mut self, reason: &str) -> IoResult<()> {
        let line = self.paint(RED, &format!("[error] {reason}"));
        write!(self.out, "\n{line}")
    }

    // -- Replay API ---------------------------------------------------
    //
    // Thin wrappers used by `chat::replay_history` to re-render the
    // persisted `ConversationEvent` stream before the first live
    // prompt. They differ from the live path only in that they're
    // driven directly by stored data (caller supplies `elapsed_ms`)
    // and each helper terminates with a newline so successive replay
    // lines don't glue together.

    /// Print a dim, single-line replay banner (e.g. session
    /// metadata). Used at the start of a resumed REPL to anchor the
    /// history that follows.
    pub fn replay_banner(&mut self, text: &str) -> IoResult<()> {
        let line = self.paint(DIM, text);
        writeln!(self.out, "{line}")?;
        self.in_text = false;
        Ok(())
    }

    /// Print a prior user input as it would have appeared at the
    /// prompt during the original session.
    pub fn replay_user_input(&mut self, text: &str) -> IoResult<()> {
        let prompt = self.paint(CYAN, "> ");
        writeln!(self.out, "\n{prompt}{text}")?;
        self.in_text = false;
        Ok(())
    }

    /// Print a prior assistant text block in one shot. Unlike live
    /// `TextDelta` rendering this is not concatenation-aware — each
    /// call produces its own `agent: …` line.
    pub fn replay_assistant_block(&mut self, text: &str) -> IoResult<()> {
        self.write_agent_label()?;
        writeln!(self.out, "{text}")?;
        self.in_text = false;
        self.in_thinking = false;
        Ok(())
    }

    /// Print a prior thinking block in one shot. Renders as a dim
    /// `thinking: <text>` line; empty text (e.g. Anthropic
    /// `redacted_thinking`) is shown as `[redacted]` so the block
    /// stays visible in history. Per ADR-0019 §2.
    pub fn replay_thinking_block(&mut self, text: &str) -> IoResult<()> {
        self.write_thinking_label()?;
        let body = if text.is_empty() { "[redacted]" } else { text };
        let painted = self.paint(DIM, body);
        writeln!(self.out, "{painted}")?;
        self.in_text = false;
        self.in_thinking = false;
        Ok(())
    }

    /// Print a prior tool call as a single start/end pair, sharing
    /// formatting with the live `ToolDispatch*` arms. `elapsed_ms`
    /// comes from the event log's persisted timestamps, so the
    /// rendered duration matches the original wall-clock — not the
    /// time the replay itself took.
    pub fn replay_tool_call(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
        elapsed_ms: u128,
        ok: bool,
        error_message: Option<&str>,
    ) -> IoResult<()> {
        self.write_tool_start_line(tool_name, args)?;
        self.write_tool_end_line(tool_name, ok, elapsed_ms, error_message)?;
        self.in_text = false;
        Ok(())
    }

    /// Print a prior turn failure during replay. Adds the trailing
    /// newline that the live `TurnFailed` arm leaves to the REPL
    /// prompt.
    pub fn replay_turn_failed(&mut self, reason: &str) -> IoResult<()> {
        self.write_error_line(reason)?;
        writeln!(self.out)?;
        self.in_text = false;
        Ok(())
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
            StreamEvent::TurnStarted {
                subagent_call_id: None,
            },
            StreamEvent::TextDelta {
                chunk: "hi".into(),
                subagent_call_id: None,
            },
            StreamEvent::TextDelta {
                chunk: " there".into(),
                subagent_call_id: None,
            },
            StreamEvent::TurnCompleted {
                stop_reason: None,
                subagent_call_id: None,
            },
        ]);
        assert_eq!(out, "\nagent: hi there\n");
    }

    #[test]
    fn tool_lifecycle_ok_no_color() {
        let out = render_events(&[
            StreamEvent::TurnStarted {
                subagent_call_id: None,
            },
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
            StreamEvent::TurnCompleted {
                stop_reason: None,
                subagent_call_id: None,
            },
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
            StreamEvent::TextDelta {
                chunk: "a".into(),
                subagent_call_id: None,
            },
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
            StreamEvent::TextDelta {
                chunk: "b".into(),
                subagent_call_id: None,
            },
        ]);
        let count = out.matches("agent: ").count();
        assert_eq!(count, 2, "expected two `agent: ` prefixes, got: {out:?}");
    }

    #[test]
    fn turn_failed_prints_reason() {
        let out = render_events(&[StreamEvent::TurnFailed {
            reason: "boom".into(),
            subagent_call_id: None,
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
            StreamEvent::TurnStarted {
                subagent_call_id: None,
            },
            StreamEvent::TextDelta {
                chunk: "hi".into(),
                subagent_call_id: None,
            },
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
            StreamEvent::TurnFailed {
                reason: "x".into(),
                subagent_call_id: None,
            },
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
            StreamEvent::TextDelta {
                chunk: "hi".into(),
                subagent_call_id: None,
            },
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            },
            StreamEvent::TurnFailed {
                reason: "x".into(),
                subagent_call_id: None,
            },
        ]);
        assert!(
            !out.contains('\x1b'),
            "no_color path leaked ESC byte: {out:?}"
        );
    }

    fn render<F: FnOnce(&mut Renderer<&mut Vec<u8>>) -> IoResult<()>>(f: F) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = Renderer::new(&mut buf, false);
            f(&mut r).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn replay_banner_writes_single_line() {
        let out = render(|r| r.replay_banner("resumed session abc · 4 turns"));
        assert_eq!(out, "resumed session abc · 4 turns\n");
    }

    #[test]
    fn replay_user_input_prints_prompt_then_text() {
        let out = render(|r| r.replay_user_input("who are you?"));
        assert_eq!(out, "\n> who are you?\n");
    }

    #[test]
    fn replay_assistant_block_prints_full_text_with_prefix() {
        let out = render(|r| r.replay_assistant_block("I am cogito."));
        assert_eq!(out, "\nagent: I am cogito.\n");
    }

    #[test]
    fn replay_tool_call_ok_shows_args_and_duration() {
        let out = render(|r| {
            r.replay_tool_call(
                "read_file",
                &serde_json::json!({"path": "a.rs"}),
                42,
                true,
                None,
            )
        });
        assert!(
            out.contains(r#"[tool] read_file {"path":"a.rs"} …"#),
            "missing start line: {out:?}"
        );
        assert!(
            out.contains("[tool] read_file ok (42ms)"),
            "missing end line with duration: {out:?}"
        );
        assert!(
            !out.contains("\n        "),
            "ok path should not emit an indented error line: {out:?}"
        );
    }

    #[test]
    fn replay_tool_call_err_shows_indented_message() {
        let out = render(|r| {
            r.replay_tool_call(
                "query_cameras",
                &serde_json::json!({"fuzzy_keyword": "深圳"}),
                7,
                false,
                Some("backend offline"),
            )
        });
        assert!(
            out.contains("[tool] query_cameras err (7ms)"),
            "missing err marker: {out:?}"
        );
        assert!(
            out.contains("\n        backend offline\n"),
            "missing indented error message: {out:?}"
        );
    }

    #[test]
    fn replay_turn_failed_prints_error_line() {
        let out = render(|r| r.replay_turn_failed("model gateway timeout"));
        assert_eq!(out, "\n[error] model gateway timeout\n");
    }

    #[test]
    fn thinking_delta_sequence_no_color() {
        let out = render_events(&[
            StreamEvent::TurnStarted {
                subagent_call_id: None,
            },
            StreamEvent::ThinkingDelta {
                chunk: "I should ".into(),
            },
            StreamEvent::ThinkingDelta {
                chunk: "grep.".into(),
            },
            StreamEvent::TurnCompleted {
                stop_reason: None,
                subagent_call_id: None,
            },
        ]);
        assert_eq!(out, "\nthinking: I should grep.\n");
    }

    #[test]
    fn thinking_then_text_emits_separate_labels() {
        let out = render_events(&[
            StreamEvent::TurnStarted {
                subagent_call_id: None,
            },
            StreamEvent::ThinkingDelta {
                chunk: "I should grep.".into(),
            },
            StreamEvent::TextDelta {
                chunk: "Looking now.".into(),
                subagent_call_id: None,
            },
            StreamEvent::TurnCompleted {
                stop_reason: None,
                subagent_call_id: None,
            },
        ]);
        assert_eq!(out, "\nthinking: I should grep.\nagent: Looking now.\n");
    }

    #[test]
    fn thinking_chunks_are_dim_painted_with_color() {
        let out = render_events_color(&[StreamEvent::ThinkingDelta {
            chunk: "reasoning".into(),
        }]);
        // DIM = "\x1b[2m"; each chunk wraps in DIM ... reset.
        assert!(
            out.contains("\x1b[2mthinking: \x1b[0m"),
            "expected dim thinking label: {out:?}"
        );
        assert!(
            out.contains("\x1b[2mreasoning\x1b[0m"),
            "expected dim-painted chunk: {out:?}"
        );
    }

    #[test]
    fn replay_thinking_block_prints_full_text_with_prefix() {
        let out = render(|r| r.replay_thinking_block("I should grep for the symbol."));
        assert_eq!(out, "\nthinking: I should grep for the symbol.\n");
    }

    #[test]
    fn replay_thinking_block_empty_text_shows_redacted_marker() {
        let out = render(|r| r.replay_thinking_block(""));
        assert_eq!(out, "\nthinking: [redacted]\n");
    }

    #[test]
    fn thinking_then_tool_dispatch_terminates_thinking_line() {
        // A turn that thinks, calls a tool, then resumes text. The
        // thinking line must be terminated by the tool start line's
        // leading `\n`; the second thinking transition into text must
        // emit a fresh `agent:` label.
        let out = render_events(&[
            StreamEvent::ThinkingDelta {
                chunk: "let me check".into(),
            },
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
            StreamEvent::TextDelta {
                chunk: "done".into(),
                subagent_call_id: None,
            },
            StreamEvent::TurnCompleted {
                stop_reason: None,
                subagent_call_id: None,
            },
        ]);
        assert!(
            out.starts_with("\nthinking: let me check\n[tool] t "),
            "expected thinking → tool sequence: {out:?}"
        );
        assert!(
            out.contains("\nagent: done\n"),
            "expected agent label after tool: {out:?}"
        );
    }
}

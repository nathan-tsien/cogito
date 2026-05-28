//! Sink-agnostic state models for the chat pane and tool-tree pane.
//!
//! These types translate the `StreamEvent` broadcast into structural
//! state. They never touch `Write`, `Frame`, or any ratatui type —
//! the UI widgets in `crate::ui::*` consume the models at render time
//! and apply palette/layout there. This separation is the spec's
//! "Q2-A locked: new ratatui-native translation; CLI Renderer untouched".

use std::collections::HashMap;
use std::time::Instant;

use cogito_protocol::stream::StreamEvent;

/// One visible line (or coalesced block) in the chat scrollback.
///
/// Stored as structural enum with raw text — the chat widget paints
/// palette lazily at render time (spec §3 detail-level decision:
/// "lazy palette painting").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLine {
    /// User prompt, rendered as `> {text}` with the user palette.
    UserPrompt(String),
    /// Assistant text block, accumulates across `TextDelta` within
    /// one content block.
    AssistantText(String),
    /// Assistant reasoning block, accumulates across `ThinkingDelta`.
    AssistantThinking(String),
    /// Tool dispatch start, paired with a `ToolEndLine` later.
    ToolStartLine {
        /// Tool name as reported by the dispatcher.
        tool: String,
        /// Compact JSON args preview, truncated.
        args_preview: String,
    },
    /// Tool dispatch end.
    ToolEndLine {
        /// Tool name (same as the matching start line).
        tool: String,
        /// `true` if the tool returned successfully.
        ok: bool,
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
        /// Error message captured on failure; `None` on success.
        error: Option<String>,
    },
    /// System-emitted notice line — `[paused]`, `[cancelled]`,
    /// `[error] ...`, MCP banner, slash command echoes.
    SystemNotice(String),
}

/// Maximum chars to preview from a tool's args JSON. Matches the CLI
/// `TOOL_ARGS_PREVIEW_MAX` to keep `[tool] foo {...}` lines bounded.
pub const TOOL_ARGS_PREVIEW_MAX: usize = 200;

/// Maximum chars to preview from a tool's error message.
pub const TOOL_ERROR_PREVIEW_MAX: usize = 400;

/// Truncate a string to at most `max` Unicode chars (not bytes),
/// appending `...` when truncated.
#[must_use]
pub fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// Per-tool dispatch timer, keyed by `call_id`. Resolved into
/// `ToolEndLine::elapsed_ms` when the matching end arrives.
type ToolTimers = HashMap<String, (Instant, String)>;

/// Chat scrollback state. Pure function of the `StreamEvent` stream.
#[derive(Debug, Default)]
pub struct ChatModel {
    /// Visible lines in display order.
    pub lines: Vec<ChatLine>,
    /// Currently in the middle of an `AssistantText` block — used to
    /// coalesce successive `TextDelta`s into one line.
    pub in_text: bool,
    /// Same for `AssistantThinking`.
    pub in_thinking: bool,
    /// Vertical scroll offset from the bottom (0 = follow tail).
    pub scroll_offset: u16,
    /// `call_id` → (`started_at`, `tool_name`) for elapsed-ms tracking.
    tool_timers: ToolTimers,
}

impl ChatModel {
    /// Construct a fresh, empty chat model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a user prompt line. Called by the input → send path, not
    /// by `on_event` (the prompt isn't a `StreamEvent`).
    pub fn push_user_prompt(&mut self, text: String) {
        self.lines.push(ChatLine::UserPrompt(text));
        self.in_text = false;
        self.in_thinking = false;
    }

    /// Push a `SystemNotice`. Used for MCP banner lines, slash echoes,
    /// `[hint]` lines.
    pub fn push_notice(&mut self, msg: impl Into<String>) {
        self.lines.push(ChatLine::SystemNotice(msg.into()));
        self.in_text = false;
        self.in_thinking = false;
    }

    /// Apply one `StreamEvent`. Pure state transition; never draws.
    pub fn on_event(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TurnStarted | StreamEvent::TurnCompleted => {
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TextDelta { chunk } => {
                if self.in_text {
                    if let Some(ChatLine::AssistantText(s)) = self.lines.last_mut() {
                        s.push_str(chunk);
                    }
                } else {
                    self.lines.push(ChatLine::AssistantText(chunk.clone()));
                    self.in_text = true;
                }
                self.in_thinking = false;
            }
            StreamEvent::ThinkingDelta { chunk } => {
                if self.in_thinking {
                    if let Some(ChatLine::AssistantThinking(s)) = self.lines.last_mut() {
                        s.push_str(chunk);
                    }
                } else {
                    self.lines.push(ChatLine::AssistantThinking(chunk.clone()));
                    self.in_thinking = true;
                }
                self.in_text = false;
            }
            StreamEvent::ToolDispatchStarted {
                call_id,
                tool_name,
                args,
            } => {
                self.tool_timers
                    .insert(call_id.clone(), (Instant::now(), tool_name.clone()));
                let args_preview = serde_json::to_string(args).map_or_else(
                    |_| "{}".to_string(),
                    |s| truncate_chars(&s, TOOL_ARGS_PREVIEW_MAX),
                );
                self.lines.push(ChatLine::ToolStartLine {
                    tool: tool_name.clone(),
                    args_preview,
                });
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::ToolDispatchEnded {
                call_id,
                ok,
                error_message,
            } => {
                let (name, ms) = self.tool_timers.remove(call_id).map_or_else(
                    || ("?".to_string(), 0_u128),
                    |(started, name)| (name, started.elapsed().as_millis()),
                );
                self.lines.push(ChatLine::ToolEndLine {
                    tool: name,
                    ok: *ok,
                    elapsed_ms: ms,
                    error: error_message
                        .as_ref()
                        .map(|m| truncate_chars(m, TOOL_ERROR_PREVIEW_MAX)),
                });
                self.in_text = false;
                self.in_thinking = false;
            }
            StreamEvent::TurnPaused => self.push_notice("[paused]"),
            StreamEvent::TurnResumed => self.push_notice("[resumed]"),
            StreamEvent::TurnCancelled => self.push_notice("[cancelled]"),
            StreamEvent::TurnFailed { reason } => self.push_notice(format!("[error] {reason}")),
            // StreamEvent is #[non_exhaustive]; future variants render
            // as no-ops until a renderer is taught about them.
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[StreamEvent]) -> ChatModel {
        let mut m = ChatModel::new();
        for e in events {
            m.on_event(e);
        }
        m
    }

    #[test]
    fn text_delta_coalesces_within_block() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::TextDelta { chunk: "hi ".into() },
            StreamEvent::TextDelta {
                chunk: "there".into(),
            },
            StreamEvent::TurnCompleted,
        ]);
        assert_eq!(m.lines, vec![ChatLine::AssistantText("hi there".into())]);
    }

    #[test]
    fn thinking_delta_coalesces_within_block() {
        let m = run(&[
            StreamEvent::ThinkingDelta {
                chunk: "let me ".into(),
            },
            StreamEvent::ThinkingDelta {
                chunk: "check".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![ChatLine::AssistantThinking("let me check".into())]
        );
    }

    #[test]
    fn thinking_then_text_emits_two_lines() {
        let m = run(&[
            StreamEvent::ThinkingDelta {
                chunk: "thinking".into(),
            },
            StreamEvent::TextDelta {
                chunk: "answer".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![
                ChatLine::AssistantThinking("thinking".into()),
                ChatLine::AssistantText("answer".into()),
            ]
        );
    }

    #[test]
    fn tool_dispatch_emits_start_and_end_lines() {
        let m = run(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: json!({"path": "a.rs"}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert!(matches!(m.lines[0], ChatLine::ToolStartLine { .. }));
        assert!(matches!(m.lines[1], ChatLine::ToolEndLine { ok: true, .. }));
    }

    #[test]
    fn tool_args_preview_is_compact_json() {
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "q".into(),
            args: json!({"fuzzy_keyword": "深圳"}),
        }]);
        match &m.lines[0] {
            ChatLine::ToolStartLine { args_preview, .. } => {
                assert!(args_preview.contains("深圳"));
                assert!(args_preview.contains("fuzzy_keyword"));
            }
            other => unreachable!("expected ToolStartLine, got {other:?}"),
        }
    }

    #[test]
    fn tool_args_preview_truncates_at_limit() {
        let blob: String = "x".repeat(500);
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({"blob": blob}),
        }]);
        match &m.lines[0] {
            ChatLine::ToolStartLine { args_preview, .. } => {
                assert!(args_preview.ends_with("..."));
                assert!(args_preview.chars().count() <= TOOL_ARGS_PREVIEW_MAX);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn tool_error_message_truncates() {
        let long: String = "x".repeat(800);
        let m = run(&[
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some(long),
            },
        ]);
        match &m.lines[1] {
            ChatLine::ToolEndLine {
                ok: false,
                error: Some(msg),
                ..
            } => {
                assert!(msg.ends_with("..."));
                assert!(msg.chars().count() <= TOOL_ERROR_PREVIEW_MAX);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn turn_paused_resumed_cancelled_failed_emit_notices() {
        let m = run(&[
            StreamEvent::TurnPaused,
            StreamEvent::TurnResumed,
            StreamEvent::TurnCancelled,
            StreamEvent::TurnFailed {
                reason: "boom".into(),
            },
        ]);
        assert_eq!(
            m.lines,
            vec![
                ChatLine::SystemNotice("[paused]".into()),
                ChatLine::SystemNotice("[resumed]".into()),
                ChatLine::SystemNotice("[cancelled]".into()),
                ChatLine::SystemNotice("[error] boom".into()),
            ]
        );
    }

    #[test]
    fn user_prompt_breaks_text_coalescing() {
        let mut m = ChatModel::new();
        m.on_event(&StreamEvent::TextDelta { chunk: "a".into() });
        m.push_user_prompt("hi".into());
        m.on_event(&StreamEvent::TextDelta { chunk: "b".into() });
        assert_eq!(
            m.lines,
            vec![
                ChatLine::AssistantText("a".into()),
                ChatLine::UserPrompt("hi".into()),
                ChatLine::AssistantText("b".into()),
            ]
        );
    }
}

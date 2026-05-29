//! Sink-agnostic state models for the chat pane and tool-tree pane.
//!
//! These types translate the `StreamEvent` broadcast into structural
//! state. They never touch `Write`, `Frame`, or any ratatui type —
//! the UI widgets in `crate::ui::*` consume the models at render time
//! and apply palette/layout there. This separation is the spec's
//! "Q2-A locked: new ratatui-native translation; CLI Renderer untouched".

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
    /// Inline tool block. Renderer looks up current state in
    /// `ToolTreeModel` via `call_id` at render time.
    ToolBlock {
        /// `call_id` from the dispatcher; matches `StreamEvent` IDs and
        /// `ToolNode.call_id`.
        call_id: String,
    },
    /// System-emitted notice line — `[paused]`, `[cancelled]`,
    /// `[error] ...`, MCP banner, slash command echoes.
    SystemNotice(String),
    /// Startup banner accent line (sigil / wordmark / tagline). Painted
    /// in the cogito accent color, bold, by the chat widget. Produced
    /// only via `push_banner`, never from a `StreamEvent`.
    Banner(String),
}

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
    /// Lines scrolled up from the bottom. `0` follows the tail (newest
    /// content visible); larger values reveal older history. The renderer
    /// clamps this to the maximum meaningful value each frame.
    pub scroll_back: u16,
}

impl ChatModel {
    /// Construct a fresh, empty chat model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scroll up (toward older history) by `n` lines. The renderer
    /// clamps the result to the available history each frame.
    pub fn scroll_up(&mut self, n: u16) {
        self.scroll_back = self.scroll_back.saturating_add(n);
    }

    /// Scroll down (toward the newest content) by `n` lines.
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll_back = self.scroll_back.saturating_sub(n);
    }

    /// Jump back to the tail (newest content), resuming follow-tail.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_back = 0;
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

    /// Push a `Banner` accent line (startup sigil / wordmark / tagline).
    pub fn push_banner(&mut self, text: impl Into<String>) {
        self.lines.push(ChatLine::Banner(text.into()));
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
            StreamEvent::ToolDispatchStarted { call_id, .. } => {
                self.lines.push(ChatLine::ToolBlock {
                    call_id: call_id.clone(),
                });
                self.in_text = false;
                self.in_thinking = false;
            }
            // Deliberate no-op for ChatModel: tool state lives in
            // ToolTreeModel. Kept as an explicit arm (rather than folding
            // into `_`) to document where ToolDispatchEnded is handled.
            #[allow(clippy::match_same_arms)]
            StreamEvent::ToolDispatchEnded { .. } => {}
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

// -- Tool-tree model -----------------------------------------------------

/// Status of one tool dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool started; awaiting `ToolDispatchEnded`.
    Running,
    /// Tool ended successfully. `elapsed_ms` captured at end time.
    Ok {
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
    },
    /// Tool ended with failure.
    Err {
        /// Wall-clock duration from start to end.
        elapsed_ms: u128,
        /// Truncated error message.
        message: String,
    },
}

impl ToolStatus {
    /// `true` once the dispatch has terminated (Ok or Err).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !matches!(self, ToolStatus::Running)
    }
}

/// One tool dispatch node in the tool-tree pane.
#[derive(Debug, Clone)]
pub struct ToolNode {
    /// `call_id` from the dispatcher; matches `StreamEvent` IDs.
    pub call_id: String,
    /// Tool name (as the dispatcher reports it).
    pub tool_name: String,
    /// Args JSON as the dispatcher received them.
    pub args: serde_json::Value,
    /// Wall-clock start time.
    pub started_at: Instant,
    /// Current status — `Running`, `Ok`, or `Err`.
    pub status: ToolStatus,
    /// Result text, populated lazily on first Ctrl-Enter expansion
    /// (spec §5.3, decision α.1). `None` until then.
    pub result_preview: Option<String>,
}

/// One turn's worth of tool calls, grouped for the tree pane.
#[derive(Debug, Clone)]
pub struct TurnGroup {
    /// 1-based turn index (incremented on `TurnStarted`).
    pub turn_idx: u32,
    /// Tool calls dispatched within this turn, in arrival order.
    pub nodes: Vec<ToolNode>,
}

/// Tree-pane state. Pure function of the `StreamEvent` stream.
#[derive(Debug, Default)]
pub struct ToolTreeModel {
    /// Turn groups in arrival order.
    pub turns: Vec<TurnGroup>,
    /// Next turn index to assign on `TurnStarted`. Starts at 1.
    next_turn_idx: u32,
}

impl ToolTreeModel {
    /// Fresh, empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self {
            turns: Vec::new(),
            next_turn_idx: 1,
        }
    }

    /// Apply one `StreamEvent`. No-op for events that don't bear on
    /// the tool-tree (`TextDelta`, `ThinkingDelta`, etc.).
    pub fn on_event(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TurnStarted => {
                self.turns.push(TurnGroup {
                    turn_idx: self.next_turn_idx,
                    nodes: Vec::new(),
                });
                self.next_turn_idx += 1;
            }
            StreamEvent::ToolDispatchStarted {
                call_id,
                tool_name,
                args,
            } => {
                // Defensive: if a tool starts before any TurnStarted
                // (shouldn't happen post-Sprint 2, but tolerate), open
                // turn 1 implicitly.
                if self.turns.is_empty() {
                    self.turns.push(TurnGroup {
                        turn_idx: self.next_turn_idx,
                        nodes: Vec::new(),
                    });
                    self.next_turn_idx += 1;
                }
                if let Some(group) = self.turns.last_mut() {
                    group.nodes.push(ToolNode {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                        args: args.clone(),
                        started_at: Instant::now(),
                        status: ToolStatus::Running,
                        result_preview: None,
                    });
                }
            }
            StreamEvent::ToolDispatchEnded {
                call_id,
                ok,
                error_message,
            } => {
                if let Some(node) = self.find_node_mut(call_id) {
                    let elapsed_ms = node.started_at.elapsed().as_millis();
                    node.status = if *ok {
                        ToolStatus::Ok { elapsed_ms }
                    } else {
                        ToolStatus::Err {
                            elapsed_ms,
                            message: error_message
                                .as_ref()
                                .map(|m| truncate_chars(m, TOOL_ERROR_PREVIEW_MAX))
                                .unwrap_or_default(),
                        }
                    };
                }
            }
            _ => {}
        }
    }

    /// Find a node by `call_id`, scanning newest turn first.
    pub fn find_node_mut(&mut self, call_id: &str) -> Option<&mut ToolNode> {
        for group in self.turns.iter_mut().rev() {
            for node in &mut group.nodes {
                if node.call_id == call_id {
                    return Some(node);
                }
            }
        }
        None
    }

    /// Total tool nodes across all turns. For tests / status hints.
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.turns.iter().map(|g| g.nodes.len()).sum()
    }
}

/// Selection cursor in the tool tree pane: `(turn_idx_in_vec,
/// node_idx_in_turn)`. Used by `Ctrl-Up/Down` navigation and
/// `Ctrl-Enter` expansion. `turn_idx_in_vec` is the position in
/// `ToolTreeModel.turns`, NOT the 1-based `TurnGroup.turn_idx`.
pub type TreePath = (usize, usize);

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
            StreamEvent::TextDelta {
                chunk: "hi ".into(),
            },
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
    fn tool_dispatch_emits_single_tool_block() {
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
        assert_eq!(m.lines.len(), 1);
        assert!(matches!(
            &m.lines[0],
            ChatLine::ToolBlock { call_id } if call_id == "c1"
        ));
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
    fn push_banner_appends_banner_line() {
        let mut m = ChatModel::new();
        m.push_banner("   \u{2234}\u{2234}\u{2234}  cogito");
        assert_eq!(
            m.lines,
            vec![ChatLine::Banner(
                "   \u{2234}\u{2234}\u{2234}  cogito".into()
            )]
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tree_tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[StreamEvent]) -> ToolTreeModel {
        let mut m = ToolTreeModel::new();
        for e in events {
            m.on_event(e);
        }
        m
    }

    #[test]
    fn turn_started_pushes_empty_group() {
        let m = run(&[StreamEvent::TurnStarted]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].turn_idx, 1);
        assert!(m.turns[0].nodes.is_empty());
    }

    #[test]
    fn tool_dispatch_started_appends_running_node() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: json!({}),
            },
        ]);
        assert_eq!(m.turns[0].nodes.len(), 1);
        assert_eq!(m.turns[0].nodes[0].call_id, "c1");
        assert!(matches!(m.turns[0].nodes[0].status, ToolStatus::Running));
    }

    #[test]
    fn tool_dispatch_ended_updates_status_to_ok() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert!(matches!(m.turns[0].nodes[0].status, ToolStatus::Ok { .. }));
    }

    #[test]
    fn tool_dispatch_ended_with_error_captures_message() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: false,
                error_message: Some("boom".into()),
            },
        ]);
        match &m.turns[0].nodes[0].status {
            ToolStatus::Err { message, .. } => assert_eq!(message, "boom"),
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[test]
    fn multi_tool_within_one_turn_lands_in_one_group() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "a".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c1".into(),
                ok: true,
                error_message: None,
            },
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "b".into(),
                args: json!({}),
            },
            StreamEvent::ToolDispatchEnded {
                call_id: "c2".into(),
                ok: true,
                error_message: None,
            },
        ]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].nodes.len(), 2);
    }

    #[test]
    fn separate_turns_produce_separate_groups() {
        let m = run(&[
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "a".into(),
                args: json!({}),
            },
            StreamEvent::TurnStarted,
            StreamEvent::ToolDispatchStarted {
                call_id: "c2".into(),
                tool_name: "b".into(),
                args: json!({}),
            },
        ]);
        assert_eq!(m.turns.len(), 2);
        assert_eq!(m.turns[0].nodes.len(), 1);
        assert_eq!(m.turns[1].nodes.len(), 1);
        assert_eq!(m.turns[0].turn_idx, 1);
        assert_eq!(m.turns[1].turn_idx, 2);
    }

    #[test]
    fn tool_without_prior_turn_started_opens_turn_implicitly() {
        let m = run(&[StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        }]);
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].nodes.len(), 1);
    }

    #[test]
    fn text_events_are_noops_for_tree() {
        let m = run(&[
            StreamEvent::TextDelta { chunk: "x".into() },
            StreamEvent::ThinkingDelta { chunk: "y".into() },
            StreamEvent::TurnCompleted,
        ]);
        assert!(m.turns.is_empty());
    }
}

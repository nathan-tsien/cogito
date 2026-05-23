//! `StandardProjector` -- reference `HistoryProjector` implementing the
//! covered-set projection algorithm from ADR-0008 §"Projection semantics".

use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::{CompactionReplacement, HistoryProjector, ProjectedMessage};
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::event::EventPayload;
use cogito_protocol::ids::TurnId;
use cogito_protocol::strategy::HarnessStrategy;

/// Reference projector. Pure synchronous function over events + strategy.
///
/// Implements the covered-set algorithm from ADR-0008:
/// 1. Build the union of all `ContextCompacted.replaced_seq_range` intervals.
/// 2. Find the latest `SystemPromptInjected` for `current_turn`; assemble the system message.
/// 3. Walk events in order, skipping those whose `seq` falls inside any covered range.
/// 4. Emit `ProjectedMessage` values according to each non-covered event's payload.
#[derive(Default, Clone, Copy, Debug)]
pub struct StandardProjector;

impl HistoryProjector for StandardProjector {
    fn project(
        &self,
        events: &[ConversationEvent],
        strategy: &HarnessStrategy,
        current_turn: TurnId,
    ) -> Vec<ProjectedMessage> {
        let covered = collect_covered_ranges(events);
        let suffix = find_system_prompt_suffix(events, current_turn);

        let system_text = if suffix.is_empty() {
            strategy.system_prompt.clone()
        } else {
            format!("{}\n\n{}", strategy.system_prompt, suffix)
        };

        let mut messages: Vec<ProjectedMessage> = vec![ProjectedMessage::System(system_text)];

        // Buffer for assistant content blocks accumulated between flush points.
        // Thinking blocks are prepended (must precede Text/ToolUse per provider requirements).
        let mut assistant_buf: AssistantBuffer = AssistantBuffer::new();

        for event in events {
            if is_covered(event.seq, &covered) {
                continue;
            }

            match &event.payload {
                EventPayload::ContextCompacted { replacement, .. } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    // Drop and any future variants emit nothing; only Summary
                    // injects a replacement user message.
                    if let CompactionReplacement::Summary { text, .. } = replacement {
                        messages.push(ProjectedMessage::User(format!(
                            "<conversation_summary>\n{text}\n</conversation_summary>"
                        )));
                    }
                }

                EventPayload::TurnStarted { user_input, .. } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    // Extract text from user input blocks. Non-text blocks (images, etc.)
                    // are skipped in v0.1: `ProjectedMessage::User` carries a plain String,
                    // and multimodal user input is not yet a v0.1 scenario. The decision to
                    // concatenate Text blocks here is intentional and documented.
                    let text = extract_text_from_blocks(user_input);
                    messages.push(ProjectedMessage::User(text));
                }

                EventPayload::AssistantMessageAppended { text } => {
                    assistant_buf.push_text(text.clone());
                }

                EventPayload::ToolUseRecorded {
                    call_id,
                    tool_name,
                    args,
                } => {
                    assistant_buf.push_tool_use(call_id.clone(), tool_name.clone(), args.clone());
                }

                EventPayload::ToolResultRecorded { call_id, result } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    let result_blocks = vec![ContentBlock::ToolResult {
                        call_id: call_id.clone(),
                        result: result.clone(),
                    }];
                    messages.push(ProjectedMessage::ToolResult {
                        call_id: call_id.clone(),
                        result_blocks,
                    });
                }

                EventPayload::ThinkingBlockRecorded {
                    text,
                    provider_opaque,
                } => {
                    // Thinking blocks must precede Text/ToolUse in the same assistant message.
                    assistant_buf.prepend_thinking(text.clone(), provider_opaque.clone());
                }

                // All other variants (session/harness-meta/context-decision) are
                // ignored by the projector. The wildcard covers both the known
                // ignore-list and any future non-exhaustive variants.
                _ => {}
            }
        }

        flush_assistant(&mut assistant_buf, &mut messages);
        messages
    }

    fn id(&self) -> &'static str {
        "standard"
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// One compacted interval (inclusive on both ends).
#[derive(Clone, Copy)]
struct SeqRange {
    start: u64,
    end: u64,
}

/// Build the set-union of all `ContextCompacted.replaced_seq_range` intervals
/// found in `events`. Non-overlapping ranges are kept as separate entries;
/// the resulting list does not need to be merged because `is_covered` only
/// asks "is this seq inside any range?".
fn collect_covered_ranges(events: &[ConversationEvent]) -> Vec<SeqRange> {
    let mut ranges = Vec::new();
    for event in events {
        if let EventPayload::ContextCompacted {
            replaced_seq_range, ..
        } = &event.payload
        {
            ranges.push(SeqRange {
                start: replaced_seq_range.0,
                end: replaced_seq_range.1,
            });
        }
    }
    ranges
}

/// Returns `true` if `seq` falls inside any of the covered ranges.
fn is_covered(seq: u64, covered: &[SeqRange]) -> bool {
    covered.iter().any(|r| seq >= r.start && seq <= r.end)
}

/// Find the `suffix` of the latest `SystemPromptInjected` event for `current_turn`.
/// Returns an empty string if no such event exists.
fn find_system_prompt_suffix(events: &[ConversationEvent], current_turn: TurnId) -> String {
    for event in events.iter().rev() {
        if let EventPayload::SystemPromptInjected {
            turn_id, suffix, ..
        } = &event.payload
        {
            if turn_id == &current_turn {
                return suffix.clone();
            }
        }
    }
    String::new()
}

/// Join all `ContentBlock::Text` blocks into a single string (space-joined).
/// Non-text blocks are skipped. This is the v0.1 user-input projection rule:
/// `TurnStarted.user_input` is `Vec<ContentBlock>` but `ProjectedMessage::User`
/// carries a `String`. Multimodal user blocks are deferred to v0.2.
fn extract_text_from_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| {
            if let ContentBlock::Text { text } = b {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Accumulated assistant content blocks. Thinking blocks are collected
/// separately and prepended when flushing so they always appear first in the
/// assembled `Vec<ContentBlock>` (per provider requirements in ADR-0019 §4).
struct AssistantBuffer {
    /// Thinking blocks (prepended to the front on flush).
    thinking: Vec<ContentBlock>,
    /// Text and `ToolUse` blocks (appended after thinking on flush).
    body: Vec<ContentBlock>,
}

impl AssistantBuffer {
    fn new() -> Self {
        Self {
            thinking: Vec::new(),
            body: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.thinking.is_empty() && self.body.is_empty()
    }

    fn push_text(&mut self, text: String) {
        self.body.push(ContentBlock::Text { text });
    }

    fn push_tool_use(&mut self, call_id: String, tool_name: String, args: serde_json::Value) {
        self.body.push(ContentBlock::ToolUse {
            call_id,
            tool_name,
            args,
        });
    }

    fn prepend_thinking(&mut self, text: String, provider_opaque: Option<serde_json::Value>) {
        self.thinking.push(ContentBlock::Thinking {
            text,
            provider_opaque,
        });
    }

    /// Drain and return all accumulated blocks (thinking first, then body).
    fn drain(&mut self) -> Vec<ContentBlock> {
        let mut blocks = std::mem::take(&mut self.thinking);
        blocks.extend(std::mem::take(&mut self.body));
        blocks
    }
}

/// Flush the assistant buffer into `messages` as a `ProjectedMessage::Assistant`,
/// then clear the buffer. No-op if the buffer is empty.
fn flush_assistant(buf: &mut AssistantBuffer, messages: &mut Vec<ProjectedMessage>) {
    if !buf.is_empty() {
        messages.push(ProjectedMessage::Assistant(buf.drain()));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
mod tests {
    use chrono::Utc;
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::context::{CompactionReplacement, ProjectedMessage};
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;
    use cogito_protocol::tool::ToolResult;

    use super::StandardProjector;
    use cogito_protocol::context::HistoryProjector as _;

    // Build a minimal `ConversationEvent` envelope with a monotonic seq.
    fn make_event(seq: u64, turn_id: Option<TurnId>, payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id,
            seq,
            ts: Utc::now(),
            payload,
        }
    }

    fn text_block(s: &str) -> ContentBlock {
        ContentBlock::Text { text: s.into() }
    }

    // ---------------------------------------------------------------------------
    // Test 1: basic two-turn session with no compaction
    // ---------------------------------------------------------------------------

    #[test]
    fn projects_basic_two_turn_session_without_compaction() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();
        let turn2 = TurnId::new();

        let events = vec![
            make_event(
                0,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("Hello")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::AssistantMessageAppended {
                    text: "Hi there!".into(),
                },
            ),
            make_event(
                2,
                Some(turn2),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("How are you?")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                3,
                Some(turn2),
                EventPayload::AssistantMessageAppended {
                    text: "I am doing well.".into(),
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn2);

        // System + User(turn1) + Assistant(turn1) + User(turn2) + Assistant(turn2).
        assert_eq!(messages.len(), 5, "expected 5 messages, got: {messages:?}");

        assert!(matches!(&messages[0], ProjectedMessage::System(_)));
        assert_eq!(messages[1], ProjectedMessage::User("Hello".into()));
        assert_eq!(
            messages[2],
            ProjectedMessage::Assistant(vec![text_block("Hi there!")])
        );
        assert_eq!(messages[3], ProjectedMessage::User("How are you?".into()));
        assert_eq!(
            messages[4],
            ProjectedMessage::Assistant(vec![text_block("I am doing well.")])
        );
    }

    // ---------------------------------------------------------------------------
    // Test 2: single Drop compaction covering early events
    // ---------------------------------------------------------------------------

    #[test]
    fn projects_single_drop_compaction() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();
        let turn2 = TurnId::new();
        let turn3 = TurnId::new();

        let events = vec![
            // seq 0-1: turn 1 (will be covered and dropped)
            make_event(
                0,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("Old message")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::AssistantMessageAppended {
                    text: "Old reply".into(),
                },
            ),
            // seq 2: compaction covering seq 0-1 (Drop)
            make_event(
                2,
                Some(turn2),
                EventPayload::ContextCompacted {
                    turn_id: turn2,
                    replaced_seq_range: (0, 1),
                    produced_by: "truncate".into(),
                    replacement: CompactionReplacement::Drop,
                    token_estimate_before: None,
                    token_estimate_after: None,
                },
            ),
            // seq 3-4: turn 3 (not covered, should appear)
            make_event(
                3,
                Some(turn3),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("New message")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                4,
                Some(turn3),
                EventPayload::AssistantMessageAppended {
                    text: "New reply".into(),
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn3);

        // System only (compaction itself emits nothing for Drop) + User(turn3) + Assistant(turn3).
        assert_eq!(messages.len(), 3, "got: {messages:?}");
        assert!(matches!(&messages[0], ProjectedMessage::System(_)));
        assert_eq!(messages[1], ProjectedMessage::User("New message".into()));
        assert_eq!(
            messages[2],
            ProjectedMessage::Assistant(vec![text_block("New reply")])
        );
    }

    // ---------------------------------------------------------------------------
    // Test 3: single Summary compaction emits the summary block
    // ---------------------------------------------------------------------------

    #[test]
    fn projects_single_summary_compaction() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();
        let turn2 = TurnId::new();
        let turn3 = TurnId::new();

        let events = vec![
            // seq 0-1: turn 1 (covered and replaced by summary)
            make_event(
                0,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("Early message")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::AssistantMessageAppended {
                    text: "Early reply".into(),
                },
            ),
            // seq 2: compaction with Summary replacement
            make_event(
                2,
                Some(turn2),
                EventPayload::ContextCompacted {
                    turn_id: turn2,
                    replaced_seq_range: (0, 1),
                    produced_by: "summarize".into(),
                    replacement: CompactionReplacement::Summary {
                        text: "The user asked an early question.".into(),
                        model: "claude-haiku-4-5".into(),
                    },
                    token_estimate_before: Some(500),
                    token_estimate_after: Some(80),
                },
            ),
            // seq 3-4: turn 3
            make_event(
                3,
                Some(turn3),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("Latest message")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                4,
                Some(turn3),
                EventPayload::AssistantMessageAppended {
                    text: "Latest reply".into(),
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn3);

        // System + User(summary) + User(turn3) + Assistant(turn3).
        assert_eq!(messages.len(), 4, "got: {messages:?}");

        let summary_msg = &messages[1];
        assert_eq!(
            *summary_msg,
            ProjectedMessage::User(
                "<conversation_summary>\nThe user asked an early question.\n</conversation_summary>"
                    .into()
            )
        );
        assert_eq!(messages[2], ProjectedMessage::User("Latest message".into()));
    }

    // ---------------------------------------------------------------------------
    // Test 4: system prompt suffix is appended for the current turn
    // ---------------------------------------------------------------------------

    #[test]
    fn system_prompt_includes_injected_suffix() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();

        let events = vec![
            make_event(
                0,
                Some(turn1),
                EventPayload::SystemPromptInjected {
                    turn_id: turn1,
                    suffix: "Today is 2026-05-23.".into(),
                    contributors: vec!["date".into()],
                    produced_by: "date-injector".into(),
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("What day is it?")],
                    activate_skills: vec![],
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn1);

        let ProjectedMessage::System(sys) = &messages[0] else {
            panic!("first message must be System");
        };
        assert!(
            sys.ends_with("Today is 2026-05-23."),
            "system prompt should end with injected suffix, got: {sys:?}"
        );
        assert!(
            sys.contains("\n\n"),
            "system prompt and suffix must be separated by a blank line"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 5: thinking blocks are prepended before text in assistant message
    // ---------------------------------------------------------------------------

    #[test]
    fn thinking_block_precedes_text_in_assistant_message() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();

        let events = vec![
            make_event(
                0,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("Think first")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::ThinkingBlockRecorded {
                    text: "Let me reason...".into(),
                    provider_opaque: Some(serde_json::json!({"signature": "sig1"})),
                },
            ),
            make_event(
                2,
                Some(turn1),
                EventPayload::AssistantMessageAppended {
                    text: "The answer is 42.".into(),
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn1);

        // System + User + Assistant(Thinking, Text).
        assert_eq!(messages.len(), 3);
        let ProjectedMessage::Assistant(blocks) = &messages[2] else {
            panic!("expected Assistant message");
        };
        assert_eq!(blocks.len(), 2, "expected 2 blocks (Thinking + Text)");
        assert!(
            matches!(blocks[0], ContentBlock::Thinking { .. }),
            "first block must be Thinking"
        );
        assert!(
            matches!(blocks[1], ContentBlock::Text { .. }),
            "second block must be Text"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 6: tool call + result round-trip
    // ---------------------------------------------------------------------------

    #[test]
    fn tool_call_and_result_projected_correctly() {
        let strategy = HarnessStrategy::default_with_model("test");
        let turn1 = TurnId::new();

        let events = vec![
            make_event(
                0,
                Some(turn1),
                EventPayload::TurnStarted {
                    user_input: vec![text_block("List files")],
                    activate_skills: vec![],
                },
            ),
            make_event(
                1,
                Some(turn1),
                EventPayload::ToolUseRecorded {
                    call_id: "call_abc".into(),
                    tool_name: "list_files".into(),
                    args: serde_json::json!({"path": "/tmp"}),
                },
            ),
            make_event(
                2,
                Some(turn1),
                EventPayload::ToolResultRecorded {
                    call_id: "call_abc".into(),
                    result: ToolResult::text("file1.rs\nfile2.rs"),
                },
            ),
        ];

        let proj = StandardProjector;
        let messages = proj.project(&events, &strategy, turn1);

        // System + User + Assistant(ToolUse) + ToolResult.
        assert_eq!(messages.len(), 4, "got: {messages:?}");

        let ProjectedMessage::Assistant(blocks) = &messages[2] else {
            panic!("expected Assistant for tool use");
        };
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], ContentBlock::ToolUse { .. }));

        assert!(
            matches!(&messages[3], ProjectedMessage::ToolResult { call_id, .. } if call_id == "call_abc"),
            "expected ToolResult with call_id call_abc"
        );
    }
}

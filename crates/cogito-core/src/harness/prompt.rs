//! H04 Prompt Composer — pure, deterministic projection of an event log
//! plus a strategy and a tool surface into a `ModelInput`.
//!
//! See `docs/components/H04-prompt-composer.md` for the projection table.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::{HistoryProjector, ProjectedMessage};
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::ids::TurnId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolDescriptor;

/// Compose the next `ModelInput`. Pure: same inputs → same output.
///
/// Delegates history projection to `projector`, which applies the
/// covered-set algorithm (compaction awareness, system-prompt suffix
/// from `SystemPromptInjected` events). The resulting `Vec<ProjectedMessage>`
/// is converted into the wire-format `ModelInput` sent to the gateway.
#[must_use]
pub fn compose(
    history: &[ConversationEvent],
    strategy: &HarnessStrategy,
    surface: &[ToolDescriptor],
    projector: &dyn HistoryProjector,
    current_turn: TurnId,
) -> ModelInput {
    let projected = projector.project(history, strategy, current_turn);
    projected_to_model_input(projected, surface, strategy)
}

/// Convert a `Vec<ProjectedMessage>` (output of `HistoryProjector::project`)
/// into a `ModelInput` for the gateway.
///
/// The first `ProjectedMessage::System` becomes `ModelInput.system`; all
/// remaining messages become the `messages` list. If no `System` message is
/// present, `strategy.system_prompt` is used as the fallback.
fn projected_to_model_input(
    projected: Vec<ProjectedMessage>,
    surface: &[ToolDescriptor],
    strategy: &HarnessStrategy,
) -> ModelInput {
    let mut system: String = strategy.system_prompt.clone();
    let mut messages: Vec<Message> = Vec::new();

    for msg in projected {
        match msg {
            ProjectedMessage::System(text) => {
                // Use the first System message as the system field; subsequent
                // System messages (unexpected but tolerated) are ignored.
                if messages.is_empty() {
                    system = text;
                }
            }
            ProjectedMessage::User(text) => {
                messages.push(Message::User {
                    content: vec![ContentBlock::Text { text }],
                });
            }
            ProjectedMessage::Assistant(blocks) => {
                messages.push(Message::Assistant { content: blocks });
            }
            ProjectedMessage::ToolResult {
                call_id,
                result_blocks,
            } => {
                // Tool results are fed back as User-role messages per the
                // Anthropic wire format.
                messages.push(Message::User {
                    content: result_blocks
                        .into_iter()
                        .map(|b| match b {
                            ContentBlock::ToolResult { .. } => b,
                            // Re-wrap non-ToolResult blocks (defensive).
                            other => ContentBlock::ToolResult {
                                call_id: call_id.clone(),
                                result: cogito_protocol::tool::ToolResult::text(format!(
                                    "{other:?}"
                                )),
                            },
                        })
                        .collect(),
                });
            }
            // ProjectedMessage is #[non_exhaustive]; future variants are
            // silently ignored to preserve forward compatibility.
            _ => {}
        }
    }

    ModelInput {
        system,
        messages,
        tools: surface.to_vec(),
        params: strategy.model_params.clone(),
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::match_wildcard_for_single_variants,
    clippy::unwrap_used,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::context::{HistoryProjector, ProjectedMessage};
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::gateway::Message;
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;

    fn evt(seq: u64, payload: EventPayload, turn_id: Option<TurnId>) -> ConversationEvent {
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

    // ---------------------------------------------------------------------------
    // Stub projectors — defined at module level to avoid items_after_statements.
    // ---------------------------------------------------------------------------

    /// Marker text injected by the stub projector.
    const STUB_MARKER: &str = "stub-marker-message";

    /// Returns `[System(strategy.system_prompt), User(STUB_MARKER)]`.
    struct MarkerProjector;

    impl HistoryProjector for MarkerProjector {
        fn project(
            &self,
            _events: &[ConversationEvent],
            strategy: &HarnessStrategy,
            _current_turn: TurnId,
        ) -> Vec<ProjectedMessage> {
            vec![
                ProjectedMessage::System(strategy.system_prompt.clone()),
                ProjectedMessage::User(STUB_MARKER.into()),
            ]
        }

        fn id(&self) -> &'static str {
            "marker-stub"
        }
    }

    /// Returns `[System("overridden system prompt")]` regardless of input.
    struct OverrideSystemProjector;

    impl HistoryProjector for OverrideSystemProjector {
        fn project(
            &self,
            _events: &[ConversationEvent],
            _strategy: &HarnessStrategy,
            _current_turn: TurnId,
        ) -> Vec<ProjectedMessage> {
            vec![ProjectedMessage::System("overridden system prompt".into())]
        }

        fn id(&self) -> &'static str {
            "override-stub"
        }
    }

    /// Mimics the pre-Task-29 inline projection: extracts text content and
    /// preserves Thinking-before-Text ordering in assistant messages.
    struct ThinkingOrderProjector;

    impl HistoryProjector for ThinkingOrderProjector {
        fn project(
            &self,
            events: &[ConversationEvent],
            strategy: &HarnessStrategy,
            _current_turn: TurnId,
        ) -> Vec<ProjectedMessage> {
            let mut out: Vec<ProjectedMessage> =
                vec![ProjectedMessage::System(strategy.system_prompt.clone())];
            let mut thinking: Vec<ContentBlock> = Vec::new();
            let mut body: Vec<ContentBlock> = Vec::new();

            for ev in events {
                match &ev.payload {
                    EventPayload::TurnStarted { user_input, .. } => {
                        let text = user_input
                            .iter()
                            .filter_map(|b| {
                                if let ContentBlock::Text { text } = b {
                                    Some(text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push(ProjectedMessage::User(text));
                    }
                    EventPayload::ThinkingBlockRecorded {
                        text,
                        provider_opaque,
                    } => {
                        thinking.push(ContentBlock::Thinking {
                            text: text.clone(),
                            provider_opaque: provider_opaque.clone(),
                        });
                    }
                    EventPayload::AssistantMessageAppended { text } => {
                        body.push(ContentBlock::Text { text: text.clone() });
                    }
                    EventPayload::ToolUseRecorded {
                        call_id,
                        tool_name,
                        args,
                    } => {
                        body.push(ContentBlock::ToolUse {
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                            args: args.clone(),
                        });
                    }
                    _ => {}
                }
            }

            if !thinking.is_empty() || !body.is_empty() {
                let mut blocks = thinking;
                blocks.extend(body);
                out.push(ProjectedMessage::Assistant(blocks));
            }
            out
        }

        fn id(&self) -> &'static str {
            "thinking-order-stub"
        }
    }

    // ---------------------------------------------------------------------------
    // Task 29: verify H04 delegates to the injected projector
    // ---------------------------------------------------------------------------

    #[test]
    fn h04_uses_history_projector_from_pipeline() {
        let turn_id = TurnId::new();
        let strategy = HarnessStrategy::default_with_model("test-model");

        let history = vec![evt(
            0,
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text {
                    text: "ignored".into(),
                }],
                activate_skills: vec![],
            },
            Some(turn_id),
        )];

        let input = compose(&history, &strategy, &[], &MarkerProjector, turn_id);

        assert_eq!(input.system, strategy.system_prompt);
        assert_eq!(input.messages.len(), 1, "expected one User message");
        match &input.messages[0] {
            Message::User { content } => {
                assert_eq!(content.len(), 1);
                assert!(
                    matches!(&content[0], ContentBlock::Text { text } if text == STUB_MARKER),
                    "marker text must appear in User message"
                );
            }
            other => panic!("expected User message, got {other:?}"),
        }
    }

    #[test]
    fn compose_system_is_taken_from_projector_system_message() {
        let turn_id = TurnId::new();
        let mut strategy = HarnessStrategy::default_with_model("test-model");
        strategy.system_prompt = "base system".into();

        let input = compose(&[], &strategy, &[], &OverrideSystemProjector, turn_id);

        assert_eq!(
            input.system, "overridden system prompt",
            "system prompt must come from projector output"
        );
        assert!(input.messages.is_empty());
    }

    // ---------------------------------------------------------------------------
    // Retained: verify Thinking blocks precede Text in assistant messages.
    // ---------------------------------------------------------------------------

    #[test]
    fn project_history_emits_thinking_before_text_within_assistant_message() {
        let turn_id = TurnId::new();
        let strategy = HarnessStrategy::default_with_model("test-model");

        let history = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![ContentBlock::Text { text: "go".into() }],
                    activate_skills: vec![],
                },
                Some(turn_id),
            ),
            evt(
                1,
                EventPayload::ThinkingBlockRecorded {
                    text: "I should grep.".into(),
                    provider_opaque: Some(serde_json::json!({"signature":"sig"})),
                },
                Some(turn_id),
            ),
            evt(
                2,
                EventPayload::AssistantMessageAppended { text: "OK.".into() },
                Some(turn_id),
            ),
            evt(
                3,
                EventPayload::ToolUseRecorded {
                    call_id: "c1".into(),
                    tool_name: "grep".into(),
                    args: serde_json::json!({"pattern":"foo"}),
                },
                Some(turn_id),
            ),
        ];

        let input = compose(&history, &strategy, &[], &ThinkingOrderProjector, turn_id);

        assert_eq!(input.messages.len(), 2, "1 user + 1 assistant message");
        match &input.messages[1] {
            Message::Assistant { content } => {
                assert_eq!(content.len(), 3);
                assert!(
                    matches!(content[0], ContentBlock::Thinking { .. }),
                    "Thinking must be at index 0 (precedes Text/ToolUse)"
                );
                assert!(matches!(content[1], ContentBlock::Text { .. }));
                assert!(matches!(content[2], ContentBlock::ToolUse { .. }));
            }
            other => panic!("expected Assistant message, got {other:?}"),
        }
    }
}

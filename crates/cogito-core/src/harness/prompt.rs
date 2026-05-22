//! H04 Prompt Composer — pure, deterministic projection of an event log
//! plus a strategy and a tool surface into a `ModelInput`.
//!
//! See `docs/components/H04-prompt-composer.md` for the projection table.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolDescriptor;

/// Compose the next `ModelInput`. Pure: same inputs → same output.
#[must_use]
pub fn compose(
    history: &[ConversationEvent],
    strategy: &HarnessStrategy,
    surface: &[ToolDescriptor],
) -> ModelInput {
    let messages = project_history(history);
    ModelInput {
        system: strategy.system_prompt.clone(),
        messages,
        tools: surface.to_vec(),
        params: strategy.model_params.clone(),
    }
}

/// Project the event log into a `Vec<Message>` per the table in
/// `docs/components/H04-prompt-composer.md` §"History projection".
fn project_history(history: &[ConversationEvent]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut current_assistant: Option<Vec<ContentBlock>> = None;

    for evt in history {
        match &evt.payload {
            EventPayload::TurnStarted { user_input } => {
                flush_assistant(&mut current_assistant, &mut out);
                out.push(Message::User {
                    content: user_input.clone(),
                });
            }
            EventPayload::AssistantMessageAppended { text } => {
                current_assistant
                    .get_or_insert_with(Vec::new)
                    .push(ContentBlock::Text { text: text.clone() });
            }
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
            } => {
                current_assistant
                    .get_or_insert_with(Vec::new)
                    .push(ContentBlock::ToolUse {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                        args: args.clone(),
                    });
            }
            EventPayload::ThinkingBlockRecorded {
                text,
                provider_opaque,
            } => {
                current_assistant
                    .get_or_insert_with(Vec::new)
                    .push(ContentBlock::Thinking {
                        text: text.clone(),
                        provider_opaque: provider_opaque.clone(),
                    });
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                flush_assistant(&mut current_assistant, &mut out);
                out.push(Message::User {
                    content: vec![ContentBlock::ToolResult {
                        call_id: call_id.clone(),
                        result: result.clone(),
                    }],
                });
            }
            // Control / hook / context events do not project into messages.
            _ => {}
        }
    }
    flush_assistant(&mut current_assistant, &mut out);
    out
}

/// Flush the accumulated assistant blocks into an `Assistant` message, if any.
fn flush_assistant(current: &mut Option<Vec<ContentBlock>>, out: &mut Vec<Message>) {
    if let Some(content) = current.take() {
        out.push(Message::Assistant { content });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::gateway::Message;
    use cogito_protocol::ids::{EventId, SessionId, TurnId};

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

    #[test]
    fn project_history_emits_thinking_before_text_within_assistant_message() {
        let turn_id = TurnId::new();
        let history = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![ContentBlock::Text { text: "go".into() }],
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

        let messages = project_history(&history);
        assert_eq!(messages.len(), 2, "1 user + 1 assistant message");
        #[allow(clippy::panic)]
        match &messages[1] {
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

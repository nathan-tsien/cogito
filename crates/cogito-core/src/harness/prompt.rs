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

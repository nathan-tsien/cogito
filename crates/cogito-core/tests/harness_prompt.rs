//! Integration tests for H04 Prompt Composer.

use cogito_core::harness::prompt::compose;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::Message;
use cogito_protocol::ids::{EventId, SessionId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolResult;
use cogito_protocol::SCHEMA_VERSION;
use chrono::Utc;

fn evt(payload: EventPayload, seq: u64) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id: SessionId::new(),
        turn_id: None,
        seq,
        ts: Utc::now(),
        payload,
    }
}

#[test]
fn empty_history_yields_empty_messages() {
    let strategy = HarnessStrategy::default_with_model("test");
    let input = compose(&[], &strategy, &[]);
    assert_eq!(input.system, strategy.system_prompt);
    assert!(input.messages.is_empty());
    assert!(input.tools.is_empty());
}

#[test]
fn single_user_turn_projects_to_user_message() {
    let events = vec![evt(
        EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: "hi".into() }],
        },
        1,
    )];
    let strategy = HarnessStrategy::default_with_model("test");
    let input = compose(&events, &strategy, &[]);
    assert_eq!(input.messages.len(), 1);
    assert!(
        matches!(&input.messages[0], Message::User { content } if content.len() == 1)
    );
}

#[test]
fn assistant_with_tool_use_and_result_round_trip() {
    let events = vec![
        evt(
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text {
                    text: "read it".into(),
                }],
            },
            1,
        ),
        evt(
            EventPayload::AssistantMessageAppended {
                text: "ok".into(),
            },
            2,
        ),
        evt(
            EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({ "path": "/tmp/x" }),
            },
            3,
        ),
        evt(
            EventPayload::ToolResultRecorded {
                call_id: "c1".into(),
                result: ToolResult::text("contents"),
            },
            4,
        ),
    ];
    let strategy = HarnessStrategy::default_with_model("test");
    let input = compose(&events, &strategy, &[]);
    // Expected: User, Assistant (text+tool_use), User (tool_result)
    assert_eq!(input.messages.len(), 3);
    assert!(matches!(input.messages[0], Message::User { .. }));
    assert!(matches!(input.messages[1], Message::Assistant { ref content } if content.len() == 2));
    assert!(
        matches!(&input.messages[2], Message::User { content } if matches!(content[0], ContentBlock::ToolResult { .. }))
    );
}

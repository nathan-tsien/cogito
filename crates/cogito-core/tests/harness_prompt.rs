//! Integration tests for H04 Prompt Composer.

use chrono::Utc;
use cogito_context::projector::standard::StandardProjector;
use cogito_core::harness::prompt::compose;
use cogito_protocol::SCHEMA_VERSION;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::Message;
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolResult;

fn evt(payload: EventPayload, seq: u64, turn_id: TurnId) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id: SessionId::new(),
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload,
    }
}

#[test]
fn empty_history_yields_empty_messages() {
    let turn_id = TurnId::new();
    let strategy = HarnessStrategy::default_with_model("test");
    let projector = StandardProjector;
    let input = compose(&[], &strategy, &[], &projector, turn_id);
    assert_eq!(input.system, strategy.system_prompt);
    // StandardProjector emits only System for empty history; projected_to_model_input
    // consumes the System as the system field and leaves messages empty.
    assert!(input.messages.is_empty());
    assert!(input.tools.is_empty());
}

#[test]
fn single_user_turn_projects_to_user_message() {
    let turn_id = TurnId::new();
    let events = vec![evt(
        EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: "hi".into() }],
        },
        1,
        turn_id,
    )];
    let strategy = HarnessStrategy::default_with_model("test");
    let projector = StandardProjector;
    let input = compose(&events, &strategy, &[], &projector, turn_id);
    // System is consumed into input.system; only the User message remains.
    assert_eq!(input.messages.len(), 1);
    assert!(matches!(&input.messages[0], Message::User { content } if content.len() == 1));
}

#[test]
fn assistant_with_tool_use_and_result_round_trip() {
    let turn_id = TurnId::new();
    let events = vec![
        evt(
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text {
                    text: "read it".into(),
                }],
            },
            1,
            turn_id,
        ),
        evt(
            EventPayload::AssistantMessageAppended { text: "ok".into() },
            2,
            turn_id,
        ),
        evt(
            EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({ "path": "/tmp/x" }),
            },
            3,
            turn_id,
        ),
        evt(
            EventPayload::ToolResultRecorded {
                call_id: "c1".into(),
                result: ToolResult::text("contents"),
            },
            4,
            turn_id,
        ),
    ];
    let strategy = HarnessStrategy::default_with_model("test");
    let projector = StandardProjector;
    let input = compose(&events, &strategy, &[], &projector, turn_id);
    // Expected: User, Assistant (text+tool_use), User (tool_result)
    assert_eq!(input.messages.len(), 3);
    assert!(matches!(input.messages[0], Message::User { .. }));
    assert!(matches!(input.messages[1], Message::Assistant { ref content } if content.len() == 2));
    assert!(
        matches!(&input.messages[2], Message::User { content } if matches!(content[0], ContentBlock::ToolResult { .. }))
    );
}

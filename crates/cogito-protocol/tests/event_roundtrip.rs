//! Serde roundtrip tests for the new `EventPayload` variants added in Sprint 2.

use chrono::Utc;
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::ids::{EventId, SessionId, TurnId};

#[test]
fn context_manage_entered_round_trip() -> serde_json::Result<()> {
    let evt = ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id: SessionId::new(),
        turn_id: Some(TurnId::new()),
        seq: 1,
        ts: Utc::now(),
        payload: EventPayload::ContextManageEntered {
            turn_id: TurnId::new(),
        },
    };
    let back: ConversationEvent = serde_json::from_str(&serde_json::to_string(&evt)?)?;
    assert_eq!(evt.event_id, back.event_id);
    assert!(matches!(back.payload, EventPayload::ContextManageEntered { .. }));
    Ok(())
}

#[test]
fn prompt_composed_round_trip() -> serde_json::Result<()> {
    let payload = EventPayload::PromptComposed {
        turn_id: TurnId::new(),
        model: "claude-opus-4-7".into(),
        surface_size: 3,
    };
    let back: EventPayload = serde_json::from_str(&serde_json::to_string(&payload)?)?;
    assert_eq!(payload, back);
    Ok(())
}

#[test]
fn model_call_started_round_trip() -> serde_json::Result<()> {
    let payload = EventPayload::ModelCallStarted {
        turn_id: TurnId::new(),
        model: "gpt-4o-mini".into(),
    };
    let back: EventPayload = serde_json::from_str(&serde_json::to_string(&payload)?)?;
    assert_eq!(payload, back);
    Ok(())
}

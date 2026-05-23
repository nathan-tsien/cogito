//! Serde roundtrip tests for the new `EventPayload` variants added in Sprint 2.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

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
        payload: EventPayload::ContextManageEntered {},
    };
    let back: ConversationEvent = serde_json::from_str(&serde_json::to_string(&evt)?)?;
    assert_eq!(evt.event_id, back.event_id);
    assert!(matches!(
        back.payload,
        EventPayload::ContextManageEntered { .. }
    ));
    Ok(())
}

#[test]
fn prompt_composed_round_trip() -> serde_json::Result<()> {
    let payload = EventPayload::PromptComposed {
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
        model: "gpt-4o-mini".into(),
    };
    let back: EventPayload = serde_json::from_str(&serde_json::to_string(&payload)?)?;
    assert_eq!(payload, back);
    Ok(())
}

#[test]
fn skill_activated_roundtrip() {
    use cogito_protocol::event::EventPayload;
    use cogito_protocol::skill::SkillActivationChannel;
    use cogito_protocol::skill::SkillSource;

    let payload = EventPayload::SkillActivated {
        skill_name: "invoice-parser".into(),
        source: SkillSource::User,
        channel: SkillActivationChannel::ModelSigil,
    };
    let json = serde_json::to_string(&payload).expect("serialize");
    let parsed: EventPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, payload);
    assert!(json.contains("\"type\":\"skill_activated\""));
}

#[test]
fn turn_started_activate_skills_defaults_to_empty() {
    use cogito_protocol::event::EventPayload;

    // Old serialized form (without activate_skills) must still deserialize.
    // `ContentBlock` is adjacently tagged (`tag="type", content="data"`), so
    // its body sits under `data` — see `crate::content::ContentBlock`.
    let old_json =
        r#"{"type":"turn_started","data":{"user_input":[{"type":"text","data":{"text":"hi"}}]}}"#;
    let parsed: EventPayload = serde_json::from_str(old_json).expect("deserialize");
    match parsed {
        EventPayload::TurnStarted {
            user_input,
            activate_skills,
        } => {
            assert_eq!(user_input.len(), 1);
            assert!(activate_skills.is_empty());
        }
        _ => panic!("expected TurnStarted"),
    }
}

#[test]
fn turn_started_with_activate_skills_roundtrip() {
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::event::EventPayload;

    let payload = EventPayload::TurnStarted {
        user_input: vec![ContentBlock::Text { text: "go".into() }],
        activate_skills: vec!["foo".into(), "bar".into()],
    };
    let json = serde_json::to_string(&payload).expect("serialize");
    let parsed: EventPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, payload);
}

//! `EventRecorder::record_skill_activated` default-impl contract test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::event::EventPayload;
use cogito_protocol::ids::TurnId;
use cogito_protocol::skill::{SkillActivationChannel, SkillSource};
use cogito_protocol::store::EventRecorder;
use cogito_test_fixtures::context::InMemoryRecorder;

#[tokio::test]
async fn record_skill_activated_writes_event() {
    let mut recorder = InMemoryRecorder::default();
    let turn_id = TurnId::new();
    let _eid = recorder
        .record_skill_activated(
            turn_id,
            "foo".into(),
            SkillSource::User,
            SkillActivationChannel::UserSlash,
        )
        .await
        .unwrap();
    assert_eq!(recorder.events.len(), 1);
    let (_, payload) = &recorder.events[0];
    match payload {
        EventPayload::SkillActivated {
            skill_name,
            source,
            channel,
        } => {
            assert_eq!(skill_name, "foo");
            assert_eq!(*source, SkillSource::User);
            assert_eq!(*channel, SkillActivationChannel::UserSlash);
        }
        other => panic!("expected SkillActivated, got {other:?}"),
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! H06 sigil-detection side-channel: validates that the demuxer emits
//! `StreamEvent::SkillActivationRequested` only for registered names and
//! only outside code fences.

use std::sync::Arc;

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider};
use cogito_protocol::stream::StreamEvent;
use tokio::sync::broadcast;

use cogito_core::harness::stream_demux::sigil_emit_for_test;

struct OnlyFoo;
impl SkillProvider for OnlyFoo {
    fn list(&self) -> Vec<SkillMetadata> {
        vec![]
    }
    fn get(&self, _: &str) -> Option<SkillContent> {
        None
    }
    fn is_registered(&self, name: &str) -> bool {
        name == "foo"
    }
}

fn provider() -> Arc<dyn SkillProvider> {
    Arc::new(OnlyFoo)
}

#[tokio::test]
async fn emits_for_registered_name_outside_fence() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&provider(), &mut state, "use $foo please", &tx).unwrap();
    let ev = rx.try_recv().unwrap();
    assert!(matches!(
        ev,
        StreamEvent::SkillActivationRequested { skill_name } if skill_name == "foo"
    ));
}

#[tokio::test]
async fn does_not_emit_for_unregistered_name() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&provider(), &mut state, "use $bar please", &tx).unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn does_not_emit_inside_fenced_code() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&provider(), &mut state, "```\n$foo\n```\n", &tx).unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn deduplicates_same_name_in_one_chunk() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&provider(), &mut state, "$foo and $foo again", &tx).unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        StreamEvent::SkillActivationRequested { skill_name } if skill_name == "foo"
    ));
    assert!(
        rx.try_recv().is_err(),
        "second occurrence in same chunk must not re-emit"
    );
}

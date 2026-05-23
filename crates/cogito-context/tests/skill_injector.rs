//! Integration tests for `SkillInjector` — registry block + activation logic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_context::injector::skill::SkillInjector;
use cogito_protocol::context::{InjectionInput, SystemPromptInjector};
use cogito_protocol::event::EventPayload;
use cogito_protocol::exec_ctx::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_test_fixtures::context::InMemoryRecorder;

struct StaticProvider {
    skills: Vec<(SkillMetadata, String)>,
}

impl SkillProvider for StaticProvider {
    fn list(&self) -> Vec<SkillMetadata> {
        self.skills.iter().map(|(m, _)| m.clone()).collect()
    }
    fn get(&self, name: &str) -> Option<SkillContent> {
        self.skills.iter().find_map(|(m, body)| {
            if m.name == name {
                Some(SkillContent {
                    name: m.name.clone(),
                    source: m.source.clone(),
                    body: body.clone(),
                })
            } else {
                None
            }
        })
    }
    fn is_registered(&self, name: &str) -> bool {
        self.skills.iter().any(|(m, _)| m.name == name)
    }
}

fn provider() -> Arc<dyn SkillProvider> {
    Arc::new(StaticProvider {
        skills: vec![(
            SkillMetadata {
                name: "invoice-parser".into(),
                description: "Parses invoices.".into(),
                source: SkillSource::User,
                disable_model_invocation: false,
                user_invocable: true,
                version: None,
            },
            "# Invoice parser body".into(),
        )],
    })
}

#[tokio::test]
async fn empty_history_emits_registry_block_only() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
    let input = InjectionInput {
        session_id,
        turn_id,
        strategy: &strategy,
        history: &[],
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let injector = SkillInjector::new(provider());
    let _ = injector.inject(input).await.unwrap();
    let (_, payload) = recorder.events.last().unwrap();
    match payload {
        EventPayload::SystemPromptInjected {
            suffix,
            contributors,
            produced_by,
            ..
        } => {
            assert!(suffix.contains("Available Skills"));
            assert!(suffix.contains("invoice-parser"));
            assert!(contributors.is_empty(), "no activations on empty history");
            assert_eq!(produced_by, "skill");
        }
        _ => panic!("expected SystemPromptInjected"),
    }
}

#[tokio::test]
async fn empty_registry_emits_empty_suffix() {
    struct Empty;
    impl SkillProvider for Empty {
        fn list(&self) -> Vec<SkillMetadata> {
            vec![]
        }
        fn get(&self, _: &str) -> Option<SkillContent> {
            None
        }
        fn is_registered(&self, _: &str) -> bool {
            false
        }
    }
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
    let input = InjectionInput {
        session_id,
        turn_id,
        strategy: &strategy,
        history: &[],
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let injector = SkillInjector::new(Arc::new(Empty));
    let _ = injector.inject(input).await.unwrap();
    let (_, payload) = recorder.events.last().unwrap();
    match payload {
        EventPayload::SystemPromptInjected { suffix, .. } => {
            assert!(suffix.is_empty());
        }
        _ => panic!("expected SystemPromptInjected"),
    }
}

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
                    root: None,
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

use chrono::Utc;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::ids::EventId;
use cogito_protocol::skill::SkillActivationChannel;

fn make_event(seq: u64, turn_id: TurnId, payload: EventPayload) -> ConversationEvent {
    ConversationEvent {
        schema_version: cogito_protocol::event::SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id: SessionId::new(),
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload,
    }
}

#[tokio::test]
async fn user_channel_activates_from_turn_started() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);

    let history = vec![make_event(
        0,
        turn_id,
        EventPayload::TurnStarted {
            user_input: vec![],
            activate_skills: vec!["invoice-parser".into()],
        },
    )];

    let input = InjectionInput {
        session_id,
        turn_id,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let mut saw_activated = false;
    let mut saw_injected = false;
    for (_, p) in &recorder.events {
        match p {
            EventPayload::SkillActivated {
                skill_name,
                channel,
                ..
            } => {
                assert_eq!(skill_name, "invoice-parser");
                assert_eq!(*channel, SkillActivationChannel::UserSlash);
                saw_activated = true;
            }
            EventPayload::SystemPromptInjected {
                suffix,
                contributors,
                ..
            } => {
                assert!(suffix.contains("<skill name=\"invoice-parser\""));
                assert!(suffix.contains("# Invoice parser body"));
                // root is None for this provider — no path attribute emitted.
                assert!(
                    !suffix.contains("root=\""),
                    "no root attribute when SkillContent.root is None"
                );
                assert_eq!(contributors, &vec!["invoice-parser".to_string()]);
                saw_injected = true;
            }
            _ => {}
        }
    }
    assert!(saw_activated && saw_injected);
}

#[tokio::test]
async fn injected_block_carries_skill_root_path() {
    // ADR-0029: when SkillContent.root is Some, the injected <skill> block
    // must surface the absolute path so the model can resolve bundled-file
    // references (scripts/, references/, assets/) against it.
    struct RootProvider;
    impl SkillProvider for RootProvider {
        fn list(&self) -> Vec<SkillMetadata> {
            vec![SkillMetadata {
                name: "pptx".into(),
                description: "deck builder".into(),
                source: SkillSource::User,
                disable_model_invocation: false,
                user_invocable: true,
                version: None,
            }]
        }
        fn get(&self, name: &str) -> Option<SkillContent> {
            (name == "pptx").then(|| SkillContent {
                name: "pptx".into(),
                source: SkillSource::User,
                body: "# pptx body".into(),
                root: Some(std::path::PathBuf::from("/abs/skills/pptx")),
            })
        }
        fn is_registered(&self, name: &str) -> bool {
            name == "pptx"
        }
    }

    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
    let history = vec![make_event(
        0,
        turn_id,
        EventPayload::TurnStarted {
            user_input: vec![],
            activate_skills: vec!["pptx".into()],
        },
    )];
    let input = InjectionInput {
        session_id,
        turn_id,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(Arc::new(RootProvider))
        .inject(input)
        .await
        .unwrap();

    let injected = recorder
        .events
        .iter()
        .find_map(|(_, p)| match p {
            EventPayload::SystemPromptInjected { suffix, .. } => Some(suffix.clone()),
            _ => None,
        })
        .expect("a SystemPromptInjected event");
    assert!(
        injected.contains("/abs/skills/pptx"),
        "injected block must surface the skill root path; got:\n{injected}"
    );
}

#[tokio::test]
async fn model_channel_activates_from_previous_text_block() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let prev_turn = TurnId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![
        make_event(
            0,
            prev_turn,
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text { text: "hi".into() }],
                activate_skills: vec![],
            },
        ),
        make_event(
            1,
            prev_turn,
            EventPayload::AssistantMessageAppended {
                text: "Sure, $invoice-parser please.".into(),
                message_id: None,
            },
        ),
        make_event(
            2,
            cur_turn,
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text { text: "go".into() }],
                activate_skills: vec![],
            },
        ),
    ];

    let input = InjectionInput {
        session_id,
        turn_id: cur_turn,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let activated: Vec<_> = recorder
        .events
        .iter()
        .filter_map(|(_, p)| {
            if let EventPayload::SkillActivated {
                skill_name,
                channel,
                ..
            } = p
            {
                Some((skill_name.clone(), *channel))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        activated,
        vec![(
            "invoice-parser".to_string(),
            SkillActivationChannel::ModelSigil
        )]
    );
}

#[tokio::test]
async fn model_channel_respects_disable_model_invocation() {
    // Build a provider whose only skill has `disable_model_invocation: true`.
    // A $sigil in a prior turn's assistant text must NOT activate it.
    let provider: Arc<dyn SkillProvider> = Arc::new(StaticProvider {
        skills: vec![(
            SkillMetadata {
                name: "internal".into(),
                description: "internal-only skill".into(),
                source: SkillSource::User,
                disable_model_invocation: true,
                user_invocable: true,
                version: None,
            },
            "body".into(),
        )],
    });

    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let prev_turn = TurnId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![
        make_event(
            0,
            prev_turn,
            EventPayload::AssistantMessageAppended {
                text: "I'll use $internal.".into(),
                message_id: None,
            },
        ),
        make_event(
            1,
            cur_turn,
            EventPayload::TurnStarted {
                user_input: vec![],
                activate_skills: vec![],
            },
        ),
    ];

    let input = InjectionInput {
        session_id,
        turn_id: cur_turn,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider).inject(input).await.unwrap();

    let activated = recorder
        .events
        .iter()
        .filter(|(_, p)| matches!(p, EventPayload::SkillActivated { .. }))
        .count();
    assert_eq!(
        activated, 0,
        "sigil must not activate a skill with disable_model_invocation: true"
    );
}

#[tokio::test]
async fn user_channel_still_activates_when_model_invocation_disabled() {
    // Same skill, this time via user-channel /skill — should activate.
    let provider: Arc<dyn SkillProvider> = Arc::new(StaticProvider {
        skills: vec![(
            SkillMetadata {
                name: "internal".into(),
                description: "internal-only skill".into(),
                source: SkillSource::User,
                disable_model_invocation: true,
                user_invocable: true,
                version: None,
            },
            "body".into(),
        )],
    });

    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![make_event(
        0,
        cur_turn,
        EventPayload::TurnStarted {
            user_input: vec![],
            activate_skills: vec!["internal".into()],
        },
    )];

    let input = InjectionInput {
        session_id,
        turn_id: cur_turn,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider).inject(input).await.unwrap();

    let activated: Vec<_> = recorder
        .events
        .iter()
        .filter_map(|(_, p)| {
            if let EventPayload::SkillActivated {
                skill_name,
                channel,
                ..
            } = p
            {
                Some((skill_name.clone(), *channel))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        activated,
        vec![("internal".to_string(), SkillActivationChannel::UserSlash)]
    );
}

#[tokio::test]
async fn prior_activation_dedupes_repeat() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let prev_turn = TurnId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![
        make_event(
            0,
            prev_turn,
            EventPayload::SkillActivated {
                skill_name: "invoice-parser".into(),
                source: SkillSource::User,
                channel: SkillActivationChannel::ModelSigil,
            },
        ),
        make_event(
            1,
            cur_turn,
            EventPayload::TurnStarted {
                user_input: vec![],
                activate_skills: vec!["invoice-parser".into()],
            },
        ),
    ];

    let input = InjectionInput {
        session_id,
        turn_id: cur_turn,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let count = recorder
        .events
        .iter()
        .filter(|(_, p)| matches!(p, EventPayload::SkillActivated { .. }))
        .count();
    assert_eq!(count, 0, "must not re-activate already-activated skill");
}

#[tokio::test]
async fn idempotent_on_existing_system_prompt_injected() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);

    let existing_id = EventId::new();
    let history = vec![ConversationEvent {
        schema_version: cogito_protocol::event::SCHEMA_VERSION,
        event_id: existing_id,
        session_id,
        turn_id: Some(turn_id),
        seq: 0,
        ts: Utc::now(),
        payload: EventPayload::SystemPromptInjected {
            turn_id,
            suffix: "preexisting".into(),
            contributors: vec![],
            produced_by: "skill".into(),
        },
    }];

    let input = InjectionInput {
        session_id,
        turn_id,
        strategy: &strategy,
        history: &history,
        exec_ctx: &exec_ctx,
        recorder: &mut recorder,
    };
    let returned = SkillInjector::new(provider()).inject(input).await.unwrap();
    assert_eq!(returned, existing_id);
    assert!(recorder.events.is_empty(), "no new events on resume hit");
}

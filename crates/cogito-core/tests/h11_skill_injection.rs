#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! End-to-end integration: H11 with `SkillInjector` configured.
//!
//! Drives two consecutive turns through a `MockModelGateway`:
//!   - Turn 1: the model emits "use $foo please" (a model-channel sigil).
//!     The `SkillInjector` runs at the start of turn 1, finds no
//!     `AssistantMessageAppended` in any prior turn, and activates nothing.
//!   - Turn 2: triggered by a follow-up `UserText`. The `SkillInjector`
//!     re-scans history, hits the `$foo` sigil in turn 1's recorded
//!     assistant text, and activates "foo" exactly once.
//!
//! Assertions across the two scenarios:
//!   1. A `SkillActivated { skill_name = "foo" }` event lands in the log on
//!      turn 2 (model-channel activation re-derived from previous-turn text).
//!   2. Turn 2's `SystemPromptInjected.suffix` contains `<skill name="foo"`
//!      (the XML-wrapped activated body) and the "Available Skills" registry
//!      block.
//!   3. When the model emits `$foo` in both turns, only one `SkillActivated`
//!      event is recorded (idempotent dedupe against prior activations).
//!
//! Scaffolding mirrors `session_e2e.rs` (`MockModelGateway`, `JsonlStore`
//! tempdir, `Runtime::builder`), and additionally:
//!   - Overrides `strategy.context.system_prompt_injector` to
//!     `SystemPromptInjectorConfig::Skill`.
//!   - Injects a static one-skill `SkillProvider` ("foo") via
//!     `RuntimeBuilder::skills(...)`.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::context::SystemPromptInjectorConfig;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::skill::{
    SkillActivationChannel, SkillContent, SkillMetadata, SkillProvider, SkillSource,
};
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Single-skill `SkillProvider`: registers "foo" with a recognizable body
/// so we can assert against the `<skill name="foo"` envelope and any prefix
/// of the body in the injected system-prompt suffix.
struct StaticFooProvider;

const FOO_BODY: &str = "## foo\n\nA test skill body used by h11_skill_injection.";
const FOO_DESCRIPTION: &str = "Test skill 'foo' for integration coverage.";

impl SkillProvider for StaticFooProvider {
    fn list(&self) -> Vec<SkillMetadata> {
        vec![SkillMetadata {
            name: "foo".into(),
            description: FOO_DESCRIPTION.into(),
            source: SkillSource::User,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }]
    }

    fn get(&self, name: &str) -> Option<SkillContent> {
        if name == "foo" {
            Some(SkillContent {
                name: "foo".into(),
                source: SkillSource::User,
                body: FOO_BODY.into(),
                root: None,
            })
        } else {
            None
        }
    }

    fn is_registered(&self, name: &str) -> bool {
        name == "foo"
    }
}

/// One `ModelEvent` script that streams `text` as a single `TextDelta`
/// followed by `TextBlockCompleted` and a clean `MessageCompleted`.
fn one_text_reply(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: text.into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: text.into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]
}

/// Build a strategy whose H11 injector slot is `Skill`. Everything else
/// matches `HarnessStrategy::default_with_model("mock")`.
fn skill_strategy() -> HarnessStrategy {
    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.context.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    strategy
}

/// Wait for a turn's single `TurnCompleted` broadcast. Since ISSUE#69 part 2
/// was fixed, exactly one `TurnCompleted` is emitted per turn (by H01's FSM
/// transition; the actor's `on_turn_complete` no longer re-records it).
///
/// A subsequent back-to-back `submit_user_text` is safe even before the actor
/// runs `on_turn_complete`: the session actor is single-threaded, so a trigger
/// arriving while `in_flight` is still set is parked in the single-slot
/// `pending_user_input` queue and drained on retirement — never dropped.
async fn wait_for_turn_completed(
    events: &mut tokio::sync::broadcast::Receiver<StreamEvent>,
) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Replay the persisted log and collect every `(skill_name, channel)` pair
/// from `EventPayload::SkillActivated` events, in seq order.
async fn collect_skill_activations(
    store: &Arc<JsonlStore>,
    session_id: SessionId,
) -> Vec<(String, SkillActivationChannel)> {
    let mut out: Vec<(String, SkillActivationChannel)> = Vec::new();
    let mut stream = store.replay(session_id, 0);
    while let Some(evt) = stream.next().await {
        let evt = evt.unwrap();
        if let EventPayload::SkillActivated {
            skill_name,
            channel,
            ..
        } = evt.payload
        {
            out.push((skill_name, channel));
        }
    }
    out
}

/// Replay the persisted log and collect every `SystemPromptInjected.suffix`,
/// in seq order. One per turn (the injector always emits).
async fn collect_injected_suffixes(store: &Arc<JsonlStore>, session_id: SessionId) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stream = store.replay(session_id, 0);
    while let Some(evt) = stream.next().await {
        let evt = evt.unwrap();
        if let EventPayload::SystemPromptInjected { suffix, .. } = evt.payload {
            out.push(suffix);
        }
    }
    out
}

#[tokio::test]
async fn model_sigil_in_turn1_activates_in_turn2() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    // Two scripted replies: turn 1 emits "$foo", turn 2 emits a plain ack.
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(one_text_reply("use $foo please"));
    mock.push_reply(one_text_reply("ack"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    let skills: Arc<dyn SkillProvider> = Arc::new(StaticFooProvider);

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock)
        .tools(tools)
        .skills(Arc::clone(&skills))
        .strategy(skill_strategy())
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // Turn 1: model emits "$foo please" — no activation yet (no prior
    // assistant text to scan).
    handle.submit_user_text("hello").await?;
    assert!(
        wait_for_turn_completed(&mut events).await,
        "turn 1 did not complete within 5s"
    );

    // Turn 2: follow-up user text. SkillInjector scans turn 1's
    // AssistantMessageAppended, finds "$foo", and activates "foo".
    handle.submit_user_text("anything").await?;
    assert!(
        wait_for_turn_completed(&mut events).await,
        "turn 2 did not complete within 5s"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    // Assert: exactly one SkillActivated event, for "foo" via ModelSigil.
    let activations = collect_skill_activations(&store, session_id).await;
    assert_eq!(
        activations.len(),
        1,
        "expected exactly one SkillActivated event, got {activations:?}",
    );
    assert_eq!(activations[0].0, "foo");
    assert!(
        matches!(activations[0].1, SkillActivationChannel::ModelSigil),
        "expected ModelSigil channel, got {:?}",
        activations[0].1,
    );

    // Assert: turn 2's SystemPromptInjected suffix carries the XML-wrapped
    // skill body. Turn 1's suffix only carries the registry block (no
    // activations yet) but never the <skill name="foo"> envelope.
    let suffixes = collect_injected_suffixes(&store, session_id).await;
    assert_eq!(
        suffixes.len(),
        2,
        "expected one SystemPromptInjected per turn, got {} entries",
        suffixes.len(),
    );
    assert!(
        !suffixes[0].contains("<skill name=\"foo\""),
        "turn 1 must not inject the skill body — no prior text was scanned"
    );
    assert!(
        suffixes[1].contains("<skill name=\"foo\""),
        "turn 2 suffix must contain <skill name=\"foo\">; got: {}",
        suffixes[1],
    );
    // Both turns should carry the Available Skills registry block.
    assert!(
        suffixes[0].contains("## Available Skills"),
        "turn 1 suffix missing Available Skills block: {}",
        suffixes[0],
    );
    assert!(
        suffixes[1].contains("## Available Skills"),
        "turn 2 suffix missing Available Skills block: {}",
        suffixes[1],
    );

    Ok(())
}

#[tokio::test]
async fn double_activation_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    // Both turns 1 and 2 emit "$foo". Without dedupe, turn 3 would re-fire
    // "foo" against turn 2's text — but the test stops at 2 turns and the
    // injector also dedupes against prior SkillActivated events, so the
    // total count must be exactly 1 across the whole log.
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(one_text_reply("calling $foo"));
    mock.push_reply(one_text_reply("$foo again"));
    mock.push_reply(one_text_reply("ack"));

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    let skills: Arc<dyn SkillProvider> = Arc::new(StaticFooProvider);

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock)
        .tools(tools)
        .skills(Arc::clone(&skills))
        .strategy(skill_strategy())
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // Turn 1 -> emit "$foo" #1.
    handle.submit_user_text("hi").await?;
    assert!(wait_for_turn_completed(&mut events).await);

    // Turn 2 -> injector activates "foo" (re-derived from turn 1 text),
    // then model emits "$foo" #2.
    handle.submit_user_text("more").await?;
    assert!(wait_for_turn_completed(&mut events).await);

    // Turn 3 -> injector would re-derive "foo" from turn 2 text, but
    // dedupes against the SkillActivated already in the log.
    handle.submit_user_text("again").await?;
    assert!(wait_for_turn_completed(&mut events).await);

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    let activations = collect_skill_activations(&store, session_id).await;
    assert_eq!(
        activations.len(),
        1,
        "double-activation guard failed — expected exactly 1 SkillActivated, got {activations:?}",
    );
    assert_eq!(activations[0].0, "foo");

    Ok(())
}

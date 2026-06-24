#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! ADR-0028: per-session provider injection via `open_session_with` /
//! `update_session`. Patterned on `tests/runtime_submit.rs` (real
//! `MockModelGateway` + `JsonlStore` + `BuiltinToolProvider`).

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, SessionHandle, SessionSpec, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::context::SystemPromptInjectorConfig;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

fn end_turn_reply() -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "ack".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "ack".into(),
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

fn builtin_tools() -> Arc<dyn ToolProvider> {
    Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    )
}

async fn await_turn_completed(handle: &SessionHandle) -> bool {
    let mut events = handle.subscribe();
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

/// Single-skill `SkillProvider` whose name is its constructor argument, so a
/// test can stand up two distinguishable providers ("foo" vs "baz") and assert
/// which one reached the H11 `SkillInjector` via the injected registry block.
struct OneSkillProvider {
    name: &'static str,
}

impl SkillProvider for OneSkillProvider {
    fn list(&self) -> Vec<SkillMetadata> {
        vec![SkillMetadata {
            name: self.name.into(),
            description: format!("Test skill '{}' for per-session injection.", self.name),
            source: SkillSource::User,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }]
    }

    fn get(&self, name: &str) -> Option<SkillContent> {
        (name == self.name).then(|| SkillContent {
            name: self.name.into(),
            source: SkillSource::User,
            body: format!("## {}\n\nbody for {}", self.name, self.name),
            root: None,
        })
    }

    fn is_registered(&self, name: &str) -> bool {
        name == self.name
    }
}

fn one_skill(name: &'static str) -> Arc<dyn SkillProvider> {
    Arc::new(OneSkillProvider { name })
}

/// `HarnessStrategy` whose H11 injector slot is `Skill`, so the registry block
/// (and any activated skill body) lands in each turn's `SystemPromptInjected`.
fn skill_strategy() -> HarnessStrategy {
    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.context.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    strategy
}

/// Wait for a turn's single `TurnCompleted` broadcast. Since ISSUE#69 part 2
/// was fixed, exactly one is emitted per turn (H01's FSM transition); the
/// actor's terminal hook no longer re-records it. A back-to-back
/// `submit_user_text` is still safe: the single-threaded actor parks a trigger
/// arriving mid-retirement in the single-slot `pending_user_input` queue and
/// drains it, so it is never dropped. Mirrors
/// `h11_skill_injection::wait_for_turn_completed`.
async fn wait_turn_completed(events: &mut tokio::sync::broadcast::Receiver<StreamEvent>) -> bool {
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

/// Replay the persisted log and collect every `SystemPromptInjected.suffix` in
/// seq order (the `Skill` injector emits exactly one per turn).
async fn collect_injected_suffixes(store: &Arc<JsonlStore>, session_id: SessionId) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stream = store.replay(session_id, 0);
    while let Some(evt) = stream.next().await {
        let evt = evt.expect("replay event");
        if let EventPayload::SystemPromptInjected { suffix, .. } = evt.payload {
            out.push(suffix);
        }
    }
    out
}

#[tokio::test]
async fn open_session_with_uses_injected_providers() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let mut per_session_strategy = HarnessStrategy::default_with_model("mock");
    per_session_strategy.name = "tenant-acme".into();
    let spec = SessionSpec {
        tools: Some(builtin_tools()),
        strategy: Some(per_session_strategy),
        tenant_id: Some("acme".into()),
        ..Default::default()
    };

    let sid = SessionId::new();
    let handle = runtime.open_session_with(sid, OpenMode::New, spec).await?;
    handle.submit_user_text("hello").await?;

    assert!(await_turn_completed(&handle).await, "turn did not complete");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}

/// The injected `SkillProvider` must reach H11 system-prompt injection, not
/// only the live sigil path. The Runtime is built with NO default skills and a
/// `Skill` injector, so without the open-time wiring fix `build_pipeline_v2`
/// would fail with `MissingSkillProvider` and the session would never open.
/// Post-fix, the spec's provider flows into the pipeline and its skill appears
/// in the injected registry block.
#[tokio::test]
async fn open_session_with_injects_session_skills() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    // No `.skills(...)` on the builder: the only SkillProvider is the one the
    // session injects via SessionSpec.
    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let spec = SessionSpec {
        skills: Some(one_skill("foo")),
        strategy: Some(skill_strategy()),
        ..Default::default()
    };

    let sid = SessionId::new();
    let handle = runtime.open_session_with(sid, OpenMode::New, spec).await?;
    let mut events = handle.subscribe();
    handle.submit_user_text("hello").await?;
    assert!(
        wait_turn_completed(&mut events).await,
        "turn did not complete"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    let suffixes = collect_injected_suffixes(&store, sid).await;
    assert_eq!(
        suffixes.len(),
        1,
        "expected one injection, got {suffixes:?}"
    );
    assert!(
        suffixes[0].contains("## Skills (mandatory)") && suffixes[0].contains("foo"),
        "session-injected skill 'foo' must appear in the registry block; got: {}",
        suffixes[0],
    );
    Ok(())
}

/// A mid-session `update_session` skills swap must rebuild the context pipeline
/// so the new provider reaches H11 injection. Turn 1 runs with "foo"; the swap
/// installs "baz"; turn 2's injected registry block must list "baz" and no
/// longer "foo". Without the `apply_session_update` rebuild, turn 2 would still
/// show "foo" (the open-time pipeline).
#[tokio::test]
async fn update_session_swaps_injected_skills() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .skills(one_skill("foo"))
        .strategy(skill_strategy())
        .build()?;

    let sid = SessionId::new();
    let handle = runtime.open_session(sid, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // Turn 1: default "foo" provider.
    handle.submit_user_text("first").await?;
    assert!(wait_turn_completed(&mut events).await, "turn 1 stalled");

    // Swap to "baz" between turns.
    handle
        .update_session(SessionSpec {
            skills: Some(one_skill("baz")),
            ..Default::default()
        })
        .await?;

    // Turn 2: the rebuilt pipeline must inject "baz", not "foo".
    handle.submit_user_text("second").await?;
    assert!(wait_turn_completed(&mut events).await, "turn 2 stalled");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    let suffixes = collect_injected_suffixes(&store, sid).await;
    assert_eq!(
        suffixes.len(),
        2,
        "expected two injections, got {suffixes:?}"
    );
    assert!(
        suffixes[0].contains("foo"),
        "turn 1 must inject 'foo'; got: {}",
        suffixes[0],
    );
    assert!(
        suffixes[1].contains("baz") && !suffixes[1].contains("foo"),
        "turn 2 must inject the swapped 'baz' and not 'foo'; got: {}",
        suffixes[1],
    );
    Ok(())
}

#[tokio::test]
async fn update_session_then_turn_completes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let sid = SessionId::new();
    let handle = runtime.open_session(sid, OpenMode::New).await?;

    // Swap the tool provider mid-session (no turn in flight yet).
    let spec = SessionSpec {
        tools: Some(builtin_tools()),
        ..Default::default()
    };
    handle.update_session(spec).await?;

    // The next turn must still complete with the swapped provider.
    handle.submit_user_text("hi").await?;
    assert!(
        await_turn_completed(&handle).await,
        "turn did not complete after update"
    );

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}

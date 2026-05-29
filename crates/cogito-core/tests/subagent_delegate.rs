//! Subagent `delegate` acceptance + depth-limit integration tests (ADR-0011).
//!
//! Both tests drive a real `Runtime` whose model is a single shared
//! `MockModelGateway` scripted in GLOBAL FIFO order: every `stream()` call,
//! across the parent session AND every spawned child session, pops the next
//! scripted reply. An in-test `StrategyRegistry` resolves any role to a
//! mock-model strategy so `delegate` is surfaced at every depth.
//!
//! Acceptance: a parent `delegate` call runs a child to completion and the
//! child's verbatim final assistant text round-trips back as the parent's
//! `ToolResult`, with the child session linked child-side via `SessionMeta`.
//!
//! Depth: with `max_depth = 2`, a `delegate` at depth 2 is blocked with a
//! `ToolResult::Error`, the recursion unwinds, and the parent terminates.

// Integration test file: clippy's library-grade lints (`unwrap_used`,
// `expect_used`, `panic`) are conventionally relaxed for tests, mirroring the
// `cogito-mock-model` and `subagent.rs` test modules.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{DelegateToolProvider, OpenMode, Runtime};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ConversationEvent;
use cogito_protocol::SessionMeta;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolResult;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, CompositeToolProvider, NamingPolicy, ReadFile};
use futures::StreamExt as _;

/// In-test registry: every requested role resolves to a mock-model strategy
/// named after the role, allowing all tools (so `delegate` is surfaced in the
/// child's tool surface too, enabling the recursion in the depth test).
struct TestRegistry;
impl StrategyRegistry for TestRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        let mut s = HarnessStrategy::default_with_model("mock");
        s.name = name.to_string();
        s.allowed_tools = ToolFilter::All;
        Ok(s)
    }
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Push one scripted tool-use turn: emit a `delegate{role, input}` block then
/// stop with `ToolUse`. Uses the mock's stable `"c1"` call id.
fn script_delegate_call(mock: &MockModelGateway, role: &str, input: &str) {
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "delegate".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "delegate".into(),
            args: serde_json::json!({ "role": role, "input": input }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);
}

/// Push one scripted text turn: emit `text` then stop with `EndTurn`.
fn script_text(mock: &MockModelGateway, text: &str) {
    mock.push_reply(vec![
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
    ]);
}

/// Build a `Runtime` wired with a composite (`read_file` + `delegate`) tool
/// surface, the shared mock model, and an all-tools parent strategy. The
/// `TestRegistry` resolves child roles; `max_depth` bounds the recursion.
fn build_runtime(
    store: &Arc<JsonlStore>,
    mock: Arc<MockModelGateway>,
    max_depth: u32,
) -> Arc<Runtime> {
    let builtin: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    let delegate: Arc<dyn cogito_protocol::tool::ToolProvider> =
        Arc::new(DelegateToolProvider::new(max_depth));
    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        CompositeToolProvider::new(vec![builtin, delegate], NamingPolicy::Strict)
            .expect("composite provider builds"),
    );

    let mut parent_strategy = HarnessStrategy::default_with_model("mock");
    parent_strategy.allowed_tools = ToolFilter::All;

    Runtime::builder()
        .store(Arc::clone(store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(tools)
        .strategy(parent_strategy)
        .strategy_registry(Arc::new(TestRegistry) as Arc<dyn StrategyRegistry>)
        .build()
        .expect("runtime builds")
}

/// Wait for the PARENT's own terminal turn. Subagent-forwarded terminal
/// events carry `Some(call_id)`; only the parent's own turns are untagged.
async fn wait_parent_done(rx: &mut tokio::sync::broadcast::Receiver<StreamEvent>) -> bool {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Ok(StreamEvent::TurnCompleted {
                    subagent_call_id: None,
                }) => return true,
                Ok(StreamEvent::TurnFailed {
                    subagent_call_id: None,
                    ..
                })
                | Err(_) => return false,
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Replay a session's full persisted log (events with `seq > 0`).
async fn replay_all(store: &Arc<JsonlStore>, id: SessionId) -> Vec<ConversationEvent> {
    let mut s = store.replay(id, 0);
    let mut out = Vec::new();
    while let Some(ev) = s.next().await {
        out.push(ev.expect("event decodes"));
    }
    out
}

/// Scan `<root>/*.jsonl` (the flat `JsonlStore` layout), skipping `parent_id`,
/// and return the first child session's `SessionMeta` (its seq=0
/// `SessionStarted` payload, which `replay(_, 0)` would skip — hence we read
/// the file's first line directly).
fn find_child_meta(root: &std::path::Path, parent_id: SessionId) -> Option<SessionMeta> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(parent_id.to_string().as_str()) {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let Some(first) = content.lines().next() else {
            continue;
        };
        let Ok(ev) = serde_json::from_str::<ConversationEvent>(first) else {
            continue;
        };
        if let EventPayload::SessionStarted { meta } = ev.payload {
            return Some(meta);
        }
    }
    None
}

/// Scan every session log under `<root>/*.jsonl` for a `ToolResultRecorded`
/// whose `ToolResult::Error` message mentions depth (the depth-limit guard).
fn any_depth_error(root: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            if let Ok(ev) = serde_json::from_str::<ConversationEvent>(line) {
                if let EventPayload::ToolResultRecorded {
                    result: ToolResult::Error { message, .. },
                    ..
                } = ev.payload
                {
                    if message.contains("depth") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[tokio::test]
async fn delegate_runs_child_and_returns_final_text() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    // Global FIFO across all sessions:
    //   parent#1 emits delegate -> child#1 emits "CHILD-DONE" (ends)
    //   -> parent#2 (after tool result fed back) emits "parent done" (ends).
    script_delegate_call(&mock, "reviewer", "go");
    script_text(&mock, "CHILD-DONE");
    script_text(&mock, "parent done");

    let runtime = build_runtime(&store, mock, 3);
    let parent_id = SessionId::new();
    let handle = runtime.open_session(parent_id, OpenMode::New).await?;
    let mut rx = handle.subscribe();
    handle.submit_user_text("please review").await?;
    assert!(
        wait_parent_done(&mut rx).await,
        "parent turn did not complete"
    );
    let _ = handle.shutdown(Duration::from_secs(5)).await;

    // The parent received the child's verbatim final text as the tool result.
    let parent_events = replay_all(&store, parent_id).await;
    let result = parent_events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { call_id, result } if call_id == "c1" => {
                Some(result.clone())
            }
            _ => None,
        })
        .expect("delegate ToolResult recorded in parent log");
    match result {
        ToolResult::Output(v) => {
            assert_eq!(v, vec![serde_json::Value::String("CHILD-DONE".into())]);
        }
        other => panic!("expected Output, got {other:?}"),
    }

    // A separate child session exists, linked child-side via SessionMeta.
    let child_meta = find_child_meta(tmp.path(), parent_id).expect("child session file");
    assert_eq!(child_meta.parent_session_id, Some(parent_id));
    assert_eq!(child_meta.parent_call_id.as_deref(), Some("c1"));
    assert_eq!(child_meta.subagent_depth, 1);
    assert_eq!(child_meta.strategy.as_deref(), Some("reviewer"));
    Ok(())
}

#[tokio::test]
async fn delegate_recursion_stops_at_max_depth() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    // max_depth = 2. Recursion: parent(0) -> child(1) -> grandchild(2)=BLOCKED.
    // Global FIFO across all sessions:
    script_delegate_call(&mock, "looper", "x"); // parent#1     (depth 0, spawns child)
    script_delegate_call(&mock, "looper", "x"); // child#1      (depth 1, spawns grandchild)
    script_delegate_call(&mock, "looper", "x"); // grandchild#1 (depth 2 -> ToolResult::Error)
    script_text(&mock, "gc-stop"); // grandchild#2 (after the depth error, ends)
    script_text(&mock, "c-stop"); //  child#2      (after gc result, ends)
    script_text(&mock, "p-stop"); //  parent#2     (after child result, ends)

    let runtime = build_runtime(&store, mock, 2);
    let parent_id = SessionId::new();
    let handle = runtime.open_session(parent_id, OpenMode::New).await?;
    let mut rx = handle.subscribe();
    handle.submit_user_text("recurse").await?;
    assert!(
        wait_parent_done(&mut rx).await,
        "recursion did not terminate"
    );
    let _ = handle.shutdown(Duration::from_secs(5)).await;

    assert!(
        any_depth_error(tmp.path()),
        "expected a depth-limit ToolResult::Error in some session log"
    );
    Ok(())
}

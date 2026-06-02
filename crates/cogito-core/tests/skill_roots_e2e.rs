#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! End-to-end: a skill provider's `skill_roots()` flow into `ExecCtx.skill_roots`
//! (ADR-0032 increment 2), letting `read_file` read a bundled file in place via
//! the read-only skill scope. The model issues a `read_file` for the bundle's
//! absolute path (as it would from the `<skill root="...">` header); the
//! persisted `ToolResultRecorded` must be the file's contents, not an error.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolResult;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// A skill whose bundle lives at `root`; `skill_roots()` exposes it so the
/// Runtime injects it into `ExecCtx.skill_roots`.
struct BundledSkillProvider {
    root: std::path::PathBuf,
}

impl SkillProvider for BundledSkillProvider {
    fn list(&self) -> Vec<SkillMetadata> {
        vec![SkillMetadata {
            name: "demo".into(),
            description: "demo skill with a bundled script".into(),
            source: SkillSource::User,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }]
    }

    fn get(&self, name: &str) -> Option<SkillContent> {
        (name == "demo").then(|| SkillContent {
            name: "demo".into(),
            source: SkillSource::User,
            body: "## demo".into(),
            root: Some(self.root.clone()),
        })
    }

    fn is_registered(&self, name: &str) -> bool {
        name == "demo"
    }

    fn skill_roots(&self) -> Vec<std::path::PathBuf> {
        vec![self.root.clone()]
    }
}

#[tokio::test]
async fn read_file_reads_a_skill_bundle_file_via_skill_roots()
-> Result<(), Box<dyn std::error::Error>> {
    // The skill bundle on disk, outside any workspace.
    let skill = tempfile::tempdir()?;
    std::fs::create_dir(skill.path().join("scripts"))?;
    std::fs::write(skill.path().join("scripts").join("gen.py"), "print('hi')")?;
    let bundled = skill.path().join("scripts").join("gen.py");

    let tmp = tempfile::tempdir()?;
    let store: Arc<dyn cogito_protocol::store::ConversationStore> =
        Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    // Turn: the model reads the bundled file by its absolute path, then ends.
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({ "path": bundled.to_str().unwrap() }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "done".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "done".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        },
    ]);

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    let provider: Arc<dyn SkillProvider> = Arc::new(BundledSkillProvider {
        root: skill.path().to_path_buf(),
    });

    let runtime = Runtime::builder()
        .store(Arc::clone(&store))
        .model(mock)
        .tools(tools)
        .skills(provider)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();
    handle.submit_user_text("read the script").await?;

    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(completed, "turn did not complete within 5s");
    handle.shutdown(Duration::from_secs(5)).await?;

    // The read_file tool result must be the bundled file's contents — proof
    // that skill_roots reached ExecCtx and read_file's skill scope served it.
    let evs: Vec<cogito_protocol::event::ConversationEvent> = store
        .replay(session_id, 0)
        .filter_map(|r| async move { r.ok() })
        .collect()
        .await;
    let result = evs
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { call_id, result } if call_id == "c1" => {
                Some(result.clone())
            }
            _ => None,
        })
        .expect("a ToolResultRecorded for c1");
    match result {
        ToolResult::Output(blocks) => {
            assert_eq!(blocks[0].as_str().expect("text block"), "print('hi')");
        }
        other => panic!("expected Output from the bundled file read, got {other:?}"),
    }
    Ok(())
}

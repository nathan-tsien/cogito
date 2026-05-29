//! Sprint 8 Task 18: end-to-end happy path for [`RunTestsTool`].
//!
//! Shape:
//! 1. Build a `Runtime` wired with `LocalJobManager`, `MockModelGateway`,
//!    and a single-tool [`RunTestsTool`].
//! 2. Switch the process cwd to the `echo_crate` fixture (a one-test
//!    standalone crate sitting outside the workspace). `RunTestsTool`
//!    spawns `cargo nextest run` against the cwd, so this is how we point
//!    it at the fixture without having to plumb a `cwd` argument through
//!    the tool's JSON schema.
//! 3. Submit a user turn. Turn 1's mock reply emits one `tool_use(run_tests)`
//!    block; turn 2's reply emits a final text and `end_turn`.
//! 4. Wait for `TurnCompleted` on the broadcast (60 s ceiling — the
//!    nested cargo build dominates, typically 5-30 s on a developer box).
//!
//! Assertions:
//! - A `ToolResultRecorded` event is persisted with `result =
//!   ToolResult::Output([{...}])`. The output value's `exit_code` field is
//!   `0`, proving the nested `cargo nextest run` invocation succeeded and
//!   the truncated JSON payload made it back through the async loop.
//!
//! Single-threaded note: `std::env::set_current_dir` is a process-global
//! mutation, so this file MUST be run with `--test-threads=1`. There is
//! only one test in the file, but if a future task adds another, mark it
//! `#[serial]` or split into its own file. The Cargo.toml `[[test]]` block
//! also sets `harness = true` (the default), so test-binary args land via
//! `cargo test -- --test-threads=1`.
//!
//! Nested cargo note: invoking `cargo nextest run` from inside another
//! `cargo nextest run` shares the user's `CARGO_HOME` but the nested
//! invocation uses its own (fixture-local) `target/` directory. This
//! avoids lock-file contention on the outer workspace's `target/`. The
//! fixture's first-time build downloads no crates (it has no deps beyond
//! the std-only one-test lib) so the run is bounded by the nested cargo
//! startup + rustc invocation, roughly 5-30 s.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{LocalJobManager, RunTestsTool};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolProvider, ToolResult};
use cogito_store::JsonlStore;
use futures::StreamExt as _;

/// Terminal outcome of the broadcast-stream wait loop. Kept at module
/// scope to avoid `clippy::items_after_statements` inside the test body.
#[derive(Debug)]
enum Outcome {
    /// `StreamEvent::TurnCompleted` arrived. Happy path.
    Completed,
    /// `StreamEvent::TurnFailed` arrived before completion.
    Failed,
    /// The broadcast sender closed before any terminal event.
    StreamClosed,
    /// Wall-clock budget elapsed before any terminal event.
    Timeout,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_tests_against_fixture_crate_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo_crate");
    assert!(
        fixture_dir.join("Cargo.toml").exists(),
        "fixture crate missing at {fixture_dir:?}"
    );

    // Switch cwd for the duration of the test so RunTestsTool runs the
    // fixture's tests. Guard restores cwd on drop even on panic.
    let _cwd = ChdirGuard::push(&fixture_dir)?;

    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let run_tests_tool: Arc<dyn ToolProvider> = Arc::new(RunTestsTool::new(
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>
    ));

    let mock = Arc::new(MockModelGateway::new());
    mock.script_tool_then_text("run_tests", serde_json::json!({}), "tests passed");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn ModelGateway>)
        .tools(run_tests_tool)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    handle.submit_user_text("please run the tests").await?;

    // 60 s ceiling: nested `cargo nextest run` for the echo_crate fixture
    // dominates wall time. Typical cold-cache run is 5-30 s. We do NOT
    // sleep blocking — instead we wait on the broadcast for the canonical
    // terminal `TurnCompleted` event so a fast machine wraps up early.
    // Distinguish the four terminal cases (see `Outcome` below) so a
    // failure mode reports specifically rather than collapsing into one
    // boolean. `match_same_arms` would otherwise want us to merge
    // `TurnFailed` with `Err(_)`, hiding which branch triggered.
    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return Outcome::Completed,
                Ok(StreamEvent::TurnFailed { .. }) => return Outcome::Failed,
                Ok(_) => {}
                Err(_) => return Outcome::StreamClosed,
            }
        }
    })
    .await
    .unwrap_or(Outcome::Timeout);
    assert!(
        matches!(outcome, Outcome::Completed),
        "expected TurnCompleted within 60s, got {outcome:?} — the nested \
         `cargo nextest run` either failed, the broadcast stream closed early, \
         or the async-job loop did not drive the resumed turn"
    );

    handle.shutdown(Duration::from_secs(10)).await?;

    // Both scripted replies must have been consumed: turn 1 produced the
    // `run_tests` tool call, turn 2 produced the final text.
    assert_eq!(
        mock.remaining(),
        0,
        "expected both scripted model replies to be consumed; {} remain",
        mock.remaining()
    );

    // Replay the JSONL log and find the `ToolResultRecorded` event that
    // carries the cargo-nextest output back to the model.
    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };
    let tool_result = log
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("ToolResultRecorded missing from persisted log");

    match tool_result {
        ToolResult::Output(blocks) => {
            assert!(
                !blocks.is_empty(),
                "RunTestsTool must emit a non-empty Output payload"
            );
            let first = &blocks[0];
            let exit = first.get("exit_code").and_then(serde_json::Value::as_i64);
            assert_eq!(
                exit,
                Some(0),
                "fixture tests should pass (exit_code == 0); full payload: {first:?}"
            );
            // Sanity: nextest prints a `Summary` line on success. We don't
            // pin its exact format (nextest version drift), but its
            // presence proves stdout was captured and round-tripped.
            let stdout = first
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let stderr = first
                .get("stderr")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            assert!(
                stdout.contains("echo_passes") || stderr.contains("echo_passes"),
                "expected the `echo_passes` test name in captured output; \
                 stdout={stdout:?} stderr={stderr:?}"
            );
        }
        other => panic!("expected ToolResult::Output, got {other:?}"),
    }

    Ok(())
}

/// RAII guard that switches the process cwd to `dir` on construction and
/// restores the previous cwd on drop. The cwd is a process-global, so
/// callers MUST gate the surrounding test on `--test-threads=1`.
struct ChdirGuard(PathBuf);

impl ChdirGuard {
    fn push(dir: &Path) -> std::io::Result<Self> {
        let prev = std::env::current_dir()?;
        std::env::set_current_dir(dir)?;
        Ok(Self(prev))
    }
}

impl Drop for ChdirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

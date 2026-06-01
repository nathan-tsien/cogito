//! Integration tests for H08 `dispatcher::dispatch`.

use std::sync::Arc;

use cogito_core::harness::dispatcher::{DispatchOutcome, dispatch};
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::tool_resolver::ToolInvocation;
use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::job::JobCompletionEvent;
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_store::JsonlStore;
use cogito_test_fixtures::MockJobManager;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use tokio::sync::{Mutex, broadcast, mpsc};

/// Auxiliary dependencies a dispatch call now needs. The `_store_tmp`
/// field is kept alive so the temporary store directory outlives the
/// recorder.
struct DispatchTestEnv {
    exec: ExecCtx,
    turn: TurnId,
    recorder: Arc<Mutex<StepRecorder>>,
    job_mgr: Arc<MockJobManager>,
    job_tx: mpsc::Sender<JobCompletionEvent>,
    _job_rx: mpsc::Receiver<JobCompletionEvent>,
    _store_tmp: tempfile::TempDir,
}

fn env() -> Result<DispatchTestEnv, Box<dyn std::error::Error>> {
    let session = SessionId::new();
    let turn = TurnId::new();
    let exec = ExecCtx::open_ended(session, turn);
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (tx, _rx) = broadcast::channel(64);
    let recorder = Arc::new(Mutex::new(StepRecorder::new(store, tx, session, 0)));
    let job_mgr = Arc::new(MockJobManager::new());
    let (job_tx, job_rx) = mpsc::channel(8);
    Ok(DispatchTestEnv {
        exec,
        turn,
        recorder,
        job_mgr,
        job_tx,
        _job_rx: job_rx,
        _store_tmp: tmp,
    })
}

#[tokio::test]
async fn sync_tool_returns_sync_result() -> Result<(), Box<dyn std::error::Error>> {
    // `read_file` reads through `ExecCtx.workspace` (ADR-0030/0031): wire a
    // `LocalWorkspace` at a temp dir and address the file by its
    // workspace-relative path.
    let ws = tempfile::tempdir()?;
    std::fs::write(ws.path().join("hi.txt"), "hi")?;
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let inv = ToolInvocation {
        call_id: "c1".into(),
        name: "read_file".into(),
        args: serde_json::json!({ "path": "hi.txt" }),
    };
    let mut e = env()?;
    e.exec.workspace = Some(Arc::new(cogito_tools::workspace::LocalWorkspace::new(
        ws.path(),
    )));
    let outcome = dispatch(
        inv,
        &provider,
        e.exec,
        e.job_mgr.as_ref(),
        &e.job_tx,
        &e.recorder,
        e.turn,
    )
    .await;
    assert!(
        matches!(outcome, DispatchOutcome::SyncResult(ToolResult::Output(_))),
        "expected SyncResult(Output), got {outcome:?}"
    );
    Ok(())
}

#[tokio::test]
async fn unknown_tool_returns_invocation_failed_error() -> Result<(), Box<dyn std::error::Error>> {
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let inv = ToolInvocation {
        call_id: "c1".into(),
        name: "nope".into(),
        args: serde_json::json!({}),
    };
    let e = env()?;
    let outcome = dispatch(
        inv,
        &provider,
        e.exec,
        e.job_mgr.as_ref(),
        &e.job_tx,
        &e.recorder,
        e.turn,
    )
    .await;
    let DispatchOutcome::SyncResult(ToolResult::Error { kind, .. }) = outcome else {
        return Err(format!("expected SyncResult(Error), got {outcome:?}").into());
    };
    assert_eq!(kind, ToolErrorKind::InvocationFailed);
    Ok(())
}

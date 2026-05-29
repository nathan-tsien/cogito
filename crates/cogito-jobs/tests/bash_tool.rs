//! Integration tests for `BashTool` against a real `DirectExecutor`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_jobs::{BashConfig, BashTool, LocalJobManager};
use cogito_protocol::ExecCtx;
use cogito_protocol::command::CommandExecutor;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::job::{JobManager, JobOutcome, LocalJobSubmitter};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use cogito_sandbox::{DirectConfig, DirectExecutor};

fn bash(cfg: BashConfig) -> (BashTool, Arc<LocalJobManager>) {
    bash_with_root(cfg, DirectConfig::default())
}

fn bash_with_root(cfg: BashConfig, dir_cfg: DirectConfig) -> (BashTool, Arc<LocalJobManager>) {
    let executor: Arc<dyn CommandExecutor> = Arc::new(DirectExecutor::new(dir_cfg));
    let job_mgr = LocalJobManager::new();
    let tool = BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        cfg,
    );
    (tool, job_mgr)
}

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

fn exit_code(result: &ToolResult) -> Option<i64> {
    match result {
        ToolResult::Output(blocks) => blocks
            .first()
            .and_then(|v| v.get("exit_code"))
            .and_then(serde_json::Value::as_i64),
        _ => None,
    }
}

#[tokio::test]
async fn sync_success_returns_stdout_and_zero_exit() {
    let (tool, _jm) = bash(BashConfig::default());
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "echo hi" }), ctx())
        .await;
    let InvokeOutcome::Sync(result) = out else {
        panic!("expected Sync");
    };
    assert_eq!(exit_code(&result), Some(0));
    let ToolResult::Output(blocks) = &result else {
        panic!("expected Output");
    };
    let stdout = blocks[0]
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(stdout.contains("hi"), "stdout={stdout:?}");
}

#[tokio::test]
async fn nonzero_exit_is_not_a_tool_error() {
    let (tool, _jm) = bash(BashConfig::default());
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "exit 7" }), ctx())
        .await;
    let InvokeOutcome::Sync(result) = out else {
        panic!("expected Sync");
    };
    assert!(
        !matches!(result, ToolResult::Error { .. }),
        "non-zero exit must surface as Output, not Error"
    );
    assert_eq!(exit_code(&result), Some(7));
}

#[tokio::test]
async fn sync_timeout_surfaces_timeout_error() {
    let cfg = BashConfig {
        sync_timeout_secs: 1,
        ..BashConfig::default()
    };
    let (tool, _jm) = bash(cfg);
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "sleep 30" }), ctx())
        .await;
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = out else {
        panic!("expected Sync Error");
    };
    assert!(matches!(kind, ToolErrorKind::Timeout), "kind={kind:?}");
}

#[tokio::test]
async fn cwd_is_honored() {
    // Root the executor at a known tempdir and create a subdir under it; the
    // `cwd` arg should make `pwd` report that subdir.
    let root = tempfile::tempdir().unwrap();
    let subdir = "work_subdir";
    std::fs::create_dir(root.path().join(subdir)).unwrap();
    let dir_cfg = DirectConfig {
        root: root.path().to_path_buf(),
        ..DirectConfig::default()
    };
    let (tool, _jm) = bash_with_root(BashConfig::default(), dir_cfg);

    let out = tool
        .invoke(
            "bash",
            serde_json::json!({ "command": "pwd", "cwd": subdir }),
            ctx(),
        )
        .await;
    let InvokeOutcome::Sync(ToolResult::Output(blocks)) = out else {
        panic!("expected Sync Output");
    };
    let stdout = blocks[0]
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        stdout.contains(subdir),
        "pwd should report the cwd subdir, got: {stdout:?}"
    );
}

#[tokio::test]
async fn cancel_surfaces_cancelled_error() {
    // Build a ctx whose cancel token we hold, fire it while a long command
    // runs, and assert the sync invoke maps to a Cancelled tool error.
    let (tool, _jm) = bash(BashConfig::default());
    let cancel = tokio_util::sync::CancellationToken::new();
    let run_ctx = ExecCtx {
        cancel: cancel.clone(),
        ..ExecCtx::open_ended(SessionId::new(), TurnId::new())
    };

    let handle = tokio::spawn(async move {
        tool.invoke(
            "bash",
            serde_json::json!({ "command": "sleep 30" }),
            run_ctx,
        )
        .await
    });

    // Give the child a moment to spawn, then cancel.
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();

    let out = handle.await.unwrap();
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = out else {
        panic!("expected Sync Error");
    };
    assert!(matches!(kind, ToolErrorKind::Cancelled), "kind={kind:?}");
}

#[tokio::test]
async fn background_returns_async_and_completes() {
    let (tool, job_mgr) = bash(BashConfig::default());
    let out = tool
        .invoke(
            "bash",
            serde_json::json!({ "command": "echo bg", "background": true }),
            ctx(),
        )
        .await;
    let InvokeOutcome::Async(job_id) = out else {
        panic!("expected Async");
    };

    // Poll the job manager until the job reaches a terminal outcome.
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(o) = job_mgr.result(job_id).await {
                return o;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("job should complete within 10s");

    let JobOutcome::Success { result } = outcome else {
        panic!("expected Success, got {outcome:?}");
    };
    assert_eq!(exit_code(&result), Some(0));
}

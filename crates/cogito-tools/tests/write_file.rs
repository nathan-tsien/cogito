//! `write_file` writes through the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031): confined to the workspace root, structured error when no
//! workspace is wired or the path escapes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::WriteFile;
use cogito_tools::provider::BuiltinTool;
use cogito_tools::workspace::LocalWorkspace;
use tempfile::TempDir;

fn ctx_with_workspace(root: &std::path::Path) -> ExecCtx {
    let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    c.workspace = Some(Arc::new(LocalWorkspace::new(root)));
    c
}

#[tokio::test]
async fn writes_into_workspace_creating_parents() {
    let tmp = TempDir::new().unwrap();
    let ctx = ctx_with_workspace(tmp.path());
    let res = WriteFile
        .invoke(
            serde_json::json!({ "path": "out/a.txt", "content": "hi there" }),
            ctx,
        )
        .await;
    assert!(
        !matches!(res, ToolResult::Error { .. }),
        "expected success, got {res:?}"
    );
    let got = std::fs::read_to_string(tmp.path().join("out").join("a.txt")).unwrap();
    assert_eq!(got, "hi there");
}

#[tokio::test]
async fn errors_when_no_workspace_wired() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new()); // workspace = None
    let res = WriteFile
        .invoke(serde_json::json!({ "path": "a.txt", "content": "x" }), ctx)
        .await;
    assert!(
        matches!(res, ToolResult::Error { .. }),
        "expected Error without a workspace, got {res:?}"
    );
}

#[tokio::test]
async fn rejects_path_escaping_root() {
    let tmp = TempDir::new().unwrap();
    let ctx = ctx_with_workspace(tmp.path());
    let res = WriteFile
        .invoke(
            serde_json::json!({ "path": "../escape.txt", "content": "x" }),
            ctx,
        )
        .await;
    match res {
        ToolResult::Error { kind, .. } => assert_eq!(kind, ToolErrorKind::InvalidArgs),
        other => panic!("expected InvalidArgs error, got {other:?}"),
    }
}

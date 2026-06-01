//! `list_dir` lists a directory through the injected `ExecCtx.workspace`
//! (ADR-0030 / ADR-0031): entries are workspace-relative, sorted, with a
//! trailing `/` marking directories. Confinement and the absent-workspace
//! case surface as structured `ToolResult::Error`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::ListDir;
use cogito_tools::provider::BuiltinTool;
use cogito_tools::workspace::LocalWorkspace;
use tempfile::TempDir;

fn ctx_with_workspace(root: &std::path::Path) -> ExecCtx {
    let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    c.workspace = Some(Arc::new(LocalWorkspace::new(root)));
    c
}

fn text_of(result: &ToolResult) -> String {
    let ToolResult::Output(blocks) = result else {
        panic!("expected Output, got {result:?}");
    };
    blocks[0].as_str().expect("text block").to_string()
}

#[tokio::test]
async fn lists_entries_sorted_with_dir_marker() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("b.txt"), "b").unwrap();
    std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let res = ListDir
        .invoke(
            serde_json::json!({ "path": "" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    // Sorted by name; directories carry a trailing slash.
    assert_eq!(text_of(&res), "a.txt\nb.txt\nsub/");
}

#[tokio::test]
async fn defaults_to_root_when_path_omitted() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("only.txt"), "x").unwrap();
    let res = ListDir
        .invoke(serde_json::json!({}), ctx_with_workspace(tmp.path()))
        .await;
    assert_eq!(text_of(&res), "only.txt");
}

#[tokio::test]
async fn lists_a_subdirectory() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("inner.txt"), "i").unwrap();
    let res = ListDir
        .invoke(
            serde_json::json!({ "path": "sub" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "inner.txt");
}

#[tokio::test]
async fn errors_when_no_workspace_wired() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let res = ListDir.invoke(serde_json::json!({ "path": "" }), ctx).await;
    let ToolResult::Error { message, .. } = res else {
        panic!("expected Error without a workspace, got {res:?}");
    };
    assert!(
        message.contains("workspace"),
        "error should mention the missing workspace, got: {message}"
    );
}

#[tokio::test]
async fn rejects_path_escaping_root() {
    let tmp = TempDir::new().unwrap();
    let res = ListDir
        .invoke(
            serde_json::json!({ "path": "../escape" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    match res {
        ToolResult::Error { kind, .. } => assert_eq!(kind, ToolErrorKind::InvalidArgs),
        other => panic!("expected InvalidArgs error, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_directory_returns_error() {
    let tmp = TempDir::new().unwrap();
    let res = ListDir
        .invoke(
            serde_json::json!({ "path": "does-not-exist" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    match res {
        ToolResult::Error { kind, .. } => assert_eq!(kind, ToolErrorKind::InvocationFailed),
        other => panic!("expected InvocationFailed error, got {other:?}"),
    }
}

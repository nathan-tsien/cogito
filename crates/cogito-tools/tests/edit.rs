//! `edit` performs a read-modify-write string replacement through the injected
//! `ExecCtx.workspace` (ADR-0030 / ADR-0031). Not-found and ambiguous matches
//! are bad input (`InvalidArgs`); confinement, missing file, and the
//! absent-workspace case surface as structured `ToolResult::Error`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::Edit;
use cogito_tools::provider::BuiltinTool;
use cogito_tools::workspace::LocalWorkspace;
use tempfile::TempDir;

fn ctx_with_workspace(root: &std::path::Path) -> ExecCtx {
    let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    c.workspace = Some(Arc::new(LocalWorkspace::new(root)));
    c
}

fn is_error(result: &ToolResult, want: ToolErrorKind) {
    match result {
        ToolResult::Error { kind, .. } => assert_eq!(*kind, want),
        other => panic!("expected {want:?} error, got {other:?}"),
    }
}

#[tokio::test]
async fn replaces_a_unique_occurrence() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "hello world").unwrap();
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "f.txt", "old_string": "world", "new_string": "cogito" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert!(
        !matches!(res, ToolResult::Error { .. }),
        "expected success, got {res:?}"
    );
    let got = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(got, "hello cogito");
}

#[tokio::test]
async fn replace_all_replaces_every_occurrence() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "a a a").unwrap();
    let res = Edit
        .invoke(
            serde_json::json!({
                "path": "f.txt", "old_string": "a", "new_string": "b", "replace_all": true
            }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert!(
        !matches!(res, ToolResult::Error { .. }),
        "expected success, got {res:?}"
    );
    let got = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(got, "b b b");
}

#[tokio::test]
async fn ambiguous_match_without_replace_all_is_invalid_args() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "a a a").unwrap();
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "f.txt", "old_string": "a", "new_string": "b" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
    // The file must be left untouched on a rejected edit.
    let got = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(got, "a a a");
}

#[tokio::test]
async fn old_string_not_found_is_invalid_args() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "f.txt", "old_string": "absent", "new_string": "x" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

#[tokio::test]
async fn errors_when_no_workspace_wired() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "f.txt", "old_string": "a", "new_string": "b" }),
            ctx,
        )
        .await;
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
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "../f.txt", "old_string": "a", "new_string": "b" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

#[tokio::test]
async fn missing_file_returns_invocation_failed() {
    let tmp = TempDir::new().unwrap();
    let res = Edit
        .invoke(
            serde_json::json!({ "path": "nope.txt", "old_string": "a", "new_string": "b" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvocationFailed);
}

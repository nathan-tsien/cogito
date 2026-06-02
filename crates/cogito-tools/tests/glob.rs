//! `glob` matches files under the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031) against a shell-style glob pattern (`**`, `*`, `?`, `{a,b}`,
//! `[..]`), walking the tree via `Workspace::list`. The pattern is matched
//! against each file's path relative to the search root (`path`, default the
//! workspace root); output is the full workspace-relative path, sorted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::Glob;
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

fn is_error(result: &ToolResult, want: ToolErrorKind) {
    match result {
        ToolResult::Error { kind, .. } => assert_eq!(*kind, want),
        other => panic!("expected {want:?} error, got {other:?}"),
    }
}

#[tokio::test]
async fn double_star_matches_recursively() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "").unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("b.rs"), "").unwrap();
    std::fs::write(tmp.path().join("c.txt"), "").unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "**/*.rs" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "a.rs\nsub/b.rs");
}

#[tokio::test]
async fn single_star_does_not_cross_directories() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("c.txt"), "").unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("d.txt"), "").unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "*.txt" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    // `*` must not cross `/`, so only the root-level file matches.
    assert_eq!(text_of(&res), "c.txt");
}

#[tokio::test]
async fn brace_alternation_is_supported() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "").unwrap();
    std::fs::write(tmp.path().join("c.txt"), "").unwrap();
    std::fs::write(tmp.path().join("e.md"), "").unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "*.{rs,txt}" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "a.rs\nc.txt");
}

#[tokio::test]
async fn pattern_is_relative_to_path_scope() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("root.rs"), "").unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("inner.rs"), "").unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "*.rs", "path": "sub" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    // Scoped to `sub`: the pattern matches relative to it, output is the full
    // workspace-relative path; the root-level file is outside the scope.
    assert_eq!(text_of(&res), "sub/inner.rs");
}

#[tokio::test]
async fn no_matches_returns_empty() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "").unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "*.zzz" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "");
}

#[tokio::test]
async fn invalid_glob_is_invalid_args() {
    let tmp = TempDir::new().unwrap();
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "a[" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

#[tokio::test]
async fn errors_when_no_workspace_wired() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let res = Glob
        .invoke(serde_json::json!({ "pattern": "*" }), ctx)
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
    let res = Glob
        .invoke(
            serde_json::json!({ "pattern": "*", "path": "../outside" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

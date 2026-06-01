//! `grep` searches files under the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031) for lines matching a regex. It walks the tree via
//! `Workspace::list` (so it works for any backend), reads each file via
//! `Workspace::read`, skips non-UTF-8 files, and emits `path:line:text` per
//! match, sorted by path then line.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::Grep;
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
async fn finds_matches_with_path_and_line() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "foo\nbar\nfoobar").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "nothing here").unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "foo" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "a.txt:1:foo\na.txt:3:foobar");
}

#[tokio::test]
async fn searches_recursively() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("inner.txt"), "a needle here").unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "needle" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "sub/inner.txt:1:a needle here");
}

#[tokio::test]
async fn respects_path_scope() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("top.txt"), "needle at top").unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("inner.txt"), "needle in sub").unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "needle", "path": "sub" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "sub/inner.txt:1:needle in sub");
}

#[tokio::test]
async fn no_matches_returns_empty() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "absent" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    assert_eq!(text_of(&res), "");
}

#[tokio::test]
async fn skips_non_utf8_files() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("bin.dat"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(tmp.path().join("a.txt"), "match me").unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "match" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    // The binary file is skipped, not an error; the text match is reported.
    assert_eq!(text_of(&res), "a.txt:1:match me");
}

#[tokio::test]
async fn invalid_regex_is_invalid_args() {
    let tmp = TempDir::new().unwrap();
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "[" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

#[tokio::test]
async fn errors_when_no_workspace_wired() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let res = Grep
        .invoke(serde_json::json!({ "pattern": "x" }), ctx)
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
    let res = Grep
        .invoke(
            serde_json::json!({ "pattern": "x", "path": "../outside" }),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    is_error(&res, ToolErrorKind::InvalidArgs);
}

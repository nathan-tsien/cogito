//! Tests for `BuiltinToolProvider` + `read_file`.
//!
//! `read_file` reads through the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031): paths are workspace-relative, absolute / escaping paths are
//! rejected as `InvalidArgs`, and the absent-workspace case is a structured
//! error. The 1 MiB cap + UTF-8 handling stay in the tool.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use cogito_tools::workspace::LocalWorkspace;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use tempfile::TempDir;

/// `ExecCtx` with a `LocalWorkspace` rooted at `root`.
fn ctx_with_workspace(root: &std::path::Path) -> ExecCtx {
    let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    c.workspace = Some(Arc::new(LocalWorkspace::new(root)));
    c
}

#[tokio::test]
async fn read_file_reads_a_file_in_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    std::fs::write(tmp.path().join("note.txt"), "hello cogito\n")?;
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": "note.txt" });
    let outcome = provider
        .invoke("read_file", args, ctx_with_workspace(tmp.path()))
        .await;
    let InvokeOutcome::Sync(ToolResult::Output(blocks)) = outcome else {
        panic!("expected Output, got {outcome:?}");
    };
    assert_eq!(blocks.len(), 1);
    let text = blocks[0].as_str().expect("text block");
    assert_eq!(text, "hello cogito\n");
    Ok(())
}

#[tokio::test]
async fn read_file_errors_when_no_workspace_wired() {
    // workspace = None
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": "note.txt" });
    let outcome = provider.invoke("read_file", args, ctx).await;
    let InvokeOutcome::Sync(ToolResult::Error { message, .. }) = outcome else {
        panic!("expected Error without a workspace, got {outcome:?}");
    };
    assert!(
        message.contains("workspace"),
        "error should mention the missing workspace, got: {message}"
    );
}

#[tokio::test]
async fn read_file_rejects_path_escaping_root() {
    let tmp = TempDir::new().unwrap();
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": "../escape.txt" });
    let outcome = provider
        .invoke("read_file", args, ctx_with_workspace(tmp.path()))
        .await;
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = outcome else {
        panic!("expected Error for an escaping path");
    };
    assert_eq!(kind, ToolErrorKind::InvalidArgs);
}

#[tokio::test]
async fn read_file_missing_path_returns_error() {
    let tmp = TempDir::new().unwrap();
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": "does-not-exist.txt" });
    let outcome = provider
        .invoke("read_file", args, ctx_with_workspace(tmp.path()))
        .await;
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = outcome else {
        panic!("expected Error variant");
    };
    assert_eq!(kind, ToolErrorKind::InvocationFailed);
}

#[tokio::test]
async fn unknown_tool_name_returns_error() {
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let outcome = provider.invoke("nope", serde_json::json!({}), ctx).await;
    let InvokeOutcome::Sync(ToolResult::Error { message, .. }) = outcome else {
        panic!("expected Error variant");
    };
    assert!(message.contains("unknown tool"));
}

#[tokio::test]
async fn read_file_bad_args_returns_invalid_args() {
    let tmp = TempDir::new().unwrap();
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let outcome = provider
        .invoke(
            "read_file",
            serde_json::json!({}),
            ctx_with_workspace(tmp.path()),
        )
        .await;
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = outcome else {
        panic!("expected Error variant");
    };
    assert_eq!(kind, ToolErrorKind::InvalidArgs);
}

#[test]
fn list_returns_registered_descriptors() {
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let desc = provider.list();
    assert_eq!(desc.len(), 1);
    assert_eq!(desc[0].name, "read_file");
}

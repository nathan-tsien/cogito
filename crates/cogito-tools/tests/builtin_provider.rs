//! Tests for `BuiltinToolProvider` + `read_file`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use cogito_protocol::ExecCtx;
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

#[tokio::test]
async fn read_file_reads_a_real_file() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), "hello cogito\n")?;
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": tmp.path().to_str().expect("utf8 tmp path") });
    let outcome = provider.invoke("read_file", args, ctx()).await;
    let InvokeOutcome::Sync(ToolResult::Output(blocks)) = outcome else {
        panic!("expected Output, got {outcome:?}");
    };
    assert_eq!(blocks.len(), 1);
    // ToolResult::text wraps as serde_json::Value::String; as_str() is the Value method.
    let text = blocks[0].as_str().expect("text block");
    assert_eq!(text, "hello cogito\n");
    Ok(())
}

#[tokio::test]
async fn read_file_unknown_path_returns_error() {
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let args = serde_json::json!({ "path": "/this/does/not/exist/12345" });
    let outcome = provider.invoke("read_file", args, ctx()).await;
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
    let outcome = provider.invoke("nope", serde_json::json!({}), ctx()).await;
    let InvokeOutcome::Sync(ToolResult::Error { message, .. }) = outcome else {
        panic!("expected Error variant");
    };
    assert!(message.contains("unknown tool"));
}

#[tokio::test]
async fn read_file_bad_args_returns_invalid_args() {
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let outcome = provider.invoke("read_file", serde_json::json!({}), ctx()).await;
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

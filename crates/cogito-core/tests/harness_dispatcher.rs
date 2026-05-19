//! Integration tests for H08 `dispatcher::dispatch`.

use std::sync::Arc;

use cogito_core::harness::dispatcher::{DispatchOutcome, dispatch};
use cogito_core::harness::tool_resolver::ToolInvocation;
use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

#[tokio::test]
async fn sync_tool_returns_sync_result() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), "hi")?;
    let path = tmp
        .path()
        .to_str()
        .ok_or("temp file path is not valid UTF-8")?;
    let provider = BuiltinToolProvider::builder()
        .with_tool(Arc::new(ReadFile))
        .build();
    let inv = ToolInvocation {
        call_id: "c1".into(),
        name: "read_file".into(),
        args: serde_json::json!({ "path": path }),
    };
    let outcome = dispatch(inv, &provider, ctx()).await;
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
    let outcome = dispatch(inv, &provider, ctx()).await;
    let DispatchOutcome::SyncResult(ToolResult::Error { kind, .. }) = outcome else {
        return Err(format!("expected SyncResult(Error), got {outcome:?}").into());
    };
    assert_eq!(kind, ToolErrorKind::InvocationFailed);
    Ok(())
}

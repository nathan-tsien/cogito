//! Tests for `CompositeToolProvider` and its naming policies.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{InvokeOutcome, ToolProvider, ToolResult};
use cogito_protocol::ExecCtx;
use cogito_tools::{BuiltinToolProvider, CompositeToolProvider, NamingPolicy, ReadFile};

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

#[test]
fn strict_rejects_duplicate_names() {
    let a = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    ) as Arc<dyn ToolProvider>;
    let b = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    ) as Arc<dyn ToolProvider>;
    match CompositeToolProvider::new(vec![a, b], NamingPolicy::Strict) {
        Err(e) => assert!(e.contains("read_file")),
        Ok(_) => panic!("expected Err for duplicate names"),
    }
}

#[test]
fn prefixed_namespaces_tools() {
    let a = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    ) as Arc<dyn ToolProvider>;
    let composite = CompositeToolProvider::new(
        vec![a],
        NamingPolicy::Prefixed(vec!["builtin".into()]),
    )
    .expect("build ok");
    let names: Vec<_> = composite.list().into_iter().map(|d| d.name).collect();
    assert_eq!(names, vec!["builtin/read_file"]);
}

#[tokio::test]
async fn prefixed_invokes_through_namespace() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), "ok")?;
    let a = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    ) as Arc<dyn ToolProvider>;
    let composite =
        CompositeToolProvider::new(vec![a], NamingPolicy::Prefixed(vec!["b".into()]))
            .expect("build ok");
    let outcome = composite
        .invoke(
            "b/read_file",
            serde_json::json!({ "path": tmp.path().to_str().expect("utf8 path") }),
            ctx(),
        )
        .await;
    assert!(matches!(outcome, InvokeOutcome::Sync(ToolResult::Output(_))));
    Ok(())
}

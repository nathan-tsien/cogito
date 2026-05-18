//! Tests for tool-layer contract types.
//!
//! These tests pin down serde stability and enum coverage. They run as
//! part of `cargo nextest run -p cogito-protocol`.

use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolResult,
};

#[test]
fn execution_class_serde_roundtrip() -> serde_json::Result<()> {
    for variant in [
        ExecutionClass::AlwaysSync,
        ExecutionClass::AlwaysAsync,
        ExecutionClass::Adaptive,
    ] {
        let json = serde_json::to_string(&variant)?;
        let back: ExecutionClass = serde_json::from_str(&json)?;
        assert_eq!(variant, back);
    }
    Ok(())
}

#[test]
fn tool_descriptor_round_trips() -> serde_json::Result<()> {
    let descriptor = ToolDescriptor {
        name: "read_file".into(),
        description: "Read a file from the workspace".into(),
        schema: serde_json::json!({ "type": "object" }),
        execution_class: ExecutionClass::AlwaysSync,
        outputs_model_visible_multimodal: false,
    };
    let json = serde_json::to_string(&descriptor)?;
    let back: ToolDescriptor = serde_json::from_str(&json)?;
    assert_eq!(descriptor.name, back.name);
    assert_eq!(descriptor.execution_class, back.execution_class);
    assert_eq!(descriptor, back);
    Ok(())
}

#[test]
#[allow(deprecated)]
fn invoke_outcome_distinguishes_sync_and_async() {
    let sync_out = InvokeOutcome::Sync(ToolResult::Output(vec![]));
    assert!(matches!(sync_out, InvokeOutcome::Sync(_)));
    // TODO(Task 8): replace JobIdStub with cogito_protocol::job::JobId
    // once the job module lands. The stub is a temporary u64 wrapper.
    let async_out = InvokeOutcome::Async(cogito_protocol::tool::JobIdStub::default());
    assert!(matches!(async_out, InvokeOutcome::Async(_)));
}

#[test]
fn tool_error_kind_serde_covers_all_variants() -> serde_json::Result<()> {
    use ToolErrorKind::*;
    for kind in [
        InvalidArgs,
        InvocationFailed,
        ToolPanicked,
        Cancelled,
        Timeout,
        JobStateLost,
        AsyncFailed,
    ] {
        let json = serde_json::to_string(&kind)?;
        let back: ToolErrorKind = serde_json::from_str(&json)?;
        assert_eq!(kind, back);
    }
    Ok(())
}

#[test]
fn tool_result_output_round_trips() -> serde_json::Result<()> {
    let out = ToolResult::text("ok");
    let json = serde_json::to_string(&out)?;
    let back: ToolResult = serde_json::from_str(&json)?;
    assert!(matches!(back, ToolResult::Output(ref v) if v.len() == 1));
    Ok(())
}

#[test]
fn tool_result_error_round_trips() -> serde_json::Result<()> {
    let err = ToolResult::Error {
        kind: ToolErrorKind::Timeout,
        message: "deadline exceeded".into(),
        retryable: false,
    };
    let json = serde_json::to_string(&err)?;
    let back: ToolResult = serde_json::from_str(&json)?;
    assert!(matches!(back, ToolResult::Error { kind: ToolErrorKind::Timeout, .. }));
    Ok(())
}

#[test]
#[allow(deprecated)]
fn invoke_outcome_serde_roundtrips() -> serde_json::Result<()> {
    let sync_out = InvokeOutcome::Sync(ToolResult::text("ok"));
    let json = serde_json::to_string(&sync_out)?;
    let back: InvokeOutcome = serde_json::from_str(&json)?;
    assert!(matches!(back, InvokeOutcome::Sync(_)));

    let async_out = InvokeOutcome::Async(cogito_protocol::tool::JobIdStub(42));
    let json = serde_json::to_string(&async_out)?;
    let back: InvokeOutcome = serde_json::from_str(&json)?;
    assert!(matches!(back, InvokeOutcome::Async(stub) if stub.0 == 42));
    Ok(())
}

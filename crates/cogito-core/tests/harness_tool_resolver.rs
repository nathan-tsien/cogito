//! Integration tests for H07 Tool Call Resolver.

use cogito_core::harness::tool_resolver::{resolve, ResolvedCall};
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};

fn read_file_desc() -> ToolDescriptor {
    ToolDescriptor {
        name: "read_file".into(),
        description: "Read file".into(),
        schema: serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
            "additionalProperties": false,
        }),
        execution_class: ExecutionClass::AlwaysSync,
        outputs_model_visible_multimodal: false,
    }
}

#[test]
fn valid_call_resolves_ok() {
    let surface = vec![read_file_desc()];
    let r = resolve(
        "c1",
        "read_file",
        serde_json::json!({ "path": "/tmp/x" }),
        &surface,
    );
    assert!(
        matches!(r, ResolvedCall::Ok(ref inv) if inv.call_id == "c1" && inv.name == "read_file")
    );
}

#[test]
fn unknown_tool_returns_error() {
    let surface = vec![read_file_desc()];
    let r = resolve("c1", "nope", serde_json::json!({}), &surface);
    let ResolvedCall::Error(ToolResult::Error { message, .. }) = r else {
        panic!("expected ResolvedCall::Error, got Ok");
    };
    assert!(message.contains("not available"));
}

#[test]
fn missing_required_field_returns_invalid_args() {
    let surface = vec![read_file_desc()];
    let r = resolve("c1", "read_file", serde_json::json!({}), &surface);
    let ResolvedCall::Error(ToolResult::Error { kind, .. }) = r else {
        panic!("expected ResolvedCall::Error, got Ok");
    };
    assert_eq!(kind, ToolErrorKind::InvalidArgs);
}

#[test]
fn extra_field_rejected_by_strict_schema() {
    let surface = vec![read_file_desc()];
    let r = resolve(
        "c1",
        "read_file",
        serde_json::json!({ "path": "/", "extra": 1 }),
        &surface,
    );
    let ResolvedCall::Error(ToolResult::Error { kind, .. }) = r else {
        panic!("expected ResolvedCall::Error, got Ok");
    };
    assert_eq!(kind, ToolErrorKind::InvalidArgs);
}

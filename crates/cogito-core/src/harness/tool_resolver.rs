//! H07 Tool Call Resolver — pure validation of one model-emitted tool
//! call against the active turn's tool surface.
//!
//! `ToolInvocation` and `ResolvedCall` are harness-internal (not in
//! `cogito-protocol`).
//!
//! ## JSON Schema validation
//!
//! Uses `jsonschema 0.18` via `JSONSchema::compile` (Draft 7 by default).
//! The `draft202012` feature is not enabled in the workspace, so the
//! `jsonschema::draft202012::new` constructor is unavailable; `JSONSchema::compile`
//! provides equivalent validation for the strict object schemas used by tools.

use cogito_protocol::tool::{ToolDescriptor, ToolErrorKind, ToolResult};

/// A validated tool call ready for dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    /// Opaque identifier matching the model's tool call.
    pub call_id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// Validated arguments JSON.
    pub args: serde_json::Value,
}

/// Outcome of `resolve()`. `Error` wraps a ready-to-record
/// `ToolResult::Error` that should be fed back to the model.
#[derive(Debug, Clone)]
pub enum ResolvedCall {
    /// Validation passed; the call is safe to dispatch.
    Ok(ToolInvocation),
    /// Validation failed; the wrapped error should be recorded and fed
    /// back to the model.
    Error(ToolResult),
}

/// Validate one tool call. `args` is the JSON object the model emitted
/// (already parsed by the gateway). Surface comes from H05.
pub fn resolve(
    call_id: &str,
    name: &str,
    args: serde_json::Value,
    surface: &[ToolDescriptor],
) -> ResolvedCall {
    let Some(desc) = surface.iter().find(|d| d.name == name) else {
        let names: Vec<&str> = surface.iter().map(|d| d.name.as_str()).collect();
        return ResolvedCall::Error(ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            message: format!(
                "tool `{name}` is not available this turn. available: {names:?}"
            ),
            retryable: false,
        });
    };

    // Compile the tool's JSON Schema. jsonschema 0.18 exposes
    // `JSONSchema::compile` for Draft 7 (the workspace does not enable
    // the `draft202012` feature, so `draft202012::new` is unavailable).
    let validator = match jsonschema::JSONSchema::compile(&desc.schema) {
        Ok(v) => v,
        Err(e) => {
            return ResolvedCall::Error(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("schema compile failed for `{name}`: {e}"),
                retryable: false,
            });
        }
    };

    if let Err(errs) = validator.validate(&args) {
        let detail = errs.map(|e| format!("{e}")).collect::<Vec<_>>().join("; ");
        return ResolvedCall::Error(ToolResult::Error {
            kind: ToolErrorKind::InvalidArgs,
            message: format!("args for `{name}` failed validation: {detail}"),
            retryable: false,
        });
    }

    ResolvedCall::Ok(ToolInvocation {
        call_id: call_id.into(),
        name: name.into(),
        args,
    })
}

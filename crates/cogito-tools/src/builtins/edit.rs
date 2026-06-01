//! `edit` — read-modify-write string replacement through the injected
//! `ExecCtx.workspace` (ADR-0030 / ADR-0031). Reads the file, replaces
//! `old_string` with `new_string` (a unique match unless `replace_all`), and
//! writes it back. Not-found / ambiguous matches are bad input the model can
//! fix (`InvalidArgs`); confinement, missing file, non-UTF-8, and the
//! absent-workspace case surface as structured `ToolResult::Error`.

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::WorkspaceError;
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Stateless editor; `Edit::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct Edit;

#[derive(Debug, Deserialize)]
struct Args {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl BuiltinTool for Edit {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "edit".into(),
            description: "Replace a string in a UTF-8 text file in the workspace. By default `old_string` must match exactly once; set `replace_all` to replace every occurrence. The path is relative to the workspace root.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path relative to the workspace root. Absolute paths and paths escaping the root are rejected."
                    },
                    "old_string": { "type": "string", "description": "Exact text to replace." },
                    "new_string": { "type": "string", "description": "Replacement text." },
                    "replace_all": { "type": "boolean", "description": "Replace every occurrence instead of requiring a unique match." }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult {
        let Args {
            path,
            old_string,
            new_string,
            replace_all,
        } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("edit args: {e}"),
                    retryable: false,
                };
            }
        };
        if old_string.is_empty() {
            return ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: "edit: old_string must not be empty".into(),
                retryable: false,
            };
        }
        let Some(workspace) = ctx.workspace else {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: "edit: no workspace is configured for this session".into(),
                retryable: false,
            };
        };
        let bytes = match workspace.read(&path).await {
            Ok(b) => b,
            Err(e @ WorkspaceError::PathEscapesRoot(_)) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("edit: {e}"),
                    retryable: false,
                };
            }
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("edit: {e}"),
                    retryable: false,
                };
            }
        };
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("edit: non-utf8 content in {path}: {e}"),
                    retryable: false,
                };
            }
        };
        let count = content.matches(old_string.as_str()).count();
        if count == 0 {
            return ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("edit: old_string not found in {path}"),
                retryable: false,
            };
        }
        if count > 1 && !replace_all {
            return ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!(
                    "edit: old_string matches {count} times in {path}; pass replace_all:true or \
                     provide a more specific old_string"
                ),
                retryable: false,
            };
        }
        let new_content = if replace_all {
            content.replace(old_string.as_str(), &new_string)
        } else {
            content.replacen(old_string.as_str(), &new_string, 1)
        };
        match workspace.write(&path, new_content.as_bytes()).await {
            Ok(()) => ToolResult::text(format!("edited {path}: {count} replacement(s)")),
            Err(e @ WorkspaceError::PathEscapesRoot(_)) => ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("edit: {e}"),
                retryable: false,
            },
            Err(e) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("edit: {e}"),
                retryable: false,
            },
        }
    }
}

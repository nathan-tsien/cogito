//! `write_file` — create or overwrite a UTF-8 text file through the injected
//! `ExecCtx.workspace` (ADR-0030 / ADR-0031). All paths are relative to the
//! session workspace root; escapes and the absent-workspace case surface as
//! structured `ToolResult::Error`.

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::WorkspaceError;
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Stateless writer; `WriteFile::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct WriteFile;

#[derive(Debug, Deserialize)]
struct Args {
    path: String,
    content: String,
}

#[async_trait]
impl BuiltinTool for WriteFile {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "write_file".into(),
            description: "Create or overwrite a UTF-8 text file in the workspace, creating parent directories. The path is relative to the workspace root.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path relative to the workspace root. Absolute paths and paths escaping the root are rejected."
                    },
                    "content": {
                        "type": "string",
                        "description": "UTF-8 text to write."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult {
        let Args { path, content } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("write_file args: {e}"),
                    retryable: false,
                };
            }
        };
        let Some(workspace) = ctx.workspace else {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: "write_file: no workspace is configured for this session".into(),
                retryable: false,
            };
        };
        match workspace.write(&path, content.as_bytes()).await {
            Ok(()) => ToolResult::text(format!("wrote {} bytes to {path}", content.len())),
            // A path that escapes the root is bad input the model can fix.
            Err(e @ WorkspaceError::PathEscapesRoot(_)) => ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("write_file: {e}"),
                retryable: false,
            },
            Err(e) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("write_file: {e}"),
                retryable: false,
            },
        }
    }
}

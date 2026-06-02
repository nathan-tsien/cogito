//! `read_file` — UTF-8 text file reader through the injected
//! `ExecCtx.workspace` (ADR-0030 / ADR-0031), with a 1 MiB cap per the v0.1
//! class B truncation compromise (see ARCHITECTURE.md §"Tool execution
//! classes"). Paths are workspace-relative; absolute / escaping paths and the
//! absent-workspace case surface as structured `ToolResult::Error`.

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::WorkspaceError;
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Cap applied per call. Files larger than this are truncated.
pub const MAX_BYTES: usize = 1 << 20;

/// Stateless reader; `ReadFile::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadFile;

#[derive(Debug, Deserialize)]
struct Args {
    path: String,
}

#[async_trait]
impl BuiltinTool for ReadFile {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "read_file".into(),
            description: "Read a UTF-8 text file from the workspace. The path is relative to the workspace root; absolute paths and paths escaping the root are rejected. Returns up to 1 MiB; longer files are truncated with a marker.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path relative to the workspace root. Absolute paths and paths escaping the root are rejected."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult {
        let Args { path } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("read_file args: {e}"),
                    retryable: false,
                };
            }
        };
        // Read-only skill-root scope (ADR-0032): a path resolving within a
        // registered skill bundle is read in place from the host, bypassing the
        // workspace. Checked before the workspace branch so the model can read
        // bundled files by the absolute root from the `<skill root="...">`
        // header.
        let mut bytes =
            if let Some(abs) = crate::skill_scope::resolve_in_roots(&path, &ctx.skill_roots) {
                match tokio::fs::read(&abs).await {
                    Ok(b) => b,
                    Err(e) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvocationFailed,
                            message: format!("read_file: cannot read {path}: {e}"),
                            retryable: false,
                        };
                    }
                }
            } else {
                let Some(workspace) = ctx.workspace else {
                    return ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: "read_file: no workspace is configured for this session".into(),
                        retryable: false,
                    };
                };
                match workspace.read(&path).await {
                    Ok(b) => b,
                    // A path that escapes the root is bad input the model can fix.
                    Err(e @ WorkspaceError::PathEscapesRoot(_)) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvalidArgs,
                            message: format!("read_file: {e}"),
                            retryable: false,
                        };
                    }
                    Err(e) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvocationFailed,
                            message: format!("read_file: {e}"),
                            retryable: false,
                        };
                    }
                }
            };
        let truncated = bytes.len() > MAX_BYTES;
        if truncated {
            bytes.truncate(MAX_BYTES);
        }
        match String::from_utf8(bytes) {
            Ok(mut s) => {
                if truncated {
                    s.push_str("\n\n[truncated at 1 MiB]\n");
                }
                ToolResult::text(s)
            }
            Err(e) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("read_file: non-utf8 content in {path}: {e}"),
                retryable: false,
            },
        }
    }
}

//! `read_file` — UTF-8 text file reader with a 1 MiB cap per v0.1 class B
//! truncation compromise (see ARCHITECTURE.md §"Tool execution classes").

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
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
            description: "Read a UTF-8 text file. Returns up to 1 MiB; longer files are truncated with a marker.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path or path relative to the workspace root."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, _ctx: ExecCtx) -> ToolResult {
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
        match tokio::fs::read(&path).await {
            Ok(mut bytes) => {
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
            Err(e) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("read_file: cannot read {path}: {e}"),
                retryable: false,
            },
        }
    }
}

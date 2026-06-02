//! `list_dir` — list a directory through the injected `ExecCtx.workspace`
//! (ADR-0030 / ADR-0031). Paths are workspace-relative (`""` lists the root);
//! entries are returned sorted, one per line, with a trailing `/` marking
//! directories. Confinement and the absent-workspace case surface as
//! structured `ToolResult::Error`.

use std::path::Path;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::{DirEntry, WorkspaceError};
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// List a host directory directly (the read-only skill-root branch, ADR-0032).
async fn read_dir_entries(dir: &Path) -> std::io::Result<Vec<DirEntry>> {
    let mut rd = tokio::fs::read_dir(dir).await?;
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        out.push(DirEntry { name, is_dir });
    }
    Ok(out)
}

/// Stateless lister; `ListDir::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct ListDir;

#[derive(Debug, Deserialize)]
struct Args {
    #[serde(default)]
    path: String,
}

#[async_trait]
impl BuiltinTool for ListDir {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_dir".into(),
            description: "List the immediate entries of a directory in the workspace. The path is relative to the workspace root (empty lists the root). Entries are sorted; directories carry a trailing `/`.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory relative to the workspace root; empty or omitted lists the root. Absolute paths and paths escaping the root are rejected."
                    }
                },
                "required": [],
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
                    message: format!("list_dir args: {e}"),
                    retryable: false,
                };
            }
        };
        // Read-only skill-root scope (ADR-0032): a directory within a
        // registered skill bundle is listed in place from the host, bypassing
        // the workspace. Checked before the workspace branch.
        let mut entries =
            if let Some(abs) = crate::skill_scope::resolve_in_roots(&path, &ctx.skill_roots) {
                match read_dir_entries(&abs).await {
                    Ok(e) => e,
                    Err(e) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvocationFailed,
                            message: format!("list_dir: cannot list {path}: {e}"),
                            retryable: false,
                        };
                    }
                }
            } else {
                let Some(workspace) = ctx.workspace else {
                    return ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: "list_dir: no workspace is configured for this session".into(),
                        retryable: false,
                    };
                };
                match workspace.list(&path).await {
                    Ok(e) => e,
                    // A path that escapes the root is bad input the model can fix.
                    Err(e @ WorkspaceError::PathEscapesRoot(_)) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvalidArgs,
                            message: format!("list_dir: {e}"),
                            retryable: false,
                        };
                    }
                    Err(e) => {
                        return ToolResult::Error {
                            kind: ToolErrorKind::InvocationFailed,
                            message: format!("list_dir: {e}"),
                            retryable: false,
                        };
                    }
                }
            };
        // Deterministic order: sort by name (the seam leaves order unspecified).
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let listing = entries
            .iter()
            .map(|e| {
                if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        ToolResult::text(listing)
    }
}

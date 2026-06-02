//! `grep` — search files under the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031) for lines matching a regular expression. The tree is walked via
//! `Workspace::list` (not the host filesystem, so any backend works), each
//! file is read via `Workspace::read`, non-UTF-8 files are skipped, and
//! matches are emitted as `path:line:text`, sorted by path then line. A bad
//! regex / escaping path is bad input (`InvalidArgs`); the absent-workspace
//! case surfaces as a structured error.

use std::fmt::Write as _;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::{Workspace, WorkspaceError};
use regex::Regex;
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Maximum matches returned per call; beyond this a truncation marker is
/// appended (mirrors `read_file`'s byte cap, keeps model context bounded).
pub const MAX_MATCHES: usize = 1000;

/// Stateless searcher; `Grep::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct Grep;

#[derive(Debug, Deserialize)]
struct Args {
    pattern: String,
    #[serde(default)]
    path: String,
}

#[async_trait]
impl BuiltinTool for Grep {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "grep".into(),
            description: "Search the workspace for lines matching a regular expression. Recurses from `path` (relative to the workspace root; empty searches the whole workspace). Returns `path:line:text` per match. Binary (non-UTF-8) files are skipped.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression to match against each line." },
                    "path": {
                        "type": "string",
                        "description": "Directory to search, relative to the workspace root; empty or omitted searches the whole workspace. Absolute paths and paths escaping the root are rejected."
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult {
        let Args { pattern, path } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("grep args: {e}"),
                    retryable: false,
                };
            }
        };
        let re = match Regex::new(&pattern) {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("grep: invalid regex: {e}"),
                    retryable: false,
                };
            }
        };
        let Some(workspace) = ctx.workspace else {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: "grep: no workspace is configured for this session".into(),
                retryable: false,
            };
        };
        let files = match collect_files(workspace.as_ref(), &path).await {
            Ok(f) => f,
            // A path that escapes the root is bad input the model can fix.
            Err(e @ WorkspaceError::PathEscapesRoot(_)) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("grep: {e}"),
                    retryable: false,
                };
            }
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("grep: {e}"),
                    retryable: false,
                };
            }
        };
        let mut out: Vec<String> = Vec::new();
        let mut truncated = false;
        'files: for file in files {
            // A file that vanished mid-walk or can't be read is skipped, not
            // fatal — grep is best-effort over the tree it discovered.
            let Ok(bytes) = workspace.read(&file).await else {
                continue;
            };
            // Binary (non-UTF-8) files are skipped silently.
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    out.push(format!("{file}:{}:{line}", i + 1));
                    if out.len() >= MAX_MATCHES {
                        truncated = true;
                        break 'files;
                    }
                }
            }
        }
        let mut text = out.join("\n");
        if truncated {
            let _ = write!(text, "\n[grep truncated at {MAX_MATCHES} matches]");
        }
        ToolResult::text(text)
    }
}

/// Collect every file under `start` (workspace-relative; `""` is the root) by
/// walking `Workspace::list`, returning workspace-relative paths sorted for a
/// deterministic match order. Walking the seam (not the host filesystem) keeps
/// `grep` correct for any `Workspace` backend, including the sandboxed
/// multi-tenant profile.
async fn collect_files(ws: &dyn Workspace, start: &str) -> Result<Vec<String>, WorkspaceError> {
    let mut dirs = vec![start.to_string()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        let entries = ws.list(&dir).await?;
        for entry in entries {
            let child = if dir.is_empty() {
                entry.name
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_dir {
                dirs.push(child);
            } else {
                files.push(child);
            }
        }
    }
    files.sort();
    Ok(files)
}

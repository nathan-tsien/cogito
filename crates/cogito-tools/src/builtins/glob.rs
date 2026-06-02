//! `glob` — find files under the injected `ExecCtx.workspace` (ADR-0030 /
//! ADR-0031) whose path matches a shell-style glob pattern. The tree is walked
//! via `Workspace::list` (not the host filesystem, so any backend works); the
//! pattern is matched against each file's path relative to the search root
//! (`path`, default the workspace root), and the full workspace-relative path
//! is emitted, sorted. A bad pattern / escaping path is bad input
//! (`InvalidArgs`); the absent-workspace case surfaces as a structured error.

use std::fmt::Write as _;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::workspace::WorkspaceError;
use globset::GlobBuilder;
use serde::Deserialize;

use crate::builtins::walk::collect_files;
use crate::provider::BuiltinTool;

/// Maximum paths returned per call; beyond this a truncation marker is
/// appended (mirrors `grep`'s match cap, keeps model context bounded).
pub const MAX_MATCHES: usize = 1000;

/// Stateless matcher; `Glob::default()` yields the canonical instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct Glob;

#[derive(Debug, Deserialize)]
struct Args {
    pattern: String,
    #[serde(default)]
    path: String,
}

#[async_trait]
impl BuiltinTool for Glob {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "glob".into(),
            description: "Find files in the workspace whose path matches a shell glob pattern (supports `**`, `*`, `?`, `{a,b}`, `[..]`). The pattern is relative to `path` (relative to the workspace root; empty searches the whole workspace). Returns matching workspace-relative paths, sorted.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Shell glob, e.g. `**/*.rs`. `*` does not cross `/`; `**` does." },
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
                    message: format!("glob args: {e}"),
                    retryable: false,
                };
            }
        };
        // `literal_separator(true)` makes `*`/`?` stop at `/`; only `**`
        // crosses directories — standard shell-glob semantics.
        let matcher = match GlobBuilder::new(&pattern).literal_separator(true).build() {
            Ok(g) => g.compile_matcher(),
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("glob: invalid pattern: {e}"),
                    retryable: false,
                };
            }
        };
        let Some(workspace) = ctx.workspace else {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: "glob: no workspace is configured for this session".into(),
                retryable: false,
            };
        };
        let files = match collect_files(workspace.as_ref(), &path).await {
            Ok(f) => f,
            // A path that escapes the root is bad input the model can fix.
            Err(e @ WorkspaceError::PathEscapesRoot(_)) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("glob: {e}"),
                    retryable: false,
                };
            }
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("glob: {e}"),
                    retryable: false,
                };
            }
        };
        // The pattern is matched against each file's path relative to the
        // search root; the full workspace-relative path is what we emit.
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let mut out: Vec<String> = Vec::new();
        let mut truncated = false;
        for file in files {
            let relative = file.strip_prefix(&prefix).unwrap_or(&file);
            if matcher.is_match(relative) {
                out.push(file.clone());
                if out.len() >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
            }
        }
        let mut text = out.join("\n");
        if truncated {
            let _ = write!(text, "\n[glob truncated at {MAX_MATCHES} matches]");
        }
        ToolResult::text(text)
    }
}

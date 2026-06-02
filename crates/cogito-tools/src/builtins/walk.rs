//! Shared workspace tree walk for the recursive file tools (`grep`, `glob`).
//!
//! Walking `Workspace::list` rather than the host filesystem keeps these tools
//! correct for any `Workspace` backend, including the future sandboxed
//! multi-tenant profile — confinement is preserved by construction.

use cogito_protocol::workspace::{Workspace, WorkspaceError};

/// Collect every file under `start` (workspace-relative; `""` is the root) by
/// breadth/depth walking `Workspace::list`, returning workspace-relative file
/// paths sorted for a deterministic order. Directory listings propagate their
/// `WorkspaceError` (the caller maps `PathEscapesRoot` to `InvalidArgs`).
pub(crate) async fn collect_files(
    ws: &dyn Workspace,
    start: &str,
) -> Result<Vec<String>, WorkspaceError> {
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

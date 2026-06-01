//! `LocalWorkspace` — host-filesystem [`Workspace`] rooted at a directory
//! (the session cwd for the Local profile). See ADR-0030.
//!
//! Path confinement is lexical: input paths are normalized component-by-
//! component and any `..` that climbs above the root, or any absolute path,
//! is rejected before touching the filesystem. This works for writes to
//! not-yet-existing paths (no `canonicalize` required).

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use cogito_protocol::workspace::{DirEntry, Workspace, WorkspaceError};

/// Host-filesystem [`Workspace`] confined to `root`.
#[derive(Debug, Clone)]
pub struct LocalWorkspace {
    root: PathBuf,
}

impl LocalWorkspace {
    /// Construct a workspace rooted at `root`. All operations are confined to
    /// this directory; `root` need not exist yet (a `write` creates it).
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Resolve a workspace-relative path to an absolute host path, rejecting
    /// anything that escapes the root.
    //
    // TODO(ADR-0030 Q4): this is a lexical guard; it does not follow symlinks.
    // A symlink inside the workspace pointing outside it would not be caught.
    // Harden (canonicalize + re-check prefix, deny escaping symlinks) when the
    // SaaS `SandboxWorkspace` lands and roots become untrusted.
    fn resolve(&self, path: &str) -> Result<PathBuf, WorkspaceError> {
        let mut stack: Vec<&std::ffi::OsStr> = Vec::new();
        for comp in Path::new(path).components() {
            match comp {
                Component::Normal(c) => stack.push(c),
                Component::CurDir => {}
                Component::ParentDir => {
                    if stack.pop().is_none() {
                        return Err(WorkspaceError::PathEscapesRoot(path.to_string()));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(WorkspaceError::PathEscapesRoot(path.to_string()));
                }
            }
        }
        let mut out = self.root.clone();
        out.extend(stack);
        Ok(out)
    }
}

#[async_trait]
impl Workspace for LocalWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    async fn read(&self, path: &str) -> Result<Vec<u8>, WorkspaceError> {
        let abs = self.resolve(path)?;
        match tokio::fs::read(&abs).await {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(WorkspaceError::NotFound(path.to_string()))
            }
            Err(e) => Err(WorkspaceError::Io(e.to_string())),
        }
    }

    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), WorkspaceError> {
        let abs = self.resolve(path)?;
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| WorkspaceError::Io(e.to_string()))?;
        }
        tokio::fs::write(&abs, bytes)
            .await
            .map_err(|e| WorkspaceError::Io(e.to_string()))
    }

    async fn exists(&self, path: &str) -> Result<bool, WorkspaceError> {
        let abs = self.resolve(path)?;
        tokio::fs::try_exists(&abs)
            .await
            .map_err(|e| WorkspaceError::Io(e.to_string()))
    }

    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, WorkspaceError> {
        let abs = self.resolve(path)?;
        let mut rd = match tokio::fs::read_dir(&abs).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(WorkspaceError::NotFound(path.to_string()));
            }
            Err(e) => return Err(WorkspaceError::Io(e.to_string())),
        };
        let mut out = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| WorkspaceError::Io(e.to_string()))?
        {
            let is_dir = entry
                .file_type()
                .await
                .map(|t| t.is_dir())
                .map_err(|e| WorkspaceError::Io(e.to_string()))?;
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
            });
        }
        Ok(out)
    }

    async fn remove(&self, path: &str) -> Result<(), WorkspaceError> {
        let abs = self.resolve(path)?;
        match tokio::fs::remove_file(&abs).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(WorkspaceError::NotFound(path.to_string()))
            }
            Err(e) => Err(WorkspaceError::Io(e.to_string())),
        }
    }
}

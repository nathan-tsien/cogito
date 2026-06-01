//! `Workspace` — a rooted, sandboxable working tree (ADR-0030).
//!
//! A working tree is distinct from the v0.5 `StorageSystem` blob store: it is
//! mutable, path-addressed, and confined to a root. Skills use it to read
//! bundled files, author scratch files, and read script output.
//!
//! Brain sees only `dyn Workspace`. Concrete impls live in Hands crates
//! (`LocalWorkspace` in `cogito-tools`; `SandboxWorkspace` in `cogito-sandbox`
//! at v0.4) and are injected by the Runtime. Path confinement (every `path`
//! is relative to `root()`; escapes are rejected) is a trait-level invariant
//! asserted by `test_support::contract_workspace`.

use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

/// One immediate child of a directory, as returned by [`Workspace::list`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// File name (final path component), not a full path.
    pub name: String,
    /// `true` if this entry is a directory.
    pub is_dir: bool,
}

/// Failure modes for [`Workspace`] operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WorkspaceError {
    /// No file or directory exists at the requested path.
    #[error("path not found: {0}")]
    NotFound(String),
    /// The input path resolves outside the workspace root (absolute path or
    /// `..` climbing above root). This is the confinement guarantee.
    #[error("path escapes workspace root: {0}")]
    PathEscapesRoot(String),
    /// Any other I/O failure, stringified (mirrors `CommandError`).
    #[error("workspace io error: {0}")]
    Io(String),
}

/// A rooted, sandboxable working tree. All `path` arguments are interpreted
/// relative to [`Workspace::root`]; implementations MUST reject paths that
/// escape the root with [`WorkspaceError::PathEscapesRoot`].
#[async_trait]
pub trait Workspace: Send + Sync {
    /// Absolute root directory this workspace is confined to.
    fn root(&self) -> &Path;

    /// Read the whole file at `path` as raw bytes. UTF-8 and size-cap policy
    /// is the caller's concern (see the `read_file` tool's 1 MiB cap).
    async fn read(&self, path: &str) -> Result<Vec<u8>, WorkspaceError>;

    /// Create or overwrite the file at `path`, creating parent directories as
    /// needed.
    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), WorkspaceError>;

    /// Whether a file or directory exists at `path`.
    async fn exists(&self, path: &str) -> Result<bool, WorkspaceError>;

    /// Immediate entries of the directory at `path` (`""` lists the root).
    /// Order is unspecified; callers that need determinism must sort.
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, WorkspaceError>;

    /// Remove the file at `path`. v0.1 does not remove directories.
    async fn remove(&self, path: &str) -> Result<(), WorkspaceError>;
}

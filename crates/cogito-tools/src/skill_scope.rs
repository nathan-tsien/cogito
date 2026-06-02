//! Read-only skill-root scope resolution (ADR-0032).
//!
//! The read-class file tools (`read_file`, `list_dir`) may read a path that
//! lies within a registered skill bundle root, in addition to the writable
//! `Workspace`. This module decides whether a requested path resolves into one
//! of `ExecCtx.skill_roots`, using the same **lexical** discipline as
//! `LocalWorkspace` (no `canonicalize`, no symlink-follow — ADR-0030 Q4); a
//! symlink inside a bundle that points outside it is therefore not blocked in
//! v0.2 (trusted operator skills; `canonicalize`-hardening is Phase 3).

use std::path::{Component, Path, PathBuf};

/// If `path` is absolute and, after lexical `.`/`..` normalization, lies within
/// one of `roots`, return the normalized absolute path (a read-only skill-root
/// hit). Otherwise return `None` — the caller falls back to the workspace.
///
/// Relative paths always return `None`: skill bundles are addressed by the
/// absolute root surfaced in the `<skill root="...">` header (ADR-0029), so a
/// relative path is a workspace-tree reference.
pub(crate) fn resolve_in_roots(path: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        return None;
    }
    let normalized = lexical_normalize(candidate);
    roots
        .iter()
        .any(|root| normalized.starts_with(root))
        .then_some(normalized)
}

/// Resolve `.` and `..` purely lexically (no filesystem access). `..` pops the
/// last kept component; popping at the root is a no-op, so an over-climbing
/// path normalizes to something that simply will not be contained by any skill
/// root and is therefore rejected by `resolve_in_roots`.
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(seg) => out.push(seg),
        }
    }
    out
}

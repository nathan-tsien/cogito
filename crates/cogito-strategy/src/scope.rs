//! Scope precedence model for `FsStrategyRegistry`.
//!
//! Repo scope (highest) is conventionally `.cogito/strategies/` at the
//! current working directory. User scope (lowest) is
//! `~/.config/cogito/strategies/` (or the XDG equivalent). Repo wins
//! over User on cross-scope name collision; same-scope duplicate is
//! fatal at registry-build.

use std::path::PathBuf;

/// Discovery scope. Repo > User.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Repo-local: `.cogito/strategies/`.
    Repo,
    /// User-global: `~/.config/cogito/strategies/`.
    User,
}

/// One scope root: a (scope, path) pair. `path` is the directory the
/// registry will scan recursively.
#[derive(Debug, Clone)]
pub struct ScopeRoot {
    /// Which scope this root belongs to (drives precedence).
    pub scope: Scope,
    /// Directory the registry will scan recursively.
    pub path: PathBuf,
}

impl ScopeRoot {
    /// Convenience constructor.
    #[must_use]
    pub fn new(scope: Scope, path: PathBuf) -> Self {
        Self { scope, path }
    }
}

/// Return the conventional roots in highest-precedence-first order.
/// Missing directories are not filtered here — `FsStrategyRegistry`
/// silently skips them. The Repo root respects an explicit override
/// (e.g., from `cogito.toml` `runtime.strategies_dir`); pass `None` to
/// use the convention.
#[must_use]
pub fn conventional_scopes(repo_override: Option<PathBuf>) -> Vec<ScopeRoot> {
    let repo = repo_override.unwrap_or_else(|| PathBuf::from(".cogito/strategies"));
    let user = user_scope_dir();
    vec![
        ScopeRoot::new(Scope::Repo, repo),
        ScopeRoot::new(Scope::User, user),
    ]
}

fn user_scope_dir() -> PathBuf {
    // Honor XDG_CONFIG_HOME if set, else fall back to ~/.config.
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("cogito").join("strategies");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("cogito")
            .join("strategies");
    }
    // Last-resort: relative path that almost certainly won't exist, which
    // FsStrategyRegistry treats as "no User scope" silently.
    PathBuf::from(".config").join("cogito").join("strategies")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_repo_path() {
        let roots = conventional_scopes(None);
        assert_eq!(roots[0].scope, Scope::Repo);
        assert_eq!(roots[0].path, PathBuf::from(".cogito/strategies"));
    }

    #[test]
    fn repo_override_honored() {
        let roots = conventional_scopes(Some(PathBuf::from("/tmp/custom")));
        assert_eq!(roots[0].path, PathBuf::from("/tmp/custom"));
    }

    #[test]
    fn user_xdg_honored() {
        // `temp_env::with_vars` snapshots the prior values, applies the
        // override for the closure, and restores them on exit. This is
        // the workspace-standard pattern (used by cogito-config tests)
        // and sidesteps the Rust 2024 `unsafe` env mutation rules.
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("/xdg/home"))], || {
            let roots = conventional_scopes(None);
            assert_eq!(roots[1].path, PathBuf::from("/xdg/home/cogito/strategies"));
        });
    }
}

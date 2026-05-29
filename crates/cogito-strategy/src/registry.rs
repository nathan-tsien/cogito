//! Filesystem-backed `StrategyRegistry` impl.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};
use walkdir::WalkDir;

use crate::error::LoadError;
use crate::parser::{ParsedStrategy, parse_strategy_file};
use crate::scope::{Scope, ScopeRoot};

/// FS-backed registry. Built once at startup; immutable thereafter.
#[derive(Debug, Clone)]
pub struct FsStrategyRegistry {
    /// `name -> parsed strategy` (winning scope only).
    by_name: BTreeMap<String, Entry>,
}

/// Per-name registry entry: the parsed strategy plus the scope that
/// "won" — useful for diagnostics and for surfacing where a strategy
/// came from.
#[derive(Debug, Clone)]
struct Entry {
    parsed: ParsedStrategy,
    #[allow(dead_code)]
    winning_scope: Scope,
}

impl FsStrategyRegistry {
    /// Build a registry by scanning the given scope roots in
    /// highest-precedence-first order. Missing roots are silently
    /// skipped. Same-scope duplicate names are fatal; cross-scope
    /// shadowing is allowed (higher-precedence scope wins, lower is
    /// silently dropped).
    ///
    /// # Errors
    ///
    /// Returns the first `LoadError` encountered (I/O, parse, or
    /// duplicate-within-scope).
    pub fn from_roots(roots: &[ScopeRoot]) -> Result<Self, LoadError> {
        let mut by_name: BTreeMap<String, Entry> = BTreeMap::new();

        for root in roots {
            let scope = root.scope;
            // Per-scope dedupe tracker to detect within-scope duplicates.
            let mut scope_seen: BTreeMap<String, PathBuf> = BTreeMap::new();

            if !root.path.exists() {
                tracing::debug!(path = %root.path.display(), "scope root missing; skipping");
                continue;
            }

            for entry in WalkDir::new(&root.path).into_iter().filter_map(Result::ok) {
                if !entry.file_type().is_file() {
                    continue;
                }
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                let parsed = parse_strategy_file(p)?;
                let name = parsed.strategy.name.clone();

                if let Some(prev_path) = scope_seen.get(&name) {
                    return Err(LoadError::DuplicateName {
                        name,
                        files: vec![prev_path.clone(), p.to_path_buf()],
                    });
                }
                scope_seen.insert(name.clone(), p.to_path_buf());

                // Cross-scope shadowing: only insert if the name is not
                // already taken by a higher-precedence scope.
                if by_name.contains_key(&name) {
                    tracing::debug!(
                        name = %name,
                        winning_path = %by_name[&name].parsed.source_path.display(),
                        shadowed_path = %p.display(),
                        "lower-precedence scope shadowed",
                    );
                } else {
                    by_name.insert(
                        name.clone(),
                        Entry {
                            parsed,
                            winning_scope: scope,
                        },
                    );
                }
            }
        }

        Ok(Self { by_name })
    }

    /// Convenience: scan the conventional roots
    /// (`Repo: .cogito/strategies/`, `User: ~/.config/cogito/strategies/`).
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_roots`].
    pub fn from_conventional_scopes() -> Result<Self, LoadError> {
        Self::from_roots(&crate::scope::conventional_scopes(None))
    }

    /// Convenience: scan the conventional roots with an explicit Repo
    /// override (used when `cogito.toml` `runtime.strategies_dir` is set).
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_roots`].
    pub fn from_conventional_scopes_with_repo_override(repo: PathBuf) -> Result<Self, LoadError> {
        Self::from_roots(&crate::scope::conventional_scopes(Some(repo)))
    }

    /// Returns the description field for a named strategy (used by
    /// `cogito chat --list-strategies`).
    #[must_use]
    pub fn description(&self, name: &str) -> Option<&str> {
        self.by_name
            .get(name)
            .and_then(|e| e.parsed.description.as_deref())
    }

    /// Returns the `provider:` reference declared by the strategy, if any.
    /// Used by the wiring layer to cross-check against `cogito.toml`.
    #[must_use]
    pub fn provider_ref(&self, name: &str) -> Option<&str> {
        self.by_name
            .get(name)
            .and_then(|e| e.parsed.provider.as_deref())
    }
}

impl StrategyRegistry for FsStrategyRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        self.by_name
            .get(name)
            .map(|e| e.parsed.strategy.clone())
            .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
    }

    fn list(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &PathBuf, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn missing_root_is_skipped() {
        let reg = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            PathBuf::from("/does/not/exist/anywhere"),
        )])
        .unwrap();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn loads_two_strategies() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("coder.md"),
            "---\nname: coder\n---\nBe precise.\n",
        );
        write(
            &tmp.path().join("planner.md"),
            "---\nname: planner\n---\nThink first.\n",
        );

        let reg = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            tmp.path().to_path_buf(),
        )])
        .unwrap();
        assert_eq!(reg.list(), vec!["coder", "planner"]);
        assert!(reg.get("coder").is_ok());
        assert!(reg.get("planner").is_ok());
    }

    #[test]
    fn duplicate_name_within_scope_is_fatal() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("coder.md"),
            "---\nname: coder\n---\none.\n",
        );
        // Same name, different file - simulate by placing in a subdir.
        write(
            &tmp.path().join("subdir/coder.md"),
            "---\nname: coder\n---\ntwo.\n",
        );

        let err = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            tmp.path().to_path_buf(),
        )])
        .unwrap_err();
        assert!(matches!(err, LoadError::DuplicateName { ref name, .. } if name == "coder"));
    }

    #[test]
    fn repo_shadows_user() {
        let repo = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write(
            &repo.path().join("coder.md"),
            "---\nname: coder\n---\nFROM REPO.\n",
        );
        write(
            &user.path().join("coder.md"),
            "---\nname: coder\n---\nFROM USER.\n",
        );

        let reg = FsStrategyRegistry::from_roots(&[
            ScopeRoot::new(Scope::Repo, repo.path().to_path_buf()),
            ScopeRoot::new(Scope::User, user.path().to_path_buf()),
        ])
        .unwrap();

        let s = reg.get("coder").unwrap();
        assert_eq!(s.system_prompt.trim(), "FROM REPO.");
    }

    #[test]
    fn unknown_name_returns_strategy_error() {
        let reg = FsStrategyRegistry::from_roots(&[]).unwrap();
        let err = reg.get("nope").unwrap_err();
        assert!(matches!(err, StrategyError::Unknown(ref n, _) if n == "nope"));
    }
}

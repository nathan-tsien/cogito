//! `SkillRegistry` — eager-scan implementation of `SkillProvider`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thiserror::Error;
use tracing::{debug, warn};

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};

use crate::discovery::{DiscoveryError, ScanConfig, discover_skills};

/// Errors from `SkillRegistry::scan`.
///
/// Note: a duplicate skill *name* is deliberately **not** an error. Name
/// collisions are resolved by precedence (one skill wins; see `scan`) and
/// never fail the build — a skill clash must not crash the runtime, mirroring
/// the non-fatal MCP duplicate-name handling (ADR-0018).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SkillRegistryError {
    /// Walker-level failure.
    #[error("discovery failed: {0}")]
    Discovery(#[from] DiscoveryError),
}

#[derive(Debug)]
struct SkillRecord {
    metadata: SkillMetadata,
    body: String,
    source: SkillSource,
    /// The skill's own directory (folder containing `SKILL.md`). Surfaced
    /// as `SkillContent.root` so the model can resolve bundled-file
    /// references (ADR-0029).
    root: std::path::PathBuf,
}

/// Eager `SkillProvider`. Built once at Runtime construction; full bodies
/// kept in-memory (they tend to be small markdown files).
#[derive(Clone, Debug)]
pub struct SkillRegistry {
    by_name: Arc<HashMap<String, Arc<SkillRecord>>>,
}

impl SkillRegistry {
    /// Scan filesystem according to `config` and build the registry.
    ///
    /// Name collisions never fail the build; they resolve by precedence and
    /// exactly one skill wins:
    /// - across scopes, higher precedence wins (Repo > User > Plugin > System);
    /// - within one scope, the first in walk order wins — for Repo, the closer
    ///   (deeper) ancestor dir; within a single `.cogito/skills/` directory, the
    ///   alphabetically-first skill folder (discovery sorts by file name).
    ///
    /// A shadowed same-scope duplicate is logged at `warn`; a cross-scope shadow
    /// (intentional layering, e.g. a repo skill overriding a user default) at
    /// `debug`. The same physical skill discovered via two overlapping scopes
    /// (e.g. `user_dir` pointing at the repo `.cogito/skills`) is treated as the
    /// cross-scope shadow case, not a collision.
    ///
    /// # Errors
    ///
    /// Returns `Err(SkillRegistryError::Discovery)` only on a walker-level I/O
    /// failure. Duplicate names are not errors.
    // Builder-style by-value API: callers compose a `ScanConfig` literal
    // and hand it off; the registry is the only consumer.
    #[allow(clippy::needless_pass_by_value)]
    pub fn scan(config: ScanConfig) -> Result<Self, SkillRegistryError> {
        let mut found = discover_skills(&config)?;
        // Sort so Repo entries come before User; within Repo, closer (deeper)
        // dirs come first. We rely on discover_skills emitting in walk-order:
        // Repo-walked dirs in cwd-to-ancestor order, then User. Within one
        // scope class, detect duplicates as fatal.
        let mut by_name: HashMap<String, Arc<SkillRecord>> = HashMap::new();
        let mut seen_in_repo: HashSet<String> = HashSet::new();
        let mut seen_in_user: HashSet<String> = HashSet::new();
        for d in found.drain(..) {
            let scope_label: &'static str = match &d.source {
                SkillSource::Repo { .. } => "repo",
                SkillSource::User => "user",
                SkillSource::Plugin { .. } => "plugin",
                SkillSource::System => "system",
                // `SkillSource` is `#[non_exhaustive]`; future variants
                // surface here as an unknown scope rather than a build
                // break of downstream crates.
                _ => "unknown",
            };
            // Same-dir duplicate detection: implemented via discover_skills
            // emitting one entry per SKILL.md; same `name` from same scope
            // is fatal.
            let same_scope_seen = match &d.source {
                SkillSource::Repo { .. } => seen_in_repo.contains(&d.parsed.name),
                SkillSource::User => seen_in_user.contains(&d.parsed.name),
                _ => false,
            };
            if same_scope_seen {
                // Same-scope name collision: the earlier entry in walk order
                // already won (closer dir for Repo; alphabetically-first folder
                // within one `.cogito/skills/`). Drop this one and warn so the
                // clash is visible — but never fail the build.
                warn!(
                    name = %d.parsed.name,
                    scope = scope_label,
                    "duplicate skill name within scope; keeping the first, dropping this one"
                );
                continue;
            }
            if let Some(existing) = by_name.get(&d.parsed.name) {
                // Cross-scope: higher precedence already won.
                debug!(
                    name = %d.parsed.name,
                    existing = ?existing.source,
                    new = ?d.source,
                    "lower-scope skill shadowed",
                );
                continue;
            }
            match &d.source {
                SkillSource::Repo { .. } => {
                    seen_in_repo.insert(d.parsed.name.clone());
                }
                SkillSource::User => {
                    seen_in_user.insert(d.parsed.name.clone());
                }
                _ => {}
            }
            let metadata = SkillMetadata {
                name: d.parsed.name.clone(),
                description: d.parsed.description.clone(),
                source: d.source.clone(),
                disable_model_invocation: d.parsed.disable_model_invocation,
                user_invocable: d.parsed.user_invocable,
                version: d.parsed.version.clone(),
            };
            by_name.insert(
                d.parsed.name.clone(),
                Arc::new(SkillRecord {
                    metadata,
                    body: d.parsed.body,
                    source: d.source,
                    root: d.dir,
                }),
            );
        }
        Ok(Self {
            by_name: Arc::new(by_name),
        })
    }
}

impl SkillProvider for SkillRegistry {
    fn list(&self) -> Vec<SkillMetadata> {
        // Sort alphabetically by name so downstream consumers (notably
        // `SkillInjector`, which serializes this list into the system
        // prompt) see a stable order — `HashMap` iteration is otherwise
        // arbitrary and would defeat prompt caching.
        let mut out: Vec<SkillMetadata> =
            self.by_name.values().map(|r| r.metadata.clone()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    fn get(&self, name: &str) -> Option<SkillContent> {
        self.by_name.get(name).map(|r| SkillContent {
            name: r.metadata.name.clone(),
            source: r.source.clone(),
            body: r.body.clone(),
            root: Some(r.root.clone()),
        })
    }

    fn is_registered(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    fn get_metadata(&self, name: &str) -> Option<SkillMetadata> {
        self.by_name.get(name).map(|r| r.metadata.clone())
    }

    fn skill_roots(&self) -> Vec<std::path::PathBuf> {
        // Each skill's own directory; dedup + sort for a deterministic,
        // stable set (HashMap order is arbitrary). Distinct skills normally
        // have distinct dirs, but dedup keeps the contract honest.
        let mut roots: Vec<std::path::PathBuf> =
            self.by_name.values().map(|r| r.root.clone()).collect();
        roots.sort();
        roots.dedup();
        roots
    }
}

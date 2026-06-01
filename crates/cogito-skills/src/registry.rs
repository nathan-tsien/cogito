//! `SkillRegistry` — eager-scan implementation of `SkillProvider`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thiserror::Error;
use tracing::debug;

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};

use crate::discovery::{DiscoveryError, ScanConfig, discover_skills};

/// Errors from `SkillRegistry::scan`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SkillRegistryError {
    /// Two skills declared the same `name` inside one directory (or the
    /// directory chain visited as a single dir scan).
    #[error("duplicate skill name '{name}' in scope {scope}")]
    DuplicateName {
        /// The colliding skill name.
        name: String,
        /// Human-readable scope label ("repo", "user", "plugin", "system").
        scope: &'static str,
    },
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
    /// # Errors
    ///
    /// Returns `Err(SkillRegistryError::DuplicateName)` if two skills
    /// within the same scope class declare the same `name`. Higher-scope
    /// shadowing across classes (Repo > User > Plugin > System) is silent.
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
                // Repo monorepo walk: closer dir wins; emit debug + skip.
                // Strict fatal only for explicit collision (which v0.1
                // hard-defines as "same scope label"). For Repo walk-up,
                // the deeper dir already populated by_name; treat the
                // later (shallower) dup as a closer-wins case.
                debug!(
                    name = %d.parsed.name,
                    scope = scope_label,
                    "duplicate within scope dropped (closer dir already won)"
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
        // Same-directory duplicate (file-system level): two SKILL.md files in
        // the same `skills/<dir>/` cannot occur (only one SKILL.md per dir).
        // Two skill dirs declaring the same `name` is what we want to surface
        // as fatal — that's the "same-dir" case in the spec. The check is
        // factored out so the main pass stays focused on building the table.
        check_same_dir_duplicates(&config)?;

        Ok(Self {
            by_name: Arc::new(by_name),
        })
    }
}

/// Same-directory duplicate detection. Two skill dirs that are direct
/// children of the same `.cogito/skills/` directory and that both declare
/// the same `name` are surfaced as a fatal `DuplicateName` error.
fn check_same_dir_duplicates(config: &ScanConfig) -> Result<(), SkillRegistryError> {
    let mut found = discover_skills(config)?;
    let mut by_dir: HashMap<std::path::PathBuf, HashSet<String>> = HashMap::new();
    for d in found.drain(..) {
        let parent = d
            .dir
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let names = by_dir.entry(parent).or_default();
        if names.contains(&d.parsed.name) {
            let scope_label = match d.source {
                SkillSource::Repo { .. } => "repo",
                SkillSource::User => "user",
                SkillSource::Plugin { .. } => "plugin",
                SkillSource::System => "system",
                // See note in `SkillRegistry::scan`; non_exhaustive.
                _ => "unknown",
            };
            return Err(SkillRegistryError::DuplicateName {
                name: d.parsed.name.clone(),
                scope: scope_label,
            });
        }
        names.insert(d.parsed.name.clone());
    }
    Ok(())
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
}

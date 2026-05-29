//! Filesystem walker — Repo scope (workspace root walk-up) + User scope
//! (`~/.cogito/skills/` by default).

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, warn};

use cogito_protocol::skill::SkillSource;

use crate::metadata::{ParseError, ParsedSkill, parse_skill_md};

/// Configuration for the discovery walker.
#[derive(Clone, Debug, Default)]
pub struct ScanConfig {
    /// Starting cwd for the Repo-scope walk-up; `None` skips Repo scope.
    pub workspace_root: Option<PathBuf>,
    /// User-scope skills directory; `None` disables user scope. Missing
    /// directories are not errors.
    pub user_dir: Option<PathBuf>,
    /// Include cogito-bundled (System) skills. v0.1 leaves this off.
    pub include_system: bool,
}

/// One discovered skill — frontmatter parsed, body retained, source known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredSkill {
    /// Parsed `SKILL.md` content.
    pub parsed: ParsedSkill,
    /// Where it was found.
    pub source: SkillSource,
    /// The skill's own directory (the parent of `SKILL.md`).
    pub dir: PathBuf,
}

/// Errors returned by `discover_skills`. Per-skill parse failures are logged
/// + skipped; only walker-level failures surface here.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// I/O failure reading the filesystem.
    #[error("io error reading {path:?}: {source}")]
    Io {
        /// Path being read when the failure occurred.
        path: PathBuf,
        /// Wrapped io error.
        source: std::io::Error,
    },
}

/// Discover skills under the configured scopes.
///
/// Repo-scope walk-up rule: start at `workspace_root`, walk parent directories
/// until either `.git/` is present, or `cogito.toml`, or filesystem root.
/// Each directory along the path is checked for `.cogito/skills/`.
pub fn discover_skills(config: &ScanConfig) -> Result<Vec<DiscoveredSkill>, DiscoveryError> {
    let mut out = Vec::new();
    if let Some(root) = &config.workspace_root {
        for dir in repo_walk_up(root) {
            scan_skills_dir(
                &dir.join(".cogito").join("skills"),
                &SkillSource::Repo { dir: dir.clone() },
                &mut out,
            )?;
        }
    }
    if let Some(user_dir) = &config.user_dir {
        scan_skills_dir(user_dir, &SkillSource::User, &mut out)?;
    }
    if config.include_system {
        // v0.1: no bundled skills yet.
    }
    Ok(out)
}

fn repo_walk_up(start: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = start.to_path_buf();
    loop {
        chain.push(current.clone());
        if current.join(".git").exists() || current.join("cogito.toml").exists() {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    chain
}

fn scan_skills_dir(
    skills_dir: &Path,
    source: &SkillSource,
    out: &mut Vec<DiscoveredSkill>,
) -> Result<(), DiscoveryError> {
    if !skills_dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(skills_dir).map_err(|e| DiscoveryError::Io {
        path: skills_dir.to_path_buf(),
        source: e,
    })?;
    // Sort by file name so discovery output (and same-dir duplicate
    // detection) is filesystem-independent. `read_dir` on Linux returns
    // inode-allocation order, which varies across runs and machines and
    // would let prompt caching churn on the downstream `SkillInjector`.
    // Per-entry I/O errors are still silently skipped via `.flatten()`,
    // matching the previous behavior.
    let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        match parse_one(&skill_md) {
            Ok(parsed) => {
                out.push(DiscoveredSkill {
                    parsed,
                    source: source.clone(),
                    dir: dir.clone(),
                });
            }
            Err(e) => {
                warn!(?skill_md, error = %e, "skipping malformed SKILL.md");
            }
        }
    }
    Ok(())
}

fn parse_one(path: &Path) -> Result<ParsedSkill, ParseError> {
    debug!(?path, "parsing SKILL.md");
    let text = fs::read_to_string(path)?;
    parse_skill_md(&text)
}

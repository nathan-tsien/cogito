//! Plugin discovery: resolve entries, parse manifests, namespace
//! artifacts, apply overrides, enforce id-uniqueness. See ADR-0021.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cogito_mcp::McpServerConfig;
use cogito_skills::PluginSkillRoot;
use serde::Deserialize;

use crate::manifest::PluginManifest;
use crate::{PluginContributions, PluginEntry, PluginError};

/// Resolves declared plugin entries into contributions.
pub struct PluginSet;

#[derive(Debug, Deserialize)]
struct McpFile {
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
}

impl PluginSet {
    /// Load all enabled plugins, namespacing and de-conflicting artifacts.
    ///
    /// # Errors
    /// Returns [`PluginError`] on missing paths, bad manifests, or
    /// duplicate plugin ids.
    pub fn load(
        entries: &[PluginEntry],
        config_dir: &Path,
    ) -> Result<PluginContributions, PluginError> {
        let mut out = PluginContributions::default();
        let mut seen: HashMap<String, PathBuf> = HashMap::new();

        for entry in entries {
            if !entry.enabled {
                continue;
            }
            let plugin_dir = resolve_path(config_dir, &entry.path);
            if !plugin_dir.is_dir() {
                return Err(PluginError::PathNotFound(plugin_dir));
            }

            let manifest = PluginManifest::load_from_dir(&plugin_dir)?;

            if let Some(first) = seen.insert(manifest.id.clone(), plugin_dir.clone()) {
                return Err(PluginError::DuplicateId {
                    id: manifest.id,
                    first,
                    second: plugin_dir,
                });
            }

            collect_skills(&plugin_dir, &manifest, entry, &mut out);
            collect_mcp(&plugin_dir, &manifest, entry, &mut out)?;
        }

        Ok(out)
    }
}

fn resolve_path(config_dir: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        config_dir.join(p)
    }
}

fn is_disabled(entry: &PluginEntry, plugin_id: &str, kind: &str, name: &str) -> bool {
    entry
        .artifact_overrides
        .iter()
        .any(|o| o.plugin == plugin_id && o.kind == kind && o.name == name && !o.enabled)
}

fn collect_skills(
    plugin_dir: &Path,
    manifest: &PluginManifest,
    entry: &PluginEntry,
    out: &mut PluginContributions,
) {
    let skills_dir = plugin_dir.join(&manifest.skills_dir);
    if !skills_dir.is_dir() {
        return;
    }
    // v0.2: per-skill disable is coarse. Register the root unless every
    // skill subdir is overridden off. (Finer per-skill filtering is a
    // follow-up; SkillRegistry::scan consumes a directory, not a name list.)
    let any_enabled = std::fs::read_dir(&skills_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !is_disabled(entry, &manifest.id, "skill", &name)
        });
    if any_enabled {
        out.skill_roots.push(PluginSkillRoot {
            plugin_id: manifest.id.clone(),
            dir: skills_dir,
        });
    }
}

fn collect_mcp(
    plugin_dir: &Path,
    manifest: &PluginManifest,
    entry: &PluginEntry,
    out: &mut PluginContributions,
) -> Result<(), PluginError> {
    let mcp_path = plugin_dir.join(&manifest.mcp_file);
    if !mcp_path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&mcp_path).map_err(|e| PluginError::Manifest {
        path: mcp_path.clone(),
        source: Box::new(e),
    })?;
    let parsed: McpFile = toml::from_str(&text).map_err(|e| PluginError::Manifest {
        path: mcp_path.clone(),
        source: Box::new(e),
    })?;
    for mut server in parsed.mcp_servers {
        if is_disabled(entry, &manifest.id, "mcp", &server.name) {
            continue;
        }
        server.name = format!("{}:{}", manifest.id, server.name);
        out.mcp_servers.push(server);
    }
    Ok(())
}

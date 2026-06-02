//! Plugin manifest parsing: `.cogito-plugin/plugin.toml` primary,
//! `.claude-plugin/plugin.json` metadata-only fallback. See ADR-0021 §1.

use std::path::Path;

use serde::Deserialize;

use crate::PluginError;

/// Internal manifest model after parsing.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Globally unique plugin id (the namespace prefix).
    pub id: String,
    /// Optional semver.
    pub version: Option<String>,
    /// Optional human description.
    pub description: Option<String>,
    /// Skills directory relative to the plugin root.
    pub skills_dir: String,
    /// MCP file relative to the plugin root.
    pub mcp_file: String,
}

/// Validate a plugin id against ADR-0021 §1: `[a-z0-9-]+` (non-empty,
/// lowercase ASCII letters, digits, and hyphens only). The id becomes the
/// `<plugin_id>:<artifact>` namespace prefix, so a tight character set keeps
/// namespaced names predictable across skills, MCP servers, and sigils.
fn validate_id(id: &str, plugin_dir: &Path) -> Result<(), PluginError> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(PluginError::InvalidId {
            id: id.to_string(),
            path: plugin_dir.to_path_buf(),
        })
    }
}

fn default_skills_dir() -> String {
    "skills".to_string()
}
fn default_mcp_file() -> String {
    "mcp.toml".to_string()
}

#[derive(Debug, Deserialize)]
struct TomlManifest {
    plugin: TomlPluginSection,
}

#[derive(Debug, Deserialize)]
struct TomlPluginSection {
    id: String,
    version: Option<String>,
    description: Option<String>,
    #[serde(default = "default_skills_dir")]
    skills_dir: String,
    #[serde(default = "default_mcp_file")]
    mcp_file: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeJsonManifest {
    name: String,
    version: Option<String>,
    description: Option<String>,
}

impl PluginManifest {
    /// Load a manifest from a plugin directory. Prefers
    /// `.cogito-plugin/plugin.toml`; falls back to
    /// `.claude-plugin/plugin.json` (metadata only). Both absent → error.
    ///
    /// # Errors
    /// Returns [`PluginError::Manifest`] if no manifest exists or parsing
    /// fails.
    pub fn load_from_dir(plugin_dir: &Path) -> Result<Self, PluginError> {
        let toml_path = plugin_dir.join(".cogito-plugin/plugin.toml");
        if toml_path.is_file() {
            let text = std::fs::read_to_string(&toml_path).map_err(|e| PluginError::Manifest {
                path: toml_path.clone(),
                source: Box::new(e),
            })?;
            let parsed: TomlManifest =
                toml::from_str(&text).map_err(|e| PluginError::Manifest {
                    path: toml_path.clone(),
                    source: Box::new(e),
                })?;
            validate_id(&parsed.plugin.id, plugin_dir)?;
            return Ok(Self {
                id: parsed.plugin.id,
                version: parsed.plugin.version,
                description: parsed.plugin.description,
                skills_dir: parsed.plugin.skills_dir,
                mcp_file: parsed.plugin.mcp_file,
            });
        }

        let json_path = plugin_dir.join(".claude-plugin/plugin.json");
        if json_path.is_file() {
            let text = std::fs::read_to_string(&json_path).map_err(|e| PluginError::Manifest {
                path: json_path.clone(),
                source: Box::new(e),
            })?;
            let parsed: ClaudeJsonManifest =
                serde_json::from_str(&text).map_err(|e| PluginError::Manifest {
                    path: json_path.clone(),
                    source: Box::new(e),
                })?;
            validate_id(&parsed.name, plugin_dir)?;
            return Ok(Self {
                id: parsed.name,
                version: parsed.version,
                description: parsed.description,
                skills_dir: default_skills_dir(),
                mcp_file: default_mcp_file(),
            });
        }

        Err(PluginError::Manifest {
            path: plugin_dir.to_path_buf(),
            source: "no .cogito-plugin/plugin.toml or .claude-plugin/plugin.json".into(),
        })
    }
}

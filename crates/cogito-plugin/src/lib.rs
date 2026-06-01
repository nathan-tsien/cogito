//! Local-path plugin loader (v0.2, Skills + MCP). See ADR-0021.
//!
//! A plugin is a directory with a `.cogito-plugin/plugin.toml` manifest
//! (or a `.claude-plugin/plugin.json` read for metadata only) bundling
//! `skills/` and `mcp.toml`. [`PluginSet::load`] resolves declared
//! entries into [`PluginContributions`] that the caller folds into the
//! existing `SkillRegistry` and `build_mcp_provider`.

#![forbid(unsafe_code)]

mod discovery;
mod manifest;

pub use discovery::PluginSet;
pub use manifest::PluginManifest;

use std::path::PathBuf;

use cogito_mcp::McpServerConfig;
use cogito_skills::PluginSkillRoot;

/// One `[[plugins]]` entry from `cogito.toml`. Owned here; aggregated by
/// `cogito-config` (mirrors the `cogito-config → cogito-mcp` edge).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PluginEntry {
    /// Path to the plugin directory (absolute, or relative to `cogito.toml`).
    pub path: String,
    /// Whether the plugin is active. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-artifact enable/disable overrides.
    #[serde(default)]
    pub artifact_overrides: Vec<ArtifactOverride>,
}

fn default_true() -> bool {
    true
}

/// Fine-grained override disabling a single bundled artifact.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ArtifactOverride {
    /// Target plugin id.
    pub plugin: String,
    /// Artifact kind: `"skill"` or `"mcp"` (v0.2).
    pub kind: String,
    /// Bare artifact name (pre-namespacing).
    pub name: String,
    /// Whether the artifact is enabled.
    pub enabled: bool,
}

/// Everything a plugin set contributes, ready to fold into the existing
/// registries. No providers are built here.
#[derive(Debug, Default)]
pub struct PluginContributions {
    /// Plugin skill roots, for `SkillRegistry` Plugin scope.
    pub skill_roots: Vec<PluginSkillRoot>,
    /// Namespaced MCP server configs, to concatenate before
    /// `build_mcp_provider`.
    pub mcp_servers: Vec<McpServerConfig>,
}

/// Errors raised while loading a plugin set.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// A declared plugin path does not exist or is not a directory.
    #[error("plugin path not found or not a directory: {0}")]
    PathNotFound(PathBuf),
    /// The manifest could not be read or parsed.
    #[error("invalid plugin manifest at {path}: {source}")]
    Manifest {
        /// Manifest path.
        path: PathBuf,
        /// Underlying parse error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Two plugins declared the same id.
    #[error("duplicate plugin id `{id}` (declared at {first} and {second})")]
    DuplicateId {
        /// The conflicting id.
        id: String,
        /// First plugin path.
        first: PathBuf,
        /// Second plugin path.
        second: PathBuf,
    },
}

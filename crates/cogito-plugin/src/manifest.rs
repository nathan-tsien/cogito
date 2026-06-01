//! Plugin manifest parsing (filled in Task 3).
#![allow(dead_code)]

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

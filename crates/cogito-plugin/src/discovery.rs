//! Plugin discovery + contribution assembly (filled in Task 4).
#![allow(dead_code)]

use std::path::Path;

use crate::{PluginContributions, PluginEntry, PluginError};

/// Resolves declared plugin entries into contributions.
pub struct PluginSet;

impl PluginSet {
    /// Load all enabled plugins, namespacing and de-conflicting artifacts.
    ///
    /// # Errors
    /// Returns [`PluginError`] on missing paths, bad manifests, or
    /// duplicate plugin ids.
    pub fn load(
        _entries: &[PluginEntry],
        _config_dir: &Path,
    ) -> Result<PluginContributions, PluginError> {
        Ok(PluginContributions::default())
    }
}

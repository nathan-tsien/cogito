//! Sandbox configuration: a tagged-union over the executor kinds
//! `cogito-sandbox` knows how to construct. Per CLAUDE.md, the
//! `match`-on-kind dispatch (`build_executor`) lives in this crate; no
//! surface reproduces it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tagged config selecting a `CommandExecutor` implementation. v0.1 ships
/// only `Direct`; v0.4 (ADR-0012/0013) adds isolating / remote variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxConfig {
    /// No isolation: run on the host. The "sandbox off" default.
    Direct(DirectConfig),
}

// `#[derive(Default)]` with `#[default]` only applies to unit enum variants;
// `Direct` carries a payload, so the default is written by hand.
impl Default for SandboxConfig {
    fn default() -> Self {
        Self::Direct(DirectConfig::default())
    }
}

/// Configuration for `DirectExecutor`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DirectConfig {
    /// Root working directory. Relative `CommandSpec::cwd` resolves under
    /// this. Defaults to the process current dir (`.`).
    pub root: PathBuf,
    /// Whether the child inherits the parent process environment. Defaults
    /// to `true` (v0.1 is not a security boundary).
    pub inherit_env: bool,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            inherit_env: true,
        }
    }
}

/// Error building a `CommandExecutor` from config.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// Reserved for future variants that validate isolation prerequisites
    /// (namespaces, cgroups). `Direct` never errors today.
    #[error("sandbox configuration error: {0}")]
    Config(String),
}

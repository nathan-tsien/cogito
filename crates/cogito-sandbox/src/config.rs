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

/// Environment policy for a child process started by `DirectExecutor`.
///
/// This is a programmatic, construction-time concern: it is set in code (the
/// local/TUI surface sets `Allowlist`), not loaded from `cogito.toml`, so it
/// is deliberately not part of the serde schema (see `DirectConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EnvPolicy {
    /// Child inherits the parent environment per `DirectConfig::inherit_env`
    /// (the v0.1 default; "not a security boundary"). The default policy, so
    /// existing behavior is preserved.
    #[default]
    InheritAll,
    /// Child starts from an empty environment and receives ONLY these keys,
    /// copied from the parent process when present. Everything else (secrets)
    /// is denied. Used by the local/TUI surface (ADR-0037).
    Allowlist(Vec<String>),
}

/// Configuration for `DirectExecutor`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DirectConfig {
    /// Root working directory. Relative `CommandSpec::cwd` resolves under
    /// this. Defaults to the process current dir (`.`).
    pub root: PathBuf,
    /// Whether the child inherits the parent process environment. Defaults
    /// to `true` (v0.1 is not a security boundary). Only consulted under
    /// `EnvPolicy::InheritAll`.
    pub inherit_env: bool,
    /// Environment scrubbing policy. Set programmatically; `#[serde(skip)]`
    /// keeps the `cogito.toml` schema unchanged, and skipped fields fall back
    /// to `Default` on deserialize, so existing configs keep working and load
    /// `EnvPolicy::InheritAll` (exact v0.1 behavior).
    #[serde(skip)]
    pub env_policy: EnvPolicy,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            inherit_env: true,
            env_policy: EnvPolicy::InheritAll,
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

//! cogito-sandbox
//!
//! Subprocess-based execution sandbox. Provides cwd isolation, resource
//! limits, and timeout enforcement. Not a security boundary — that's a
//! production concern (v0.4 ADR-0012/0013). Goal here is to *behave* like
//! a sandbox so the Harness can be validated against the production
//! contract. Implements `cogito_protocol::CommandExecutor`; the executor
//! is selected by `build_executor` and injected into tools (e.g. `bash`)
//! at the Surface layer.

mod config;
mod executor;
mod truncate;

use std::sync::Arc;

use cogito_protocol::CommandExecutor;

pub use config::{DirectConfig, EnvPolicy, SandboxConfig, SandboxError};
pub use executor::DirectExecutor;

/// Curated set of environment variable names safe to forward to a child
/// process under `EnvPolicy::Allowlist`. This is the local/TUI default
/// (ADR-0037): default-deny everything else (secrets) and copy in only these
/// keys when present in the parent. Consumers extend this list as their tools
/// require (e.g. proxy settings, language-specific paths).
#[must_use]
pub fn default_safe_env_allowlist() -> Vec<String> {
    [
        "PATH", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "USER", "LOGNAME", "SHELL", "TERM",
        "PWD",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Build a `CommandExecutor` from `SandboxConfig`. The only place in the
/// workspace that pattern-matches on the sandbox `kind`; surfaces call
/// this and receive a trait object.
///
/// # Errors
///
/// Returns `SandboxError` for configurations whose prerequisites cannot be
/// satisfied. `Direct` never errors today.
pub fn build_executor(cfg: &SandboxConfig) -> Result<Arc<dyn CommandExecutor>, SandboxError> {
    match cfg {
        SandboxConfig::Direct(c) => Ok(Arc::new(DirectExecutor::new(c.clone()))),
    }
}

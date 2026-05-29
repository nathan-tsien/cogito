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

pub use config::{DirectConfig, SandboxConfig, SandboxError};
pub use executor::DirectExecutor;

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

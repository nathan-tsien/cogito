//! `EnvConfigLoader` — reads `COGITO_*` environment variables and
//! produces a `RuntimeConfigPartial`. Default-features available
//! (std-only; no third-party deps).
//!
//! Legacy variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
//! `OPENAI_BASE_URL`) are NOT handled here — they belong to the
//! `cogito-cli` legacy bridge that synthesizes a `default` provider
//! when `cogito.toml` is absent. See `docs/configuration/overview.md`
//! §10.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::{RuntimeConfigPartial, RuntimeSectionPartial};

/// Reads `COGITO_*` env vars synchronously inside an async `load`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvConfigLoader;

#[async_trait]
impl ConfigLoader for EnvConfigLoader {
    /// Read `COGITO_SESSION_ROOT`, `COGITO_DEFAULT_PROVIDER`,
    /// `COGITO_DEFAULT_MODEL`, and `COGITO_STRATEGIES_DIR` from the
    /// process environment. Returns a `RuntimeConfigPartial` whose
    /// `runtime` field is `Some(..)` iff at least one variable is set
    /// to a non-empty value; otherwise the partial is fully empty.
    /// Never fails: missing vars are simply absent contributions.
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
        let session_root = read_path("COGITO_SESSION_ROOT");
        let default_provider = read_string("COGITO_DEFAULT_PROVIDER");
        let default_model = read_string("COGITO_DEFAULT_MODEL");
        let strategies_dir = read_path("COGITO_STRATEGIES_DIR");

        let any_runtime_field = session_root.is_some()
            || default_provider.is_some()
            || default_model.is_some()
            || strategies_dir.is_some();

        let partial = RuntimeConfigPartial {
            runtime: any_runtime_field.then_some(RuntimeSectionPartial {
                session_root,
                default_provider,
                default_model,
                default_strategy: None,
                strategies_dir,
            }),
            providers: None,
            mcp_servers: None,
            skills: None,
            tools: None,
        };
        Ok(partial)
    }
}

fn read_string(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn read_path(key: &str) -> Option<PathBuf> {
    read_string(key).map(PathBuf::from)
}

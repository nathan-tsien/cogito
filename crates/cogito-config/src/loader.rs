//! `ConfigLoader` trait and `ConfigError`. Every source (file, env, db,
//! custom) implements `ConfigLoader::load -> RuntimeConfigPartial`.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::types::RuntimeConfigPartial;

/// One source of configuration. Sources do not see each other; merge
/// happens externally via `merge_layers`.
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    /// Read this source and produce a partial configuration contribution.
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError>;
}

/// Failures a `ConfigLoader` may report. Marked `#[non_exhaustive]` so
/// future loaders (e.g. database, secrets manager) can add variants
/// without breaking downstream `match` arms.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A required environment variable was not set or was empty.
    #[error("missing required environment variable: {0}")]
    MissingEnv(String),

    /// TOML deserialization failed for the file at `path`.
    #[cfg(feature = "file")]
    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        /// Path of the file that failed to parse.
        path: PathBuf,
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
    },

    /// YAML deserialization failed for the file at `path`.
    #[cfg(feature = "file")]
    #[error("invalid YAML in {path}: {source}")]
    YamlParse {
        /// Path of the file that failed to parse.
        path: PathBuf,
        /// Underlying parser error.
        #[source]
        source: serde_yaml::Error,
    },

    /// Filesystem read failed (missing file, permission denied, etc.).
    #[error("I/O error reading {path}: {source}")]
    Io {
        /// Path the loader attempted to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// `${ENV_VAR}` interpolation pass failed (unknown variable,
    /// unbalanced braces, etc.).
    #[error("interpolation error: {0}")]
    Interpolation(String),

    /// Post-merge validation rejected the resolved configuration.
    #[error("validation failed: {0}")]
    Validation(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct EmptyLoader;

    #[async_trait]
    impl ConfigLoader for EmptyLoader {
        async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
            Ok(RuntimeConfigPartial::default())
        }
    }

    #[tokio::test]
    async fn empty_loader_returns_default() {
        let l = EmptyLoader;
        let p = l.load().await.expect("ok");
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
    }

    #[test]
    fn config_error_messages_are_informative() {
        let err = ConfigError::MissingEnv("ANTHROPIC_API_KEY".into());
        assert!(err.to_string().contains("ANTHROPIC_API_KEY"));

        let err = ConfigError::Validation("no provider declared".into());
        assert!(err.to_string().contains("no provider declared"));
    }
}

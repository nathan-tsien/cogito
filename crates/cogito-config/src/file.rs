//! `FileConfigLoader` — reads `cogito.toml` from a resolved path,
//! interpolates `${ENV_VAR}` placeholders, and deserializes into a
//! `RuntimeConfigPartial`. See ADR-0017 §7 for the search-path
//! decision.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::interpolate::interpolate_value;
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::RuntimeConfigPartial;

/// Resolves a `cogito.toml` path per the four-step search rule and
/// reads it on `load`. If no path is found, `load` returns
/// `RuntimeConfigPartial::default()`.
#[derive(Debug, Clone, Default)]
pub struct FileConfigLoader {
    resolved_path: Option<PathBuf>,
}

impl FileConfigLoader {
    /// Resolve the file path per the search order:
    ///
    /// 1. `--config <path>` arg (parameter)
    /// 2. `$COGITO_CONFIG`
    /// 3. `./cogito.toml`
    /// 4. `$XDG_CONFIG_HOME/cogito/config.toml` (if `XDG_CONFIG_HOME` is set)
    /// 5. No file -> loader returns empty partial on load.
    ///
    /// Returns the loader (with the resolved path, if any). Path
    /// resolution itself never fails; load-time errors surface from
    /// `load()`.
    pub fn resolve<P: AsRef<Path>>(arg: Option<P>) -> Result<Self, ConfigError> {
        if let Some(p) = arg {
            return Ok(Self {
                resolved_path: Some(p.as_ref().to_path_buf()),
            });
        }
        if let Ok(v) = std::env::var("COGITO_CONFIG")
            && !v.is_empty()
        {
            return Ok(Self {
                resolved_path: Some(PathBuf::from(v)),
            });
        }
        let local = PathBuf::from("./cogito.toml");
        if local.is_file() {
            return Ok(Self {
                resolved_path: Some(local),
            });
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let candidate = PathBuf::from(xdg).join("cogito").join("config.toml");
            if candidate.is_file() {
                return Ok(Self {
                    resolved_path: Some(candidate),
                });
            }
        }
        Ok(Self {
            resolved_path: None,
        })
    }

    /// Path the loader will read on `load`, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.resolved_path.as_deref()
    }
}

#[async_trait]
impl ConfigLoader for FileConfigLoader {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
        let Some(path) = &self.resolved_path else {
            tracing::debug!(target: "cogito::config", "no cogito.toml found; empty partial");
            return Ok(RuntimeConfigPartial::default());
        };
        let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.clone(),
            source: e,
        })?;
        let parsed: toml::Value = toml::from_str(&raw).map_err(|e| ConfigError::TomlParse {
            path: path.clone(),
            source: e,
        })?;
        let interpolated = interpolate_value(parsed)?;
        let partial: RuntimeConfigPartial =
            interpolated
                .try_into()
                .map_err(|e: toml::de::Error| ConfigError::TomlParse {
                    path: path.clone(),
                    source: e,
                })?;
        tracing::debug!(
            target: "cogito::config",
            path = %path.display(),
            "loaded cogito.toml"
        );
        Ok(partial)
    }
}

/// End-to-end convenience: load File + Env layers, merge, finalize.
/// Available behind `feature = "file"`.
///
/// CLI args are not part of this convenience — the surface (`cogito-cli`)
/// applies its own CLI patch as the highest-precedence layer after this
/// call returns.
pub async fn load_runtime_config<P: AsRef<Path>>(
    config_path: Option<P>,
) -> Result<crate::RuntimeConfig, ConfigError> {
    let file = FileConfigLoader::resolve(config_path)?;
    let env = crate::env::EnvConfigLoader;
    let layers = vec![file.load().await?, env.load().await?];
    let merged = crate::merge::merge_layers(layers);
    merged.finalize()
}

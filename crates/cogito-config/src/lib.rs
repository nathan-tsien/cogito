//! cogito-config — configuration loading for the cogito Agent Runtime.
//!
//! See [`docs/configuration/overview.md`](../../../docs/configuration/overview.md)
//! for the orientation map and ADR-0017 for the architectural anchor.
//!
//! Default features: value types + `ConfigLoader` trait +
//! `EnvConfigLoader` + layered merge. No file-format parsers.
//!
//! Feature `file`: adds `FileConfigLoader` (TOML + YAML), the
//! `${ENV_VAR}` interpolation pass, and the
//! [`load_runtime_config`] convenience.

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

pub mod types;

pub use types::{RuntimeConfig, RuntimeConfigPartial, RuntimeSection, RuntimeSectionPartial};

pub mod loader;

pub use loader::{ConfigError, ConfigLoader};

pub mod env;

pub use env::EnvConfigLoader;

pub mod merge;

pub use merge::merge_layers;

#[cfg(feature = "file")]
pub mod interpolate;

//! Layered partial merge + `finalize` (apply defaults, run minimal
//! validation). See ADR-0017 §3 for precedence semantics.

use std::path::PathBuf;

use crate::loader::ConfigError;
use crate::types::{RuntimeConfig, RuntimeConfigPartial, RuntimeSection, RuntimeSectionPartial};

/// Merge a stack of partials in order. The first element is the lowest
/// precedence (e.g. `defaults`); the last element is the highest
/// (e.g. CLI). Later layers' `Some(_)` overrides earlier layers'.
///
/// Arrays (`providers`) replace wholesale — no element-wise merge.
#[must_use]
pub fn merge_layers(layers: Vec<RuntimeConfigPartial>) -> RuntimeConfigPartial {
    layers
        .into_iter()
        .fold(RuntimeConfigPartial::default(), merge_into)
}

fn merge_into(mut acc: RuntimeConfigPartial, next: RuntimeConfigPartial) -> RuntimeConfigPartial {
    if let Some(rt_next) = next.runtime {
        acc.runtime = Some(merge_runtime(acc.runtime.unwrap_or_default(), rt_next));
    }
    if let Some(providers_next) = next.providers {
        acc.providers = Some(providers_next);
    }
    if let Some(mcp_next) = next.mcp_servers {
        acc.mcp_servers = Some(mcp_next);
    }
    if let Some(skills_next) = next.skills {
        acc.skills = Some(skills_next);
    }
    acc
}

fn merge_runtime(
    mut acc: RuntimeSectionPartial,
    next: RuntimeSectionPartial,
) -> RuntimeSectionPartial {
    if next.session_root.is_some() {
        acc.session_root = next.session_root;
    }
    if next.default_provider.is_some() {
        acc.default_provider = next.default_provider;
    }
    if next.default_model.is_some() {
        acc.default_model = next.default_model;
    }
    if next.default_strategy.is_some() {
        acc.default_strategy = next.default_strategy;
    }
    if next.strategies_dir.is_some() {
        acc.strategies_dir = next.strategies_dir;
    }
    acc
}

impl RuntimeConfigPartial {
    /// Fill defaults and apply minimal validation:
    ///
    /// - `runtime.session_root`   -> `"./sessions"`
    /// - `runtime.strategies_dir` -> `".cogito/strategies"`
    /// - `runtime.default_provider`: if absent AND exactly one provider
    ///   declared, auto-select its name; if absent AND multiple
    ///   providers declared, return `ConfigError::Validation`.
    /// - `runtime.default_model`: kept `None` if absent; surfaces
    ///   may supply via CLI.
    ///
    /// Empty `providers` is **not** an error here — Sprint 4.5's
    /// `cogito-cli` legacy bridge synthesizes a `default` provider
    /// in that case before constructing the gateway.
    pub fn finalize(self) -> Result<RuntimeConfig, ConfigError> {
        let rt = self.runtime.unwrap_or_default();
        let providers = self.providers.unwrap_or_default();

        let mut default_provider = rt.default_provider;
        if default_provider.is_none() && providers.len() >= 2 {
            return Err(ConfigError::Validation(format!(
                "multiple providers declared ({}) but no `default_provider` selected; \
                 set runtime.default_provider in cogito.toml or pass --provider on the CLI",
                providers.len()
            )));
        }
        if default_provider.is_none() && providers.len() == 1 {
            default_provider = Some(providers[0].name().to_string());
        }

        let (mcp_servers, mcp_parse_failures) =
            crate::types::finalize_mcp_servers(self.mcp_servers);

        Ok(RuntimeConfig {
            runtime: RuntimeSection {
                session_root: rt
                    .session_root
                    .unwrap_or_else(|| PathBuf::from("./sessions")),
                default_provider,
                default_model: rt.default_model,
                default_strategy: rt.default_strategy,
                strategies_dir: rt
                    .strategies_dir
                    .unwrap_or_else(|| PathBuf::from(".cogito/strategies")),
            },
            providers,
            strategies: std::collections::HashMap::new(),
            mcp_servers,
            mcp_parse_failures,
            skills: self.skills,
        })
    }
}

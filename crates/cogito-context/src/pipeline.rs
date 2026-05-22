//! `ContextPipeline` factory.

use std::sync::Arc;

use cogito_protocol::context::{
    Compactor, CompactorConfig, ContextConfig, ContextPipeline, HistoryProjector,
    HistoryProjectorConfig, SystemPromptInjector, SystemPromptInjectorConfig, ToolFilterOverrider,
    ToolFilterOverriderConfig,
};

use crate::compactor::{none::NoneCompactor, truncate::TruncateCompactor};
use crate::injector::none::NoneInjector;
use crate::overrider::none::NoneOverrider;
use crate::projector::standard::StandardProjector;

/// Assemble a `ContextPipeline` from a `ContextConfig` by dispatching each
/// tagged variant to the corresponding implementation in this crate.
/// See CLAUDE.md §"Tagged-config factories".
#[must_use]
pub fn build_pipeline(config: &ContextConfig) -> ContextPipeline {
    ContextPipeline {
        compactor: build_compactor(&config.compactor),
        projector: build_projector(&config.history_projector),
        injector: build_injector(&config.system_prompt_injector),
        overrider: build_overrider(&config.tool_filter_overrider),
    }
}

// All build_* helpers use `#[allow(clippy::match_same_arms)]` because the
// enums are `#[non_exhaustive]`: the wildcard arm is a forward-compat fallback
// for variants added in future sprints; its body intentionally duplicates the
// safe default arm rather than panicking on unknown input.

#[allow(clippy::match_same_arms)]
fn build_compactor(cfg: &CompactorConfig) -> Arc<dyn Compactor> {
    match cfg {
        CompactorConfig::None => Arc::new(NoneCompactor),
        CompactorConfig::Truncate(c) => Arc::new(TruncateCompactor::new(c.clone())),
        _ => Arc::new(NoneCompactor),
    }
}

#[allow(clippy::match_same_arms)]
fn build_projector(cfg: &HistoryProjectorConfig) -> Arc<dyn HistoryProjector> {
    match cfg {
        HistoryProjectorConfig::Standard => Arc::new(StandardProjector),
        _ => Arc::new(StandardProjector),
    }
}

#[allow(clippy::match_same_arms)]
fn build_injector(cfg: &SystemPromptInjectorConfig) -> Arc<dyn SystemPromptInjector> {
    match cfg {
        SystemPromptInjectorConfig::None => Arc::new(NoneInjector),
        _ => Arc::new(NoneInjector),
    }
}

#[allow(clippy::match_same_arms)]
fn build_overrider(cfg: &ToolFilterOverriderConfig) -> Arc<dyn ToolFilterOverrider> {
    match cfg {
        ToolFilterOverriderConfig::None => Arc::new(NoneOverrider),
        _ => Arc::new(NoneOverrider),
    }
}

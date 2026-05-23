//! `ContextPipeline` factory.

use std::sync::Arc;

use cogito_protocol::context::{
    Compactor, CompactorConfig, ContextConfig, ContextPipeline, HistoryProjector,
    HistoryProjectorConfig, SystemPromptInjector, SystemPromptInjectorConfig, ToolFilterOverrider,
    ToolFilterOverriderConfig,
};
use cogito_protocol::skill::SkillProvider;
use thiserror::Error;

use crate::compactor::{none::NoneCompactor, truncate::TruncateCompactor};
use crate::injector::none::NoneInjector;
use crate::injector::skill::SkillInjector;
use crate::overrider::none::NoneOverrider;
use crate::projector::standard::StandardProjector;

/// Errors from `build_pipeline_v2`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PipelineBuildError {
    /// `SystemPromptInjectorConfig::Skill` was selected but no `SkillProvider`
    /// was supplied.
    #[error(
        "system_prompt_injector kind = 'skill' requires a SkillProvider to be injected at Runtime build time"
    )]
    MissingSkillProvider,
}

/// Build a `ContextPipeline` from config + optional `SkillProvider`.
///
/// `build_pipeline` (no `_v2` suffix) is preserved for backward compatibility
/// and forwards to this with `skill_provider = None`. Internal callers
/// migrating to Sprint 7 should switch to this entry point.
///
/// # Errors
///
/// Returns [`PipelineBuildError::MissingSkillProvider`] if the config selects
/// `SystemPromptInjectorConfig::Skill` but no `SkillProvider` was supplied.
pub fn build_pipeline_v2(
    config: &ContextConfig,
    skill_provider: Option<Arc<dyn SkillProvider>>,
) -> Result<ContextPipeline, PipelineBuildError> {
    let compactor = build_compactor(&config.compactor);
    let projector = build_projector(&config.history_projector);
    let overrider = build_overrider(&config.tool_filter_overrider);
    // The wildcard arm intentionally mirrors the `None` arm: `SystemPromptInjectorConfig`
    // is `#[non_exhaustive]`, so any future variant must fall back to the safe no-op
    // injector rather than panic. This mirrors the policy used by the private
    // `build_compactor` / `build_projector` / `build_overrider` helpers.
    #[allow(clippy::match_same_arms)]
    let injector: Arc<dyn SystemPromptInjector> = match &config.system_prompt_injector {
        SystemPromptInjectorConfig::None => Arc::new(NoneInjector),
        SystemPromptInjectorConfig::Skill => {
            let p = skill_provider.ok_or(PipelineBuildError::MissingSkillProvider)?;
            Arc::new(SkillInjector::new(p))
        }
        _ => Arc::new(NoneInjector),
    };
    Ok(ContextPipeline {
        compactor,
        projector,
        injector,
        overrider,
    })
}

/// Assemble a `ContextPipeline` from a `ContextConfig` by dispatching each
/// tagged variant to the corresponding implementation in this crate.
///
/// Legacy Sprint 6 entry point: forwards to [`build_pipeline_v2`] with
/// `skill_provider = None`. Callers that need the Skill injector must use
/// [`build_pipeline_v2`] instead.
///
/// See CLAUDE.md §"Tagged-config factories".
///
/// # Panics
///
/// Panics if `config.system_prompt_injector == SystemPromptInjectorConfig::Skill`,
/// since this legacy entry point cannot supply a `SkillProvider`. Use
/// [`build_pipeline_v2`] in that case.
#[must_use]
pub fn build_pipeline(config: &ContextConfig) -> ContextPipeline {
    // Legacy callers force injector = None semantics: the only way Skill
    // could appear here is misconfiguration of an unmigrated caller.
    #[allow(clippy::expect_used)]
    {
        build_pipeline_v2(config, None).expect("legacy build_pipeline forces injector=None")
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
fn build_overrider(cfg: &ToolFilterOverriderConfig) -> Arc<dyn ToolFilterOverrider> {
    match cfg {
        ToolFilterOverriderConfig::None => Arc::new(NoneOverrider),
        _ => Arc::new(NoneOverrider),
    }
}

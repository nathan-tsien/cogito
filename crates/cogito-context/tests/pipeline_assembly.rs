//! Integration tests for `build_pipeline` factory.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]

use std::sync::Arc;

use cogito_context::build_pipeline;
use cogito_protocol::context::{
    CompactorConfig, ContextConfig, SystemPromptInjectorConfig, TokenThreshold, TruncateConfig,
};
use cogito_protocol::skill::SkillProvider;

#[test]
fn default_config_assembles_no_op_pipeline() {
    let p = build_pipeline(&ContextConfig::default());
    assert_eq!(p.compactor.id(), "none");
    assert_eq!(p.projector.id(), "standard");
    assert_eq!(p.injector.id(), "none");
    assert_eq!(p.overrider.id(), "none");
}

#[test]
fn truncate_config_assembles_truncate_compactor() {
    let mut cfg = ContextConfig::default();
    cfg.compactor = CompactorConfig::Truncate(TruncateConfig {
        max_tokens: TokenThreshold::default(),
        keep_first_user: true,
        keep_recent_turns: 5,
    });
    let p = build_pipeline(&cfg);
    assert_eq!(p.compactor.id(), "truncate");
}

struct EmptyProvider;
impl SkillProvider for EmptyProvider {
    fn list(&self) -> Vec<cogito_protocol::skill::SkillMetadata> {
        vec![]
    }
    fn get(&self, _: &str) -> Option<cogito_protocol::skill::SkillContent> {
        None
    }
    fn is_registered(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn skill_injector_requires_provider() {
    let mut cfg = ContextConfig::default();
    cfg.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    // `ContextPipeline` does not implement `Debug`, so we cannot use
    // `.unwrap_err()` here; pattern-match instead.
    let result = cogito_context::build_pipeline_v2(&cfg, None);
    let Err(err) = result else {
        panic!("expected MissingSkillProvider error, got Ok")
    };
    assert!(err.to_string().to_lowercase().contains("skill"));
}

#[test]
fn skill_injector_builds_with_provider() {
    let mut cfg = ContextConfig::default();
    cfg.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    let provider: Arc<dyn SkillProvider> = Arc::new(EmptyProvider);
    let pipeline = cogito_context::build_pipeline_v2(&cfg, Some(provider)).unwrap();
    assert_eq!(pipeline.injector.id(), "skill");
}

#[test]
fn none_injector_works_without_provider() {
    let cfg = ContextConfig::default();
    let pipeline = cogito_context::build_pipeline_v2(&cfg, None).unwrap();
    assert_eq!(pipeline.injector.id(), "none");
}

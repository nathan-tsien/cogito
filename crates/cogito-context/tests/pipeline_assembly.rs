//! Integration tests for `build_pipeline` factory.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]

use cogito_context::build_pipeline;
use cogito_protocol::context::{CompactorConfig, ContextConfig, TokenThreshold, TruncateConfig};

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

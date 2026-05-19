//! H09 Hook Pipeline — Sprint 2 ships no-op insertion points. Real hooks
//! land in Sprint 6 with the `HookHandler` trait.
//!
//! See `docs/components/H09-hook-pipeline.md`.

use cogito_protocol::gateway::ModelInput;

/// Hook decision shape. Sprint 2 hooks always return `Allow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    /// Continue the normal pipeline flow.
    Allow,
    /// Abort the pipeline with the given reason.
    Reject {
        /// Human-readable reason for the rejection.
        reason: String,
    },
}

/// No-op hook pipeline. All lifecycle methods return `Allow`.
#[derive(Debug, Default, Clone)]
pub struct HookPipeline;

impl HookPipeline {
    /// Create a new no-op pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Runs at `ContextManaged -> PromptBuilt`. v0.1 no-op.
    #[must_use]
    pub fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs before each tool dispatch. v0.1 no-op.
    #[must_use]
    pub fn pre_dispatch(&self, _call_id: &str, _name: &str) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs after model stream completes. v0.1 no-op.
    pub fn post_model(&self) {}

    /// Runs at terminal turn states. v0.1 no-op.
    pub fn post_turn(&self) {}

    /// Runs on `Failed`. v0.1 no-op.
    pub fn on_error(&self, _reason: &str) {}
}

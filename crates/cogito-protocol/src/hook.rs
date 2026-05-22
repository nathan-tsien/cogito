//! H09 Hook lifecycle contract.
//!
//! `HookHandler` is a pure, synchronous policy gate (see ADR-0004 §6 and
//! `docs/components/H09-hook-pipeline.md`). Implementations may inspect
//! turn state and return `Allow` or `Reject`; they may NOT perform I/O.
//! Side effects belong in `ToolProvider` / `JobManager`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::gateway::ModelInput;

/// One lifecycle point at which the Hook Pipeline is invoked.
///
/// Persisted into `EventPayload::HookRejected` to make rejection events
/// fully reconstructable from the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HookLifecyclePoint {
    /// Fires at the end of prompt build (`ContextManaged` -> `PromptBuilt`).
    PrePrompt,
    /// Fires before each tool dispatch.
    PreDispatch,
    /// Fires after model stream completion.
    PostModel,
    /// Fires at terminal Completed / Paused states.
    PostTurn,
    /// Fires at terminal Failed state.
    OnError,
}

/// Decision returned by a `HookHandler`.
///
/// `#[non_exhaustive]` so future variants (`Modify`, etc. -- see H09 doc
/// section "Open design questions") can be added additively.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookDecision {
    /// Continue the normal pipeline flow.
    Allow,
    /// Abort the pipeline with the given reason.
    Reject {
        /// Name of the hook that rejected (from `HookHandler::name()`).
        /// Recorded in `EventPayload::HookRejected.hook_name` and
        /// surfaced via `TurnFailureReason::HookRejected.hook_name`.
        hook_name: String,
        /// Human-readable reason for the rejection. Recorded in the
        /// `HookRejected` event and surfaced via `TurnFailureReason`.
        reason: String,
    },
}

/// Brain-side policy gate. All methods MUST be free of I/O.
///
/// Default impls return `Allow` / no-op so implementers override only
/// the lifecycle points they care about.
pub trait HookHandler: Send + Sync {
    /// Stable identifier used in events and metrics. SHOULD be
    /// kebab-case and unique within a deployment.
    fn name(&self) -> &str;

    /// Runs at `ContextManaged -> PromptBuilt`.
    fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs before each tool dispatch.
    fn pre_dispatch(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs after model stream completion. Observation-only.
    fn post_model(&self) {}

    /// Runs at terminal Completed / Paused. Observation-only.
    fn post_turn(&self) {}

    /// Runs at terminal Failed. Observation-only.
    fn on_error(&self, _reason: &str) {}
}

/// Aggregation surface for `HookHandler` providers. Used by the runtime
/// to build a `CompositeHookPipeline` from one or more sources
/// (Sprint 5: built-ins; v0.2 Plugin: plugin-bundled hooks).
pub trait HookProvider: Send + Sync {
    /// Returns all handlers the provider contributes.
    fn list(&self) -> Vec<Arc<dyn HookHandler>>;
}

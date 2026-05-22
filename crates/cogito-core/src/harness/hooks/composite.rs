//! `CompositeHookPipeline` — the runtime invocation surface for H09.
//!
//! Holds a `Vec<Arc<dyn HookHandler>>` plus an optional
//! `Arc<dyn MetricsRecorder>`. Each lifecycle method iterates
//! handlers, applies panic catch, times each call, records metrics,
//! and short-circuits on the first `Reject`. See Task 5 for the full
//! implementation.

use std::sync::Arc;

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler};
use cogito_protocol::metrics::{MetricsRecorder, NoOpMetricsRecorder};

/// Composite hook pipeline that runs all registered handlers in order.
///
/// In this task the lifecycle methods are stubs returning `Allow`.
/// Full implementation (panic catch, metrics, short-circuit) lands in Task 5.
#[allow(dead_code)]
#[derive(Clone)]
pub struct CompositeHookPipeline {
    handlers: Vec<Arc<dyn HookHandler>>,
    metrics: Arc<dyn MetricsRecorder>,
}

impl CompositeHookPipeline {
    /// Creates a pipeline with the given handlers and the default no-op metrics recorder.
    #[must_use]
    pub fn with_handlers(handlers: Vec<Arc<dyn HookHandler>>) -> Self {
        Self {
            handlers,
            metrics: Arc::new(NoOpMetricsRecorder),
        }
    }

    /// Creates a pipeline with explicit handlers and metrics recorder.
    #[must_use]
    pub fn with_handlers_and_metrics(
        handlers: Vec<Arc<dyn HookHandler>>,
        metrics: Arc<dyn MetricsRecorder>,
    ) -> Self {
        Self { handlers, metrics }
    }

    /// Returns the number of registered handlers.
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }

    /// Stub — full impl in Task 5.
    #[must_use]
    pub fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        HookDecision::Allow
    }

    /// Stub — full impl in Task 5.
    #[must_use]
    pub fn pre_dispatch(&self, _call_id: &str, _name: &str) -> HookDecision {
        HookDecision::Allow
    }

    // Lifecycle methods fully implemented in Task 5.
}

impl Default for CompositeHookPipeline {
    fn default() -> Self {
        Self::with_handlers(Vec::new())
    }
}

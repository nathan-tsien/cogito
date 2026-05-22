//! `CompositeHookPipeline` — the runtime invocation surface for H09.
//!
//! Holds a `Vec<Arc<dyn HookHandler>>` plus an optional
//! `Arc<dyn MetricsRecorder>`. Each lifecycle method iterates
//! handlers, applies panic catch, times each call, records metrics,
//! and short-circuits on the first `Reject`.

use std::sync::Arc;
use std::time::Instant;

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint};
use cogito_protocol::metrics::{MetricsRecorder, NoOpMetricsRecorder};

use super::panic_catch;

/// Composite hook pipeline that runs all registered handlers in order.
///
/// Each lifecycle method iterates handlers in registration order, applies
/// panic catch, times each call, records metrics, and short-circuits on
/// the first `Reject`.
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

    /// Runs the `pre_prompt` hook for each handler in order.
    ///
    /// Returns the first `Reject` encountered (short-circuit), or `Allow`
    /// if all handlers pass. Panics in handlers are caught and converted to
    /// `Reject`.
    #[must_use]
    pub fn pre_prompt(&self, input: &ModelInput) -> HookDecision {
        for handler in &self.handlers {
            let start = Instant::now();
            let dec = panic_catch::wrap_pre_prompt(handler.as_ref(), input);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PrePrompt,
                handler.name(),
                elapsed,
                matches!(dec, HookDecision::Allow),
            );
            if let HookDecision::Reject { .. } = &dec {
                return dec;
            }
        }
        HookDecision::Allow
    }

    /// Runs the `pre_dispatch` hook for each handler in order.
    ///
    /// Returns the first `Reject` encountered (short-circuit), or `Allow`
    /// if all handlers pass. Panics in handlers are caught and converted to
    /// `Reject`.
    #[must_use]
    pub fn pre_dispatch(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> HookDecision {
        for handler in &self.handlers {
            let start = Instant::now();
            let dec = panic_catch::wrap_pre_dispatch(handler.as_ref(), call_id, tool_name, args);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PreDispatch,
                handler.name(),
                elapsed,
                matches!(dec, HookDecision::Allow),
            );
            if let HookDecision::Reject { .. } = &dec {
                return dec;
            }
        }
        HookDecision::Allow
    }

    /// Runs `post_model` for each handler in order.
    ///
    /// Panics are caught and suppressed; the turn has already produced model
    /// output so a hook panic must not abort it.
    pub fn post_model(&self) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_post_model(handler.as_ref());
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PostModel,
                handler.name(),
                elapsed,
                true,
            );
        }
    }

    /// Runs `post_turn` for each handler in order.
    ///
    /// Panics are caught and suppressed.
    pub fn post_turn(&self) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_post_turn(handler.as_ref());
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PostTurn,
                handler.name(),
                elapsed,
                true,
            );
        }
    }

    /// Runs `on_error` for each handler in order.
    ///
    /// Panics are caught and suppressed.
    pub fn on_error(&self, reason: &str) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_on_error(handler.as_ref(), reason);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::OnError,
                handler.name(),
                elapsed,
                true,
            );
        }
    }
}

impl Default for CompositeHookPipeline {
    fn default() -> Self {
        Self::with_handlers(Vec::new())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use cogito_protocol::gateway::ModelInput;
    use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint};
    use cogito_protocol::metrics::MetricsRecorder;

    use super::*;

    struct CallCounter(AtomicUsize);
    impl HookHandler for CallCounter {
        fn name(&self) -> &'static str {
            "counter"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            self.0.fetch_add(1, Ordering::SeqCst);
            HookDecision::Allow
        }
    }

    struct Reject;
    impl HookHandler for Reject {
        fn name(&self) -> &'static str {
            "rejecter"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            HookDecision::Reject {
                hook_name: "rejecter".into(),
                reason: "nope".into(),
            }
        }
    }

    struct Panicky;
    impl HookHandler for Panicky {
        fn name(&self) -> &'static str {
            "panicky"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            panic!("boom")
        }
    }

    struct CountingMetrics {
        invocations: AtomicUsize,
        rejections: AtomicUsize,
    }
    impl MetricsRecorder for CountingMetrics {
        fn record_hook_invocation(
            &self,
            _: HookLifecyclePoint,
            _: &str,
            _: Duration,
            allowed: bool,
        ) {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            if !allowed {
                self.rejections.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn record_counter(&self, _: &str, _: &[(&str, &str)]) {}
    }

    #[test]
    fn empty_pipeline_returns_allow() {
        let p = CompositeHookPipeline::default();
        assert!(matches!(
            p.pre_prompt(&ModelInput::default()),
            HookDecision::Allow
        ));
    }

    #[test]
    fn first_reject_short_circuits() {
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers(vec![
            Arc::new(Reject) as Arc<dyn HookHandler>,
            counter.clone() as Arc<dyn HookHandler>,
        ]);
        let dec = p.pre_prompt(&ModelInput::default());
        assert!(matches!(dec, HookDecision::Reject { .. }));
        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            0,
            "second hook should be skipped"
        );
    }

    #[test]
    fn panicked_hook_becomes_reject_and_short_circuits() {
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers(vec![
            Arc::new(Panicky) as Arc<dyn HookHandler>,
            counter.clone() as Arc<dyn HookHandler>,
        ]);
        let dec = p.pre_prompt(&ModelInput::default());
        match dec {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "panicky");
                assert!(reason.contains("panicky"));
            }
            _ => panic!("expected Reject"),
        }
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn metrics_record_every_invocation() {
        let metrics = Arc::new(CountingMetrics {
            invocations: AtomicUsize::new(0),
            rejections: AtomicUsize::new(0),
        });
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers_and_metrics(
            vec![counter as Arc<dyn HookHandler>],
            metrics.clone() as Arc<dyn MetricsRecorder>,
        );
        let _ = p.pre_prompt(&ModelInput::default());
        assert_eq!(metrics.invocations.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.rejections.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn metrics_record_rejection() {
        let metrics = Arc::new(CountingMetrics {
            invocations: AtomicUsize::new(0),
            rejections: AtomicUsize::new(0),
        });
        let p = CompositeHookPipeline::with_handlers_and_metrics(
            vec![Arc::new(Reject) as Arc<dyn HookHandler>],
            metrics.clone() as Arc<dyn MetricsRecorder>,
        );
        let _ = p.pre_prompt(&ModelInput::default());
        assert_eq!(metrics.rejections.load(Ordering::SeqCst), 1);
    }
}

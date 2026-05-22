//! Pluggable metrics sink.
//!
//! v0.1 ships `NoOpMetricsRecorder` as the default. Real adapters
//! (OpenTelemetry / Prometheus) land in v0.4 per ROADMAP -- but the
//! trait surface is frozen here in Sprint 5 so the Hook Pipeline
//! can emit measurements from day one.

use std::time::Duration;

use crate::hook::HookLifecyclePoint;

/// Sink that consumes metric samples emitted by the Brain.
///
/// All methods MUST be synchronous and non-blocking. Implementers
/// should buffer + drain off-task if their backend has latency.
pub trait MetricsRecorder: Send + Sync {
    /// Records one hook invocation at the given lifecycle point.
    ///
    /// - `duration` measures the hook's own execution time only
    ///   (excludes pipeline overhead).
    /// - `allowed = true` for `HookDecision::Allow`; `false` for
    ///   `HookDecision::Reject` (including panic-induced rejection).
    fn record_hook_invocation(
        &self,
        point: HookLifecyclePoint,
        hook_name: &str,
        duration: Duration,
        allowed: bool,
    );

    /// Increments a named counter with optional labels.
    fn record_counter(&self, name: &str, labels: &[(&str, &str)]);
}

/// Default no-op implementation. Used by the runtime when no metrics
/// adapter is configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpMetricsRecorder;

impl MetricsRecorder for NoOpMetricsRecorder {
    fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, _: bool) {}
    fn record_counter(&self, _: &str, _: &[(&str, &str)]) {}
}

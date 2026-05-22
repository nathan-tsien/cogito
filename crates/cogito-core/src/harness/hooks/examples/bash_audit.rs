//! `BashAuditHook` — increments `cogito.tool.bash.invocations` for
//! every `bash` tool call. Never rejects (audit-only).

use std::sync::Arc;

use cogito_protocol::hook::{HookDecision, HookHandler};
use cogito_protocol::metrics::MetricsRecorder;

const HOOK_NAME: &str = "bash-audit";
const COUNTER_NAME: &str = "cogito.tool.bash.invocations";

/// Audit hook for `bash` tool invocations.
pub struct BashAuditHook {
    metrics: Arc<dyn MetricsRecorder>,
}

impl BashAuditHook {
    /// Creates a new [`BashAuditHook`] that reports to the given metrics recorder.
    #[must_use]
    pub fn new(metrics: Arc<dyn MetricsRecorder>) -> Self {
        Self { metrics }
    }
}

impl HookHandler for BashAuditHook {
    fn name(&self) -> &str {
        HOOK_NAME
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        tool_name: &str,
        _args: &serde_json::Value,
    ) -> HookDecision {
        if tool_name == "bash" {
            self.metrics.record_counter(COUNTER_NAME, &[]);
        }
        HookDecision::Allow
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint};
    use cogito_protocol::metrics::MetricsRecorder;
    use serde_json::json;

    use super::*;

    struct CounterMetrics {
        bash: AtomicUsize,
        other: AtomicUsize,
    }
    impl MetricsRecorder for CounterMetrics {
        fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, _: bool) {}
        fn record_counter(&self, name: &str, _: &[(&str, &str)]) {
            if name == "cogito.tool.bash.invocations" {
                self.bash.fetch_add(1, Ordering::SeqCst);
            } else {
                self.other.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn bash_increments_counter() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m.clone() as Arc<dyn MetricsRecorder>);
        let dec = h.pre_dispatch("c1", "bash", &json!({"cmd": "ls"}));
        assert!(matches!(dec, HookDecision::Allow));
        assert_eq!(m.bash.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_bash_does_not_increment() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m.clone() as Arc<dyn MetricsRecorder>);
        let _ = h.pre_dispatch("c1", "read_file", &json!({"path": "/tmp/x"}));
        assert_eq!(m.bash.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn always_allows() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m as Arc<dyn MetricsRecorder>);
        let dec = h.pre_dispatch("c1", "bash", &json!({"cmd": "rm -rf /"}));
        assert!(
            matches!(dec, HookDecision::Allow),
            "audit must never reject"
        );
    }
}

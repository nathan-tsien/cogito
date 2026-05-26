//! Sleep tool — deterministic async-tool fixture for integration and
//! chaos tests. Behind the `test-tools` Cargo feature so it does not
//! ship in production builds.
//!
//! Shape: a one-tool [`ToolProvider`] named `sleep` whose schema accepts
//! `{ "duration_ms": <u64> }`. Every invocation submits a `tokio::time::sleep`
//! future into the supplied [`LocalJobManager`] and returns
//! [`InvokeOutcome::Async`]; when the future resolves the job outcome is
//! `JobOutcome::Success { result: ToolResult::text("slept") }`. Implementing
//! `ToolProvider` directly (rather than going through a `BuiltinTool`-style
//! sync trait in `cogito-tools`) keeps `cogito-jobs` free of a layering
//! dependency on `cogito-tools` and lets tests wire the fixture as
//! `Arc<dyn ToolProvider>` without an adapter.

#![cfg(any(test, feature = "test-tools"))]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::job::JobOutcome;
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use serde::Deserialize;

use crate::LocalJobManager;

/// Tool name exposed to the model.
const TOOL_NAME: &str = "sleep";

/// Arguments accepted by [`SleepTool`].
#[derive(Debug, Deserialize)]
struct SleepArgs {
    /// Number of milliseconds to sleep before resolving the job.
    duration_ms: u64,
}

/// Deterministic async-tool fixture. Submits a `tokio::time::sleep` future
/// into the wrapped [`LocalJobManager`] and returns the resulting `JobId`
/// to the dispatcher.
///
/// Construct via [`SleepTool::new`] with the same `Arc<LocalJobManager>`
/// that the `RuntimeBuilder` will receive — otherwise the job submitted
/// here is invisible to the Brain registering `on_complete`.
pub struct SleepTool {
    job_mgr: Arc<LocalJobManager>,
}

impl SleepTool {
    /// Build a new `SleepTool` bound to `job_mgr`.
    #[must_use]
    pub fn new(job_mgr: Arc<LocalJobManager>) -> Self {
        Self { job_mgr }
    }
}

#[async_trait]
impl ToolProvider for SleepTool {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: TOOL_NAME.into(),
            description: "Sleep for the specified number of milliseconds (test fixture).".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "duration_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Milliseconds to sleep before completing."
                    }
                },
                "required": ["duration_ms"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysAsync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, _ctx: ExecCtx) -> InvokeOutcome {
        if name != TOOL_NAME {
            return InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("unknown tool: {name}"),
                retryable: false,
            });
        }
        let parsed: SleepArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return InvokeOutcome::Sync(ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("sleep args: {e}"),
                    retryable: false,
                });
            }
        };
        let dur = Duration::from_millis(parsed.duration_ms);
        let job_id = self.job_mgr.submit(async move {
            tokio::time::sleep(dur).await;
            JobOutcome::Success {
                result: ToolResult::text("slept"),
            }
        });
        InvokeOutcome::Async(job_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::job::{JobManager, JobStatus};

    fn ctx() -> ExecCtx {
        ExecCtx::open_ended(SessionId::new(), TurnId::new())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn descriptor_shape() {
        let tool = SleepTool::new(LocalJobManager::new());
        let descs = tool.list();
        assert_eq!(descs.len(), 1);
        let d = &descs[0];
        assert_eq!(d.name, "sleep");
        assert_eq!(d.execution_class, ExecutionClass::AlwaysAsync);
        assert!(!d.outputs_model_visible_multimodal);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invoke_returns_async_and_resolves_with_slept() {
        let job_mgr = LocalJobManager::new();
        let tool = SleepTool::new(Arc::clone(&job_mgr));
        let outcome = tool
            .invoke("sleep", serde_json::json!({ "duration_ms": 10 }), ctx())
            .await;
        let job_id = match outcome {
            InvokeOutcome::Async(id) => id,
            other => panic!("expected Async, got {other:?}"),
        };
        // Poll for completion (bounded to avoid hangs).
        for _ in 0..50 {
            if matches!(job_mgr.status(job_id).await.unwrap(), JobStatus::Completed) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let result = job_mgr.result(job_id).await.unwrap();
        match result {
            JobOutcome::Success { result } => {
                assert_eq!(result, ToolResult::text("slept"));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invoke_with_bad_args_returns_sync_error() {
        let tool = SleepTool::new(LocalJobManager::new());
        let outcome = tool
            .invoke("sleep", serde_json::json!({ "wrong": "field" }), ctx())
            .await;
        match outcome {
            InvokeOutcome::Sync(ToolResult::Error {
                kind, retryable, ..
            }) => {
                assert_eq!(kind, ToolErrorKind::InvalidArgs);
                assert!(!retryable);
            }
            other => panic!("expected Sync(Error), got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_tool_name_returns_sync_error() {
        let tool = SleepTool::new(LocalJobManager::new());
        let outcome = tool
            .invoke("other", serde_json::json!({ "duration_ms": 0 }), ctx())
            .await;
        match outcome {
            InvokeOutcome::Sync(ToolResult::Error { kind, .. }) => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
            }
            other => panic!("expected Sync(Error), got {other:?}"),
        }
    }
}

//! H08 Tool Dispatcher — sync path with `catch_unwind`. Sprint 4 wires
//! the async path via `JobManager`.
//!
//! See `docs/components/H08-tool-dispatcher.md`.

use std::panic::AssertUnwindSafe;

use cogito_protocol::job::JobId;
use cogito_protocol::tool::{ExecutionClass, InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use cogito_protocol::ExecCtx;
use futures::FutureExt;

use crate::harness::tool_resolver::ToolInvocation;

/// Outcome of a single dispatch attempt.
#[derive(Debug)]
#[non_exhaustive]
pub enum DispatchOutcome {
    /// Invocation completed synchronously (success or structured error).
    SyncResult(ToolResult),
    /// Invocation was offloaded to an async job.
    AsyncJob(JobId),
}

/// Dispatch one validated tool call.
///
/// - `AlwaysSync` / `Adaptive` tools: invoked immediately with
///   `catch_unwind`; panics are turned into `ToolResult::Error`.
/// - `AlwaysAsync` tools and calls that return `InvokeOutcome::Async`:
///   return a structured error for now (Sprint 4 wires `JobManager`).
pub async fn dispatch(
    inv: ToolInvocation,
    provider: &dyn ToolProvider,
    ctx: ExecCtx,
) -> DispatchOutcome {
    let descriptors = provider.list();
    let class = descriptors
        .iter()
        .find(|d| d.name == inv.name)
        .map_or(ExecutionClass::AlwaysSync, |d| d.execution_class);

    if matches!(class, ExecutionClass::AlwaysAsync) {
        return DispatchOutcome::SyncResult(async_not_supported(&inv.name));
    }

    let name = inv.name.clone();
    let args = inv.args.clone();
    let caught = AssertUnwindSafe(provider.invoke(&name, args, ctx))
        .catch_unwind()
        .await;

    let outcome = match caught {
        Ok(o) => o,
        Err(payload) => {
            return DispatchOutcome::SyncResult(ToolResult::Error {
                kind: ToolErrorKind::ToolPanicked,
                message: format!("tool `{name}` panicked: {}", panic_msg(&payload)),
                retryable: false,
            });
        }
    };

    match outcome {
        InvokeOutcome::Sync(result) => DispatchOutcome::SyncResult(result),
        // InvokeOutcome::Async and any future variants (#[non_exhaustive]) are
        // unsupported until Sprint 4 wires JobManager.
        _ => DispatchOutcome::SyncResult(async_not_supported(&name)),
    }
}

/// Build a structured error for tools that require async dispatch, which is
/// not wired until Sprint 4.
fn async_not_supported(name: &str) -> ToolResult {
    ToolResult::Error {
        kind: ToolErrorKind::InvocationFailed,
        message: format!(
            "tool `{name}` returned Async, but JobManager is not wired in Sprint 2"
        ),
        retryable: false,
    }
}

/// Extract a human-readable string from a panic payload.
fn panic_msg(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).into()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".into()
    }
}

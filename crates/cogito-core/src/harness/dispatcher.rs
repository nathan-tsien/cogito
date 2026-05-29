//! H08 Tool Dispatcher.
//!
//! Sync path: `catch_unwind` wraps the provider call; panics become
//! `ToolResult::Error { kind: ToolPanicked }`.
//!
//! Async path (Sprint 4 → Sprint 8 wiring): when the provider returns
//! `InvokeOutcome::Async(job_id)`, the dispatcher:
//!
//! 1. Records `EventPayload::JobSubmitted` via [`StepRecorder`] BEFORE
//!    registering the `on_complete` sink. The write-before-transition rule
//!    (ADR-0003) means the log entry must hit the store before any side
//!    effect that could later be observed externally — here, before the
//!    `JobManager` is allowed to fan out a completion event onto the
//!    session mailbox.
//! 2. Registers the per-session completion sink with the `JobManager`.
//! 3. Returns `DispatchOutcome::AsyncJob(job_id)`. The caller (the
//!    `ToolDispatching` transition) translates this into
//!    `TurnState::Paused { job_id }`, which the outer FSM loop converts
//!    into `TurnOutcome::Paused`. `on_turn_complete` then parks the
//!    session as `InFlight::PausedOnJob`.
//!
//! If recording or `on_complete` registration fails, the dispatcher
//! synthesises a `ToolResult::Error` so the turn surfaces a recoverable
//! error rather than hanging. On `on_complete` failure the partially
//! submitted job is best-effort cancelled so workers do not run for a
//! tool result no one will read.
//!
//! See `docs/components/H08-tool-dispatcher.md`.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::TurnId;
use cogito_protocol::job::{JobCompletionEvent, JobId, JobManager};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use futures::FutureExt;
use tokio::sync::{Mutex, mpsc};

use crate::harness::step_recorder::StepRecorder;
use crate::harness::tool_resolver::ToolInvocation;

/// Outcome of a single dispatch attempt.
#[derive(Debug)]
#[non_exhaustive]
pub enum DispatchOutcome {
    /// Invocation completed synchronously (success or structured error).
    SyncResult(ToolResult),
    /// Invocation was offloaded to an async job; the turn will pause on it.
    AsyncJob(JobId),
}

/// Dispatch one validated tool call.
///
/// - `AlwaysSync` / `Adaptive` tools: invoked immediately with
///   `catch_unwind`; panics are turned into `ToolResult::Error`.
/// - `AlwaysAsync` tools and calls that return `InvokeOutcome::Async`:
///   record `JobSubmitted`, register the completion sink on `job_mgr`,
///   and return `DispatchOutcome::AsyncJob` so the turn pauses.
pub async fn dispatch(
    inv: ToolInvocation,
    provider: &dyn ToolProvider,
    ctx: ExecCtx,
    job_mgr: &dyn JobManager,
    job_completion_tx: &mpsc::Sender<JobCompletionEvent>,
    recorder: &Arc<Mutex<StepRecorder>>,
    turn_id: TurnId,
) -> DispatchOutcome {
    // Sprint 8 wired the JobManager, so `AlwaysAsync` tools no longer
    // short-circuit; they flow through the normal `invoke` path and
    // return `InvokeOutcome::Async(job_id)` which the async-handling arm
    // below takes care of. The `ExecutionClass` declared on the descriptor
    // is now purely advisory for surfaces (e.g. H05 surface filtering).

    let name = inv.name.clone();
    let call_id = inv.call_id.clone();
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
        InvokeOutcome::Async(job_id) => {
            // Write-before-transition (ADR-0003): persist the JobSubmitted
            // event before registering the completion sink. If the recorder
            // fails, surface a structured error and let the model continue
            // — better than a turn that pauses on a job the log does not
            // know about.
            {
                let mut rec = recorder.lock().await;
                if let Err(e) = rec
                    .record_job_submitted(turn_id, call_id.clone(), job_id, name.clone())
                    .await
                {
                    return DispatchOutcome::SyncResult(ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: format!("failed to record JobSubmitted: {e}"),
                        retryable: false,
                    });
                }
            }
            if let Err(e) = job_mgr.on_complete(job_id, job_completion_tx.clone()).await {
                tracing::error!(%job_id, error = %e, "on_complete registration failed");
                // Best-effort cancel: we have submitted but cannot observe
                // completion, so let the worker stop instead of doing work
                // whose result nobody will read.
                let _ = job_mgr.cancel(job_id).await;
                return DispatchOutcome::SyncResult(ToolResult::Error {
                    kind: ToolErrorKind::AsyncFailed,
                    message: format!("on_complete failed: {e}"),
                    retryable: false,
                });
            }
            DispatchOutcome::AsyncJob(job_id)
        }
        // `InvokeOutcome` is `#[non_exhaustive]`; any future variant that we
        // do not yet know how to dispatch surfaces as a structured error.
        _ => DispatchOutcome::SyncResult(ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            message: format!("tool `{name}` returned an unsupported InvokeOutcome variant"),
            retryable: false,
        }),
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

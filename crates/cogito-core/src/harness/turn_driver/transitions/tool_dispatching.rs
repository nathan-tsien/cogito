//! `ToolDispatching → Init` transition.
//!
//! Drains the pending tool call queue via H08 dispatcher, recording each
//! result via the step recorder. After all calls are processed, returns
//! `Init` so the outer FSM loop sends the results back to the model.

use std::collections::VecDeque;

use cogito_protocol::tool::{ToolDescriptor, ToolErrorKind, ToolResult};

use crate::harness::dispatcher::{DispatchOutcome, dispatch};
use crate::harness::hooks::HookDecision;
use crate::harness::resume::{ResumeDecision, ResumePoint};
use crate::harness::tool_resolver::ToolInvocation;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `ToolDispatching` back to `Init`.
///
/// For each pending invocation:
/// 1. Runs `pre_dispatch` hook; on `Reject`, records a structured error and
///    skips dispatch.
/// 2. Calls H08 `dispatch`; translates the outcome into a `ToolResult`.
/// 3. Calls `recorder.record_tool_result` (write before re-entering, ADR-0003).
///
/// After draining all pending calls, returns `TurnState::Init` so the model
/// receives the results in the next inner-loop iteration.
pub async fn transit(
    ctx: TurnCtx,
    mut pending: VecDeque<ToolInvocation>,
    mut completed: Vec<(String, ToolResult)>,
    _surface: Vec<ToolDescriptor>,
    deps: &TurnDeps,
) -> TurnState {
    while let Some(inv) = pending.pop_front() {
        match deps.hooks.pre_dispatch(&inv.call_id, &inv.name) {
            HookDecision::Allow => {}
            HookDecision::Reject { reason } => {
                let result = ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("rejected by pre_dispatch hook: {reason}"),
                    retryable: false,
                };
                // Write tool result before continuing (ADR-0003).
                let _ = deps
                    .step
                    .lock()
                    .await
                    .record_tool_result(ctx.turn_id, inv.call_id.clone(), result.clone())
                    .await;
                completed.push((inv.call_id, result));
                continue;
            }
        }

        let result = match dispatch(inv.clone(), deps.tools.as_ref(), ctx.exec_ctx.clone()).await {
            DispatchOutcome::SyncResult(r) => r,
            DispatchOutcome::AsyncJob(_job_id) => {
                // Async jobs are not wired until Sprint 4. Return a structured
                // error so the model can be informed without aborting the turn.
                ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!(
                        "tool `{}` returned Async; JobManager is not wired in Sprint 2",
                        inv.name
                    ),
                    retryable: false,
                }
            }
        };

        // Write tool result before advancing (ADR-0003).
        let _ = deps
            .step
            .lock()
            .await
            .record_tool_result(ctx.turn_id, inv.call_id.clone(), result.clone())
            .await;
        completed.push((inv.call_id, result));
    }

    // All tool calls are done. Re-enter Init so the model sees the results.
    // Sprint 2 keeps one `turn_id` per TurnDriver task; inner-loop
    // iterations share it. Sprint 3 may mint a fresh turn_id per inner loop.
    TurnState::Init {
        ctx,
        resume: ResumeDecision {
            point: ResumePoint::FreshTurn,
            last_event_seq: None,
        },
    }
}

//! `ModelCompleted → Completed | ToolDispatching` transition.
//!
//! Runs H07 Tool Call Resolver over the model output. If no tool calls
//! are present, records `TurnCompleted` and moves to `Completed`. If tool
//! calls are present, moves to `ToolDispatching`.

use std::collections::VecDeque;

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::ids::EventId;
use cogito_protocol::tool::ToolDescriptor;
use cogito_protocol::turn::TurnFailureReason;

use crate::harness::tool_resolver::{ResolvedCall, resolve};
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{MAX_CONSECUTIVE_TOOL_ERRORS, TurnCtx, TurnState};

/// Transition from `ModelCompleted` to `Completed` or `ToolDispatching`.
///
/// Writes `TurnCompleted` before entering `Completed` (ADR-0003). When
/// tool calls are pending, the dispatch loop writes per-tool events itself.
///
/// H07 validation errors are persisted here (not in the dispatch loop) so
/// the event log always has a `ToolResultRecorded` for every `ToolUseRecorded`.
/// Without this, the second model request would send an assistant message with
/// `tool_calls` but no matching `role: "tool"` results, causing a 400 from
/// OpenAI-compatible providers.
pub async fn transit(
    ctx: TurnCtx,
    output: ModelOutput,
    surface: Vec<ToolDescriptor>,
    deps: &TurnDeps,
) -> TurnState {
    let assistant_content = output.content.clone();
    let mut pending: VecDeque<crate::harness::tool_resolver::ToolInvocation> = VecDeque::new();
    let mut errors: Vec<(String, cogito_protocol::tool::ToolResult)> = Vec::new();

    for block in &output.content {
        if let ContentBlock::ToolUse {
            call_id,
            tool_name,
            args,
        } = block
        {
            match resolve(call_id, tool_name, args.clone(), &surface) {
                ResolvedCall::Ok(inv) => pending.push_back(inv),
                ResolvedCall::Error(err) => {
                    // Persist immediately (ADR-0003): every ToolUseRecorded must
                    // have a matching ToolResultRecorded before the next model
                    // call, regardless of whether dispatch is reached.
                    let _ = deps
                        .step
                        .lock()
                        .await
                        .record_tool_result(ctx.turn_id, call_id.clone(), err.clone())
                        .await;
                    errors.push((call_id.clone(), err));
                }
            }
        }
    }

    if pending.is_empty() && errors.is_empty() {
        // Write TurnCompleted before returning the terminal state (ADR-0003).
        let _ = deps
            .step
            .lock()
            .await
            .record_turn_completed(ctx.turn_id, cogito_protocol::turn::TurnOutcome::Completed)
            .await;
        return TurnState::Completed {
            final_assistant_content: assistant_content,
        };
    }

    // Update the consecutive-error counter.
    //
    // If every tool call in this round failed H07 validation (nothing is
    // pending to dispatch), the model made no progress. Increment the
    // counter and fail-fast if the threshold is exceeded so we don't loop
    // forever when the model keeps omitting required arguments.
    let mut ctx = ctx;
    if pending.is_empty() {
        // All resolved calls were errors.
        ctx.consecutive_tool_errors += 1;
        tracing::warn!(
            consecutive = ctx.consecutive_tool_errors,
            max = MAX_CONSECUTIVE_TOOL_ERRORS,
            "all tool calls in this round failed validation"
        );
        if ctx.consecutive_tool_errors >= MAX_CONSECUTIVE_TOOL_ERRORS {
            let msg = format!(
                "aborted after {MAX_CONSECUTIVE_TOOL_ERRORS} consecutive rounds where every \
                 tool call failed argument validation — the model is not following tool schemas"
            );
            tracing::error!("{msg}");
            let reason = TurnFailureReason::ModelGatewayFailed { message: msg };
            let recorded_event_id = match deps
                .step
                .lock()
                .await
                .record_turn_failed(ctx.turn_id, reason.clone())
                .await
            {
                Ok(id) => id,
                // Recorder failed while recording the failure itself.
                Err(_) => EventId::recorder_failure_placeholder(),
            };
            return TurnState::Failed {
                reason,
                recorded_event_id,
            };
        }
    } else {
        // At least one call succeeded — reset the error streak.
        ctx.consecutive_tool_errors = 0;
    }

    // Tool calls present — dispatch loop will handle per-tool events.
    TurnState::ToolDispatching {
        ctx,
        pending,
        completed: errors,
        surface,
    }
}

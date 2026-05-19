//! `ModelCompleted → Completed | ToolDispatching` transition.
//!
//! Runs H07 Tool Call Resolver over the model output. If no tool calls
//! are present, records `TurnCompleted` and moves to `Completed`. If tool
//! calls are present, moves to `ToolDispatching`.

use std::collections::VecDeque;

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::tool::ToolDescriptor;

use crate::harness::tool_resolver::{resolve, ResolvedCall};
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `ModelCompleted` to `Completed` or `ToolDispatching`.
///
/// Writes `TurnCompleted` before entering `Completed` (ADR-0003). When
/// tool calls are pending, the dispatch loop writes per-tool events itself.
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
                ResolvedCall::Error(err) => errors.push((call_id.clone(), err)),
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

    // Tool calls present — dispatch loop will handle per-tool events.
    TurnState::ToolDispatching {
        ctx,
        pending,
        completed: errors,
        surface,
    }
}

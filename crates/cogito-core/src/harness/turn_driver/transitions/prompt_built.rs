//! `PromptBuilt → ModelCalling` transition.
//!
//! Records `ModelCallStarted` and opens the gateway stream. If the gateway
//! fails to open, transitions to `Failed`.

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::tool::ToolDescriptor;

use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `PromptBuilt` to `ModelCalling` (or `Failed`).
///
/// Writes `ModelCallStarted` before calling `ModelGateway::stream` (ADR-0003).
pub async fn transit(
    ctx: TurnCtx,
    input: ModelInput,
    surface: Vec<ToolDescriptor>,
    deps: &TurnDeps,
) -> TurnState {
    // Write event before opening the stream (ADR-0003).
    let _ = deps
        .step
        .lock()
        .await
        .record_model_call_started(ctx.turn_id, ctx.strategy.model_params.model.clone())
        .await;

    match deps.model.stream(input, ctx.exec_ctx.clone()).await {
        Ok(stream) => TurnState::ModelCalling { ctx, stream, surface },
        Err(e) => TurnState::Failed {
            reason: cogito_protocol::turn::TurnFailureReason::ModelGatewayFailed {
                message: e.to_string(),
            },
        },
    }
}

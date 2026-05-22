//! `PromptBuilt → ModelCalling` transition.
//!
//! Records `ModelCallStarted` and opens the gateway stream. If the gateway
//! fails to open, transitions to `Failed`.

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::ids::EventId;
use cogito_protocol::tool::ToolDescriptor;
use cogito_protocol::turn::TurnFailureReason;

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
        Ok(stream) => TurnState::ModelCalling {
            ctx,
            stream,
            surface,
        },
        Err(e) => {
            let reason = TurnFailureReason::ModelGatewayFailed {
                message: e.to_string(),
            };
            // Capture reason string before moving `reason` into `record_turn_failed`.
            let reason_str = format!("{reason:?}");
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
            deps.hooks.on_error(&reason_str);
            TurnState::Failed {
                reason,
                recorded_event_id,
            }
        }
    }
}

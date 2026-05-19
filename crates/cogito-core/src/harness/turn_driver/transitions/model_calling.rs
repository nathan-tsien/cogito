//! `ModelCalling → ModelCompleted` transition.
//!
//! Drives H06 stream demux to completion and transitions to `ModelCompleted`
//! (or `Failed` on a gateway error).

use cogito_protocol::gateway::{ModelError, ModelEvent};
use cogito_protocol::tool::ToolDescriptor;
use futures::stream::BoxStream;

use crate::harness::stream_demux::demux;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `ModelCalling` to `ModelCompleted` (or `Failed`).
///
/// Locks the `StepRecorder` mutex for the duration of the demux call so
/// `demux` can drive `on_text_delta` / `on_text_block_complete` /
/// `record_tool_use` synchronously without holding the lock across
/// unrelated `.await` points.
pub async fn transit(
    ctx: TurnCtx,
    stream: BoxStream<'static, Result<ModelEvent, ModelError>>,
    surface: Vec<ToolDescriptor>,
    deps: &TurnDeps,
) -> TurnState {
    let output = {
        let mut recorder = deps.step.lock().await;
        demux(stream, &mut recorder, ctx.turn_id).await
    };

    match output {
        Ok(output) => TurnState::ModelCompleted {
            ctx,
            output,
            surface,
        },
        Err(e) => TurnState::Failed {
            reason: cogito_protocol::turn::TurnFailureReason::ModelGatewayFailed {
                message: e.to_string(),
            },
        },
    }
}

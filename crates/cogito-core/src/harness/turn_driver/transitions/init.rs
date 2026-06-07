//! `Init → ContextManaged` transition.
//!
//! Records `ContextManageEntered` before advancing to `ContextManaged`.
//! Sprint 2 ignores the `ResumeDecision`; Sprint 3 will wire it.

use cogito_protocol::ids::EventId;
use cogito_protocol::turn::TurnFailureReason;

use crate::harness::resume::ResumeDecision;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `Init` to `ContextManaged`.
///
/// Writes `ContextManageEntered` first (ADR-0003 invariant), then returns
/// the next state. Store errors are silently swallowed in Sprint 2; Sprint
/// 3 will propagate them as `TurnState::Failed { StoreUnavailable }`.
///
/// Before advancing, enforces the per-turn iteration budget (ADR-0038): if
/// `model_calls` has reached `strategy.max_turns`, record `TurnFailed`
/// (`MaxTurnsExceeded`) and stop instead of starting another round. This gate
/// runs first thing each inner-loop iteration, including every resume path
/// (which loops back through `Init` before the next model call), and before
/// H11 context management so the over-budget iteration does no further work.
pub async fn transit(ctx: TurnCtx, _resume: ResumeDecision, deps: &TurnDeps) -> TurnState {
    // Enforce the iteration budget before starting another round (ADR-0038).
    if ctx.model_calls >= ctx.strategy.max_turns {
        let reason = TurnFailureReason::MaxTurnsExceeded {
            turns: ctx.model_calls,
        };
        // Capture the reason string before moving `reason` into the recorder.
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
        return TurnState::Failed {
            reason,
            recorded_event_id,
        };
    }

    // Write event before transitioning (ADR-0003).
    let _ = deps
        .step
        .lock()
        .await
        .record_context_manage_entered(ctx.turn_id)
        .await;
    TurnState::ContextManaged { ctx }
}

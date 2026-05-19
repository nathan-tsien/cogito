//! `Init → ContextManaged` transition.
//!
//! Records `ContextManageEntered` before advancing to `ContextManaged`.
//! Sprint 2 ignores the `ResumeDecision`; Sprint 3 will wire it.

use crate::harness::resume::ResumeDecision;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `Init` to `ContextManaged`.
///
/// Writes `ContextManageEntered` first (ADR-0003 invariant), then returns
/// the next state. Store errors are silently swallowed in Sprint 2; Sprint
/// 3 will propagate them as `TurnState::Failed { StoreUnavailable }`.
pub async fn transit(ctx: TurnCtx, _resume: ResumeDecision, deps: &TurnDeps) -> TurnState {
    // Write event before transitioning (ADR-0003).
    let _ = deps
        .step
        .lock()
        .await
        .record_context_manage_entered(ctx.turn_id)
        .await;
    TurnState::ContextManaged { ctx }
}

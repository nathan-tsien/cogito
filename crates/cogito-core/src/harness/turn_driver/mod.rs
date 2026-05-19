//! H01 Turn Driver — the FSM body. See
//! `docs/components/H01-turn-driver.md` for the full design.

pub mod deps;
pub mod state;
pub mod transitions;

pub use deps::TurnDeps;
pub use state::{TurnCtx, TurnState};

use cogito_protocol::turn::TurnOutcome;

use crate::harness::resume::ResumeDecision;

/// Translate a `ResumeDecision` into the starting `TurnState` and run the
/// FSM to completion.
pub async fn enter_turn(
    decision: ResumeDecision,
    ctx: TurnCtx,
    deps: TurnDeps,
) -> TurnOutcome {
    let initial = match decision {
        ResumeDecision::FreshTurn => TurnState::Init {
            ctx,
            resume: ResumeDecision::FreshTurn,
        },
        ResumeDecision::ResumeFromToolDispatching {
            pending,
            completed,
            surface_snapshot,
        } => TurnState::ToolDispatching {
            ctx,
            pending: pending.into(),
            completed,
            surface: surface_snapshot,
        },
        ResumeDecision::ResumeFromModelCompleted {
            output,
            surface_snapshot,
        } => TurnState::ModelCompleted {
            ctx,
            output,
            surface: surface_snapshot,
        },
    };
    run(initial, &deps).await
}

/// Drive the FSM loop to a terminal state and return the outcome.
pub async fn run(initial: TurnState, deps: &TurnDeps) -> TurnOutcome {
    let mut state = initial;
    loop {
        state = match state {
            TurnState::Init { ctx, resume } => {
                transitions::init::transit(ctx, resume, deps).await
            }
            TurnState::ContextManaged { ctx } => {
                transitions::context_managed::transit(ctx, deps).await
            }
            TurnState::PromptBuilt { ctx, input, surface } => {
                transitions::prompt_built::transit(ctx, input, surface, deps).await
            }
            TurnState::ModelCalling { ctx, stream, surface } => {
                transitions::model_calling::transit(ctx, stream, surface, deps).await
            }
            TurnState::ModelCompleted { ctx, output, surface } => {
                transitions::model_completed::transit(ctx, output, surface, deps).await
            }
            TurnState::ToolDispatching {
                ctx,
                pending,
                completed,
                surface,
            } => {
                transitions::tool_dispatching::transit(ctx, pending, completed, surface, deps)
                    .await
            }
            terminal @ (TurnState::Completed { .. }
            | TurnState::Paused { .. }
            | TurnState::Failed { .. }) => {
                return terminal.into_outcome();
            }
        };
    }
}

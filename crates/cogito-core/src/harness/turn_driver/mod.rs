//! H01 Turn Driver — the FSM body. See
//! `docs/components/H01-turn-driver.md` for the full design.

pub mod deps;
pub mod state;
pub mod transitions;

pub use deps::TurnDeps;
pub use state::{TurnCtx, TurnState};

use cogito_protocol::turn::TurnOutcome;

use crate::harness::resume::{ResumeDecision, ResumePoint};

/// Translate a `ResumeDecision` into the starting `TurnState` and run the
/// FSM to completion.
pub async fn enter_turn(decision: ResumeDecision, ctx: TurnCtx, deps: TurnDeps) -> TurnOutcome {
    let initial = match decision.point {
        ResumePoint::FreshTurn => TurnState::Init {
            ctx,
            resume: ResumeDecision {
                point: ResumePoint::FreshTurn,
                last_event_seq: decision.last_event_seq,
            },
        },
        // Sprint 3 P3.2 transitional: non-FreshTurn variants land in P3.4.
        // Sprint 2 callers always produce FreshTurn (replay() is still a stub),
        // so this branch is unreachable in practice. Log + fall back to fresh
        // (no panic — clippy::panic is denied).
        non_fresh => {
            tracing::error!(
                ?non_fresh,
                "non-FreshTurn ResumePoint observed in P3.2 transitional state; \
                 P3.4 will implement actual handling. Falling back to fresh turn."
            );
            TurnState::Init {
                ctx,
                resume: ResumeDecision {
                    point: ResumePoint::FreshTurn,
                    last_event_seq: decision.last_event_seq,
                },
            }
        }
    };
    run(initial, &deps).await
}

/// Drive the FSM loop to a terminal state and return the outcome.
pub async fn run(initial: TurnState, deps: &TurnDeps) -> TurnOutcome {
    let mut state = initial;
    loop {
        state = match state {
            TurnState::Init { ctx, resume } => transitions::init::transit(ctx, resume, deps).await,
            TurnState::ContextManaged { ctx } => {
                transitions::context_managed::transit(ctx, deps).await
            }
            TurnState::PromptBuilt {
                ctx,
                input,
                surface,
            } => transitions::prompt_built::transit(ctx, input, surface, deps).await,
            TurnState::ModelCalling {
                ctx,
                stream,
                surface,
            } => transitions::model_calling::transit(ctx, stream, surface, deps).await,
            TurnState::ModelCompleted {
                ctx,
                output,
                surface,
            } => transitions::model_completed::transit(ctx, output, surface, deps).await,
            TurnState::ToolDispatching {
                ctx,
                pending,
                completed,
                surface,
            } => {
                transitions::tool_dispatching::transit(ctx, pending, completed, surface, deps).await
            }
            terminal @ (TurnState::Completed { .. }
            | TurnState::Paused { .. }
            | TurnState::Failed { .. }) => {
                return terminal.into_outcome();
            }
        };
    }
}

//! H01 Turn Driver — the FSM body. See
//! `docs/components/H01-turn-driver.md` for the full design.

pub mod deps;
pub mod state;
pub mod transitions;

pub use deps::TurnDeps;
pub use state::{TurnCtx, TurnState};

use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::ids::EventId;
use cogito_protocol::tool::ToolResult;
use cogito_protocol::turn::TurnOutcome;

use crate::harness::resume::{ResumeDecision, ResumePendingCall, ResumePoint};

/// Harness-internal translation of `ResumePoint` into the FSM-level shape
/// `enter_turn` consumes. Three variants because `FreshTurn` and
/// `ResumePausedJob` are actor-level (handled before `enter_turn` is called):
/// `FreshTurn` means the actor idles until the next caller `submit`, and
/// `ResumePausedJob` puts the actor into `InFlight::PausedOnJob` directly
/// without entering the FSM.
///
/// Kept `pub` so integration tests (e.g. `turn_driver_text_only`) can call
/// `enter_turn` directly. Callers outside `cogito-core` should treat this as
/// an unstable internal detail.
#[derive(Debug)]
pub enum TurnEntry {
    /// FSM enters `Init`. Maps from `ResumePoint::FreshTurn` and
    /// `ResumePoint::RestartCurrentTurn` (the latter re-runs H04 to rebuild
    /// prompt from the event log).
    FreshLikeInit,
    /// FSM enters `ModelCompleted` with rebuilt output; fast-paths to
    /// `Completed` (no model re-call). Maps from
    /// `ResumePoint::ResumeFromModelCompleted`.
    FromModelCompleted {
        /// Reconstructed model output (content + `stop_reason` + usage).
        output: ModelOutput,
    },
    /// FSM enters `ToolDispatching` with pending/completed pre-populated.
    /// `enter_turn` re-validates `pending` via H07 (`tool_resolver::resolve`)
    /// and rebuilds the tool surface via H10+H05. Maps from
    /// `ResumePoint::ResumeFromToolDispatching` and (will eventually map from)
    /// `ResumePoint::ResumeAfterJobCompletion` once P4 wires the job
    /// outcome into the completed prefix.
    FromToolDispatching {
        /// Tool calls that need re-resolution + dispatch.
        pending: Vec<ResumePendingCall>,
        /// `(call_id, result)` pairs already completed before the crash.
        completed: Vec<(String, ToolResult)>,
    },
}

/// Translate a `TurnEntry` into the starting `TurnState` and run the FSM
/// to completion.
pub async fn enter_turn(entry: TurnEntry, ctx: TurnCtx, deps: TurnDeps) -> TurnOutcome {
    let initial = match entry {
        TurnEntry::FreshLikeInit => TurnState::Init {
            ctx,
            resume: ResumeDecision {
                point: ResumePoint::FreshTurn,
                last_event_seq: None,
            },
        },
        TurnEntry::FromModelCompleted { output } => {
            let surface = crate::harness::tool_surface::surface(&ctx.strategy, deps.tools.as_ref());
            TurnState::ModelCompleted {
                ctx,
                output,
                surface,
            }
        }
        TurnEntry::FromToolDispatching { pending, completed } => {
            let surface = crate::harness::tool_surface::surface(&ctx.strategy, deps.tools.as_ref());
            // Re-validate every pending call through H07 against the current
            // tool surface. Any failure → fail the turn cleanly (don't half-resume).
            match resolve_pending(&pending, &surface) {
                Ok(invocations) => TurnState::ToolDispatching {
                    ctx,
                    pending: invocations.into(),
                    completed,
                    surface,
                },
                Err(message) => {
                    // Record the failure event before constructing Failed state.
                    // Mirror the P2.5 pattern: capture EventId from the recorder.
                    let reason = cogito_protocol::turn::TurnFailureReason::ResumeFailed { message };
                    // Capture reason string before moving `reason` into `record_turn_failed`.
                    let reason_str = format!("{reason:?}");
                    let event_id = match deps
                        .step
                        .lock()
                        .await
                        .record_turn_failed(ctx.turn_id, reason.clone())
                        .await
                    {
                        Ok(id) => id,
                        Err(_) => EventId::recorder_failure_placeholder(),
                    };
                    deps.hooks.on_error(&reason_str);
                    TurnState::Failed {
                        reason,
                        recorded_event_id: event_id,
                    }
                }
            }
        }
    };
    run(initial, &deps).await
}

/// Re-validate persisted pending tool calls through H07. Returns an error
/// string on the first failure (tool unavailable, schema drift, etc).
fn resolve_pending(
    pending: &[ResumePendingCall],
    surface: &[cogito_protocol::tool::ToolDescriptor],
) -> Result<Vec<crate::harness::tool_resolver::ToolInvocation>, String> {
    let mut out = Vec::with_capacity(pending.len());
    for p in pending {
        match crate::harness::tool_resolver::resolve(
            &p.call_id,
            &p.tool_name,
            p.args.clone(),
            surface,
        ) {
            crate::harness::tool_resolver::ResolvedCall::Ok(inv) => out.push(inv),
            crate::harness::tool_resolver::ResolvedCall::Error(result) => {
                // Spec §4.3: persisted tool args fail current schema validation
                // → resume MUST fail (don't silently dispatch the bad call).
                return Err(format!(
                    "tool re-validation failed for call_id={call_id}: {result:?}",
                    call_id = p.call_id,
                ));
            }
        }
    }
    Ok(out)
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

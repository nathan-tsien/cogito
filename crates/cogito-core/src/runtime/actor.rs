//! `SessionActor` — the long-lived per-session tokio task.
//!
//! Implements Topology I: a `tokio::select!` loop that polls three mpsc
//! channels — the turn-result channel, the mailbox, and the job-completion
//! channel.  All three channels use `mpsc::Receiver` so we never hold a
//! mutable borrow on more than one receiver at a time, which satisfies the
//! borrow checker cleanly.
//!
//! When a `TurnDriver` task finishes, it sends its `(TurnId, TurnOutcome)`
//! through a bounded `mpsc` channel back to the actor rather than having the
//! actor join the task handle directly.  This sidesteps the well-known borrow
//! conflict that arises when trying to `select!` on both a `JoinHandle` and
//! another channel that are both behind `&mut self`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::job::{JobCompletionEvent, JobId};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{NewMessage, SessionCommand, ShutdownOutcome};
use crate::harness::hooks::HookPipeline;
use crate::harness::resume::{ResumeDecision, ResumePoint, replay};
use crate::harness::step_recorder::StepRecorder;
use crate::harness::turn_driver::{TurnCtx, TurnDeps, enter_turn};

/// In-flight state for the actor.
pub(super) enum InFlight {
    /// A `TurnDriver` task is running.
    Active {
        /// Turn identifier for event correlation.
        #[allow(dead_code)] // read via pattern matching in has_active_turn
        turn_id: TurnId,
        /// Wall-clock start time (for observability).
        #[allow(dead_code)] // reserved for metrics / tracing in later sprints
        started_at: Instant,
    },
    /// Turn paused awaiting a background job (Sprint 4).
    #[allow(dead_code)] // Sprint 4 will construct this variant
    PausedOnJob {
        /// The background job this session is waiting on.
        #[allow(dead_code)]
        job_id: JobId,
    },
}

/// All state owned by the actor task. One instance per live session.
pub(super) struct ActorState {
    /// Session identifier threaded into every recorder call.
    pub(super) session_id: SessionId,
    /// Strategy governs prompt composition and tool surface for every turn.
    pub(super) strategy: HarnessStrategy,
    /// Currently running (or paused) turn, if any.
    pub(super) in_flight: Option<InFlight>,
    /// Per-turn cancellation token; replaced on every `try_start_turn`.
    pub(super) current_cancel_token: Arc<parking_lot::Mutex<CancellationToken>>,
    /// Receives job-completion notifications from a `JobManager` (Sprint 4).
    pub(super) job_completion_rx: mpsc::Receiver<JobCompletionEvent>,
    /// Receives `(TurnId, TurnOutcome)` from the spawned `TurnDriver` task.
    pub(super) turn_result_rx: mpsc::Receiver<(TurnId, TurnOutcome)>,
    /// Sender half kept alive so the channel stays open even between turns.
    pub(super) turn_result_tx: mpsc::Sender<(TurnId, TurnOutcome)>,
    /// Broadcast channel for live `StreamEvent`s. Cloned into each `TurnDeps`.
    /// Retained here so the channel stays open while the session is alive.
    #[allow(dead_code)]
    // kept alive for channel liveness; Sprint 4 uses it for sub-actor fan-out
    pub(super) broadcast_tx: broadcast::Sender<StreamEvent>,
    /// Step recorder shared with `TurnDeps` so transitions can record events.
    pub(super) recorder: Arc<Mutex<StepRecorder>>,
    /// Store handle for replay and recorder construction.
    pub(super) store: Arc<dyn ConversationStore>,
}

/// External dependencies injected at spawn time.
pub(super) struct ActorDeps {
    /// Model gateway.
    pub model: Arc<dyn ModelGateway>,
    /// Tool provider.
    pub tools: Arc<dyn ToolProvider>,
}

impl ActorState {
    /// True iff a `TurnDriver` task is currently executing.
    pub(super) fn has_active_turn(&self) -> bool {
        matches!(self.in_flight, Some(InFlight::Active { .. }))
    }
}

/// Main actor loop. Runs until `Shutdown` is received or the mailbox closes.
///
/// Three arms in priority order (biased):
/// 1. `turn_result_rx` — receives `(TurnId, TurnOutcome)` from the spawned
///    `TurnDriver` wrapper task.  Drains first so completed turns are always
///    recorded before the next command is processed.
/// 2. `mailbox_rx` — caller commands (`Input`, `Shutdown`, etc.).
/// 3. `job_completion_rx` — async job callbacks (Sprint 4); forwarded to the
///    mailbox for FIFO ordering.
pub(super) async fn actor_main(
    mut state: ActorState,
    mut mailbox_rx: mpsc::Receiver<SessionCommand>,
    mailbox_tx: mpsc::Sender<SessionCommand>,
    deps: ActorDeps,
) {
    loop {
        tokio::select! {
            biased;

            // Arm 1: turn completion (highest priority).
            Some((turn_id, outcome)) = state.turn_result_rx.recv() => {
                on_turn_complete(&mut state, turn_id, outcome).await;
            }

            // Arm 2: caller commands.
            cmd = mailbox_rx.recv() => {
                let Some(cmd) = cmd else { break; };
                let should_break = handle_command(&mut state, cmd, &mailbox_tx, &deps).await;
                if should_break {
                    break;
                }
            }

            // Arm 3: job-completion events from JobManager (Sprint 4).
            Some(evt) = state.job_completion_rx.recv() => {
                // Translate to a mailbox command for FIFO ordering.
                let _ = mailbox_tx.send(evt.into()).await;
            }
        }
    }
}

/// Dispatch one `SessionCommand`. Returns `true` if the loop should exit.
async fn handle_command(
    state: &mut ActorState,
    cmd: SessionCommand,
    mailbox_tx: &mpsc::Sender<SessionCommand>,
    deps: &ActorDeps,
) -> bool {
    match cmd {
        SessionCommand::Input(msg) => {
            try_start_turn(state, msg, deps).await;
        }
        SessionCommand::JobCompleted { .. } => {
            // Sprint 4: resume a paused turn.
        }
        SessionCommand::InternalCancel { ack } => {
            // The cancel token is fired by the handle before sending this
            // command; just acknowledge receipt.
            let _ = ack.send(());
        }
        SessionCommand::Shutdown { deadline, ack } => {
            let outcome = drain_shutdown(state, deadline).await;
            let _ = ack.send(outcome);
            return true;
        }
    }
    let _ = mailbox_tx; // retained for future use
    false
}

/// Attempt to start a new turn. No-op if one is already in flight.
async fn try_start_turn(state: &mut ActorState, msg: NewMessage, deps: &ActorDeps) {
    if state.has_active_turn() {
        return;
    }

    let turn_id = TurnId::new();
    let new_token = CancellationToken::new();
    *state.current_cancel_token.lock() = new_token.clone();

    // Write-before-transition: record TurnStarted before spawning the task.
    {
        let mut rec = state.recorder.lock().await;
        if let Err(e) = rec
            .record_turn_started(
                turn_id,
                vec![ContentBlock::Text {
                    text: msg.text.clone(),
                }],
            )
            .await
        {
            tracing::error!(
                session_id = %state.session_id,
                turn_id = %turn_id,
                error = %e,
                "failed to record TurnStarted; aborting turn"
            );
            return;
        }
    }

    let exec_ctx = ExecCtx {
        session_id: state.session_id,
        turn_id,
        deadline: None,
        cancel: new_token,
    };
    let ctx = TurnCtx {
        session_id: state.session_id,
        turn_id,
        exec_ctx,
        strategy: state.strategy.clone(),
        consecutive_tool_errors: 0,
    };
    let turn_deps = TurnDeps {
        step: Arc::clone(&state.recorder),
        store: Arc::clone(&state.store),
        model: Arc::clone(&deps.model),
        tools: Arc::clone(&deps.tools),
        hooks: HookPipeline::new(),
    };

    // Clone the sender so the wrapper task can deliver the outcome back.
    let result_tx = state.turn_result_tx.clone();
    let decision = replay(&[]).unwrap_or(ResumeDecision {
        point: ResumePoint::FreshTurn,
        last_event_seq: None,
    });

    tokio::spawn(async move {
        let outcome = enter_turn(decision, ctx, turn_deps).await;
        // Ignore send errors — the actor may have already shut down.
        let _ = result_tx.send((turn_id, outcome)).await;
    });

    state.in_flight = Some(InFlight::Active {
        turn_id,
        started_at: Instant::now(),
    });
}

/// Record the terminal event after a turn finishes.
async fn on_turn_complete(state: &mut ActorState, turn_id: TurnId, outcome: TurnOutcome) {
    state.in_flight = None;
    let mut rec = state.recorder.lock().await;
    let result: Result<(), _> = match outcome {
        TurnOutcome::Completed => rec
            .record_turn_completed(turn_id, TurnOutcome::Completed)
            .await
            .map(|_| ()),
        TurnOutcome::Paused { job_id } => rec.record_turn_paused(turn_id, job_id).await.map(|_| ()),
        TurnOutcome::Cancelled => rec
            .record_turn_failed(turn_id, TurnFailureReason::TurnTimedOut)
            .await
            .map(|_| ()),
        // FSM transition already recorded the TurnFailed event.
        TurnOutcome::Failed { .. } => Ok(()),
        // Non-exhaustive guard for future variants added in later sprints.
        _ => rec
            .record_turn_failed(
                turn_id,
                TurnFailureReason::TurnPanicked {
                    location: "unhandled TurnOutcome variant".into(),
                },
            )
            .await
            .map(|_| ()),
    };
    if let Err(e) = result {
        tracing::error!(
            session_id = %state.session_id,
            turn_id = %turn_id,
            error = %e,
            "failed to record terminal turn event"
        );
    }
}

/// Cancel the running turn and wait (up to `deadline`) for it to finish.
async fn drain_shutdown(state: &mut ActorState, deadline: Duration) -> ShutdownOutcome {
    let started = Instant::now();
    // Signal the TurnDriver to stop cooperatively.
    state.current_cancel_token.lock().cancel();

    // Poll the turn-result channel until either the turn drains or the
    // deadline expires.
    while state.has_active_turn() && started.elapsed() < deadline {
        let remaining = deadline.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, state.turn_result_rx.recv()).await {
            Ok(Some((turn_id, outcome))) => {
                on_turn_complete(state, turn_id, outcome).await;
            }
            Ok(None) | Err(_) => {
                // Channel closed or timeout — stop waiting.
                break;
            }
        }
    }

    let clean = !state.has_active_turn();
    ShutdownOutcome {
        clean,
        in_flight_cancelled: if clean {
            None
        } else {
            Some("turn still running at shutdown deadline".into())
        },
    }
}

/// Record the `SessionStarted` event once at session open.
pub(super) async fn record_session_started(
    recorder: &Arc<Mutex<StepRecorder>>,
    session_id: SessionId,
    strategy: &HarnessStrategy,
) {
    let meta = SessionMeta {
        cogito_version: env!("CARGO_PKG_VERSION").into(),
        strategy: Some(strategy.name.clone()),
        model: Some(strategy.model_params.model.clone()),
        ..Default::default()
    };
    let mut rec = recorder.lock().await;
    if let Err(e) = rec.record_session_started(meta).await {
        tracing::error!(
            session_id = %session_id,
            error = %e,
            "failed to record SessionStarted event"
        );
    }
}

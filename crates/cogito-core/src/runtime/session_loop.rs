//! Per-session loop — the long-lived tokio task that drives one session.
//!
//! Conceptually this is the "session actor": it owns the session's private
//! state, accepts commands through a mailbox, and is the single mutator of
//! its own state (see ADR-0006 §"Actor model — why and how"). The public
//! entry point is the free function [`run_session`], which keeps the API
//! surface free of the "Actor" terminology while preserving the underlying
//! invariants. Internal docs still refer to the "actor" / "actor task"
//! where that vocabulary clarifies the design (private state, message-
//! driven interaction, single mutable owner, cooperative termination).
//!
//! Implements Topology I: a `tokio::select!` loop that polls three mpsc
//! channels — the turn-result channel, the mailbox, and the job-completion
//! channel.  All three channels use `mpsc::Receiver` so we never hold a
//! mutable borrow on more than one receiver at a time, which satisfies the
//! borrow checker cleanly.
//!
//! When a `TurnDriver` task finishes, it sends its `(TurnId, TurnOutcome)`
//! through a bounded `mpsc` channel back to the loop rather than having the
//! loop join the task handle directly.  This sidesteps the well-known borrow
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

use super::types::{SessionCommand, ShutdownOutcome, TurnTrigger};
use crate::harness::hooks::HookPipeline;
use crate::harness::resume::{ResumePoint, replay};
use crate::harness::step_recorder::StepRecorder;
use crate::harness::turn_driver::{TurnCtx, TurnDeps, TurnEntry, enter_turn};

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
        /// The turn that was paused.
        #[allow(dead_code)]
        turn_id: TurnId,
        /// The background job this session is waiting on.
        #[allow(dead_code)]
        job_id: JobId,
    },
}

/// All state owned by the actor task. One instance per live session.
pub(super) struct SessionState {
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
pub(super) struct SessionDeps {
    /// Model gateway.
    pub model: Arc<dyn ModelGateway>,
    /// Tool provider.
    pub tools: Arc<dyn ToolProvider>,
}

impl SessionState {
    /// True iff a `TurnDriver` task is currently executing.
    pub(super) fn has_active_turn(&self) -> bool {
        matches!(self.in_flight, Some(InFlight::Active { .. }))
    }
}

/// Main session loop. Runs until `Shutdown` is received or the mailbox
/// closes. This is the "actor task" body — see the module-level docs for
/// the actor-model invariants it upholds.
///
/// # Startup sequence (Sprint 3 P4.4, per spec §5.2)
///
/// 1. Schema check: any event with `schema_version > SCHEMA_VERSION` aborts
///    startup with `ShutdownOutcome::ResumeFailed`.
/// 2. Call H03 `replay()` over the persisted log to compute a `ResumePoint`.
/// 3. Dispatch via `apply_resume_point` — for `FreshTurn` this is a no-op
///    and the actor enters the mailbox loop idle; for resume variants it
///    spawns a `TurnDriver` into the correct FSM state.
/// 4. Enter the mailbox loop (below).
///
/// Steps that the plan calls out but are already handled elsewhere:
/// - Seq init lives in `RuntimeBuilder::open_session` at the `StepRecorder`
///   construction site.
/// - `SessionStarted` recording is gated in `open_session` (only on a
///   fresh-store session).
///
/// # Mailbox loop
///
/// Three arms in priority order (biased):
/// 1. `turn_result_rx` — receives `(TurnId, TurnOutcome)` from the spawned
///    `TurnDriver` wrapper task.  Drains first so completed turns are always
///    recorded before the next command is processed.
/// 2. `mailbox_rx` — caller commands (`Trigger`, `Shutdown`, etc.).
/// 3. `job_completion_rx` — async job callbacks (Sprint 4); forwarded to the
///    mailbox for FIFO ordering.
///
/// # Return value
///
/// `ShutdownOutcome::Clean` on a normal exit (mailbox closed or `Shutdown`
/// received); a non-Clean variant if the startup sequence failed before the
/// loop began. The spawn site in `builder.rs` logs the outcome.
pub(super) async fn run_session(
    mut state: SessionState,
    mut mailbox_rx: mpsc::Receiver<SessionCommand>,
    mailbox_tx: mpsc::Sender<SessionCommand>,
    deps: SessionDeps,
    initial_events: Vec<cogito_protocol::ConversationEvent>,
) -> ShutdownOutcome {
    // 1. Schema check (must come before replay so we never feed a
    //    newer-schema log into the resume coordinator).
    if let Some(evt) = initial_events
        .iter()
        .find(|e| e.schema_version > cogito_protocol::SCHEMA_VERSION)
    {
        return ShutdownOutcome::ResumeFailed(format!(
            "unsupported schema_version={} (this build supports up to {})",
            evt.schema_version,
            cogito_protocol::SCHEMA_VERSION
        ));
    }

    // 2. Compute the resume decision (pure H03 projection).
    let decision = match replay(&initial_events) {
        Ok(d) => d,
        Err(e) => return ShutdownOutcome::ResumeFailed(e.to_string()),
    };

    // 3. Seq init: already handled in builder.rs at StepRecorder construction.
    // 4. SessionStarted: already gated in builder.rs::open_session.

    // 5. Dispatch the resume point. Errors here are startup-fatal.
    if let Err(outcome) = apply_resume_point(&mut state, decision.point, &deps) {
        return outcome;
    }

    // 6. Mailbox loop.
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
                let outcome_opt = handle_command(&mut state, cmd, &mailbox_tx, &deps).await;
                if let Some(outcome) = outcome_opt {
                    return outcome;
                }
            }

            // Arm 3: job-completion events from JobManager (Sprint 4).
            Some(evt) = state.job_completion_rx.recv() => {
                // Translate to a mailbox command for FIFO ordering.
                let _ = mailbox_tx.send(evt.into()).await;
            }
        }
    }
    // Mailbox channel closed without a Shutdown command — treat as Clean.
    ShutdownOutcome::Clean {
        in_flight_cancelled: None,
    }
}

/// Translate a `ResumePoint` into actor-level startup actions. See the
/// per-variant matrix in the Sprint 3 P4.4 plan:
///
/// - `FreshTurn` — no-op (actor will idle until next `submit` / `Trigger`).
/// - `RestartCurrentTurn` — v0.1 downgrade to `FreshTurn` with a `tracing::warn`
///   (full implementation requires recovering `user_input` from
///   `initial_events` and is post-Sprint-3 work).
/// - `ResumeFromModelCompleted` — spawn `TurnDriver` with
///   `TurnEntry::FromModelCompleted`.
/// - `ResumeFromToolDispatching` — spawn `TurnDriver` with
///   `TurnEntry::FromToolDispatching`.
/// - `ResumePausedJob` / `ResumeAfterJobCompletion` — v0.1 returns
///   `ShutdownOutcome::JobManagerUnavailable`; `JobManager` injection is a
///   Sprint 4 deliverable.
fn apply_resume_point(
    state: &mut SessionState,
    point: ResumePoint,
    deps: &SessionDeps,
) -> Result<(), ShutdownOutcome> {
    match point {
        ResumePoint::FreshTurn => Ok(()),

        ResumePoint::RestartCurrentTurn { turn_id } => {
            // TODO(post-Sprint-3): recover user_input from initial_events
            // (EventPayload::TurnStarted { user_input } at the latest
            // TurnStarted boundary) and call
            // spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit, deps).
            // For v0.1, downgrade to fresh-idle: warn loudly so operators see
            // the recovery gap, then let the actor wait for a new Input.
            tracing::warn!(
                session_id = %state.session_id,
                %turn_id,
                "RestartCurrentTurn requested but user_input recovery is not yet wired \
                 (post-Sprint-3 work); downgrading to FreshTurn"
            );
            Ok(())
        }

        ResumePoint::ResumeFromModelCompleted {
            turn_id,
            rebuilt_output,
        } => {
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromModelCompleted {
                    output: rebuilt_output,
                },
                deps,
            );
            Ok(())
        }

        ResumePoint::ResumeFromToolDispatching {
            turn_id,
            pending,
            completed,
        } => {
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromToolDispatching { pending, completed },
                deps,
            );
            Ok(())
        }

        ResumePoint::ResumePausedJob { .. } | ResumePoint::ResumeAfterJobCompletion { .. } => {
            Err(ShutdownOutcome::JobManagerUnavailable(
                "Sprint 4 deliverable - v0.1 has no JobManager injection".into(),
            ))
        }
    }
}

/// Spawn the `TurnDriver` task for an existing `turn_id` and entry shape, set
/// `state.in_flight = Active`, and reset the per-turn cancel token.
///
/// **Does not** record a `TurnStarted` event — used by both fresh-start
/// (whose caller records `TurnStarted` itself) and resume (whose
/// `TurnStarted` is already in the persisted log). Callers that need to
/// record `TurnStarted` must do so before invoking this helper.
fn spawn_turn_driver(
    state: &mut SessionState,
    turn_id: TurnId,
    entry: TurnEntry,
    deps: &SessionDeps,
) {
    // TODO(cancel-token-disconnect): SessionShared.current_cancel_token holds
    // a sibling token cloned from the *initial* token at session-open time,
    // not a shared Arc<Mutex<...>> with SessionState. Replacing the inner token
    // here means SessionHandle::cancel_turn() fires the original sibling and
    // does not reach this newly minted token. Fix by sharing one
    // Arc<Mutex<CancellationToken>> across SessionState and SessionShared so
    // mutations are visible to both. Tracked separately; current chaos tests
    // do not exercise mid-turn cancellation past the first turn.
    let new_token = CancellationToken::new();
    *state.current_cancel_token.lock() = new_token.clone();

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
    let result_tx = state.turn_result_tx.clone();
    tokio::spawn(async move {
        let outcome = enter_turn(entry, ctx, turn_deps).await;
        // Ignore send errors — the actor may have already shut down.
        let _ = result_tx.send((turn_id, outcome)).await;
    });
    // NOTE: for resumed turns this records the resume wall time, not the
    // original TurnStarted timestamp from the persisted log. The field is
    // currently used only for observability (and is `#[allow(dead_code)]`),
    // but consumers that surface turn durations across resumes should pull
    // the canonical start time from the event log, not from this field.
    state.in_flight = Some(InFlight::Active {
        turn_id,
        started_at: Instant::now(),
    });
}

/// Dispatch one `SessionCommand`. Returns `Some(outcome)` if the loop should
/// exit with that `ShutdownOutcome`, or `None` to keep looping.
///
/// `mailbox_tx` is retained for `JobCompleted` re-injection on Arm 3 of the
/// mailbox loop (which still owns the sender directly) and for future
/// commands that need to enqueue follow-ups; it is not used here today.
async fn handle_command(
    state: &mut SessionState,
    cmd: SessionCommand,
    mailbox_tx: &mpsc::Sender<SessionCommand>,
    deps: &SessionDeps,
) -> Option<ShutdownOutcome> {
    let _ = mailbox_tx; // reserved for future enqueue-follow-up commands
    match cmd {
        SessionCommand::Trigger(trigger) => {
            try_start_turn(state, trigger, deps).await;
            None
        }
        SessionCommand::JobCompleted { .. } => {
            // Sprint 4: resume a paused turn.
            None
        }
        SessionCommand::InternalCancel { ack } => {
            // The cancel token is fired by the handle before sending this
            // command; just acknowledge receipt.
            let _ = ack.send(());
            None
        }
        SessionCommand::Shutdown { deadline, ack } => {
            let outcome = drain_shutdown(state, deadline).await;
            let _ = ack.send(outcome);
            // Caller-requested shutdown is always a "clean" actor exit from
            // the spawn site's perspective; the detailed `ShutdownOutcome`
            // was already delivered to the caller via the oneshot ack above.
            Some(ShutdownOutcome::Clean {
                in_flight_cancelled: None,
            })
        }
    }
}

/// Attempt to start a fresh turn from a caller-submitted `TurnTrigger`.
/// No-op if a turn is already in flight. Always uses
/// `TurnEntry::FreshLikeInit` — resume dispatch happens once at actor
/// startup via `apply_resume_point`, not here.
///
/// `TurnTrigger` projection (v0.1 single-variant; ADR-0016):
/// - `UserText(text)` -> `user_input = vec![ContentBlock::Text { text }]`
///
/// Future variants (`UserContent` / `SkillInvocation` / `HookFired`) extend
/// this match. `#[non_exhaustive]` forces the `_ =>` arm; we log loudly
/// and drop the trigger rather than panic — a missed variant is a
/// runtime bug, not a turn failure.
async fn try_start_turn(state: &mut SessionState, trigger: TurnTrigger, deps: &SessionDeps) {
    if state.has_active_turn() {
        return;
    }

    // match_wildcard_for_single_variants: required — TurnTrigger is
    //   #[non_exhaustive], so omitting `_` would be a compile error.
    // single_match_else: optional — let-else would silence the lint, but
    //   `match` is preferred because future ADR-0016 §6 variants extend
    //   this arm list and the match-shape signals "list will grow".
    #[allow(clippy::match_wildcard_for_single_variants)]
    #[allow(clippy::single_match_else)]
    let user_input: Vec<ContentBlock> = match trigger {
        TurnTrigger::UserText(text) => vec![ContentBlock::Text { text }],
        // `#[non_exhaustive]` guard: when a future TurnTrigger variant
        // lands (ADR-0016 §6 migration table) the consumer crate that
        // adds the variant must also extend this match. Until then,
        // log + drop is correct: no event is written, no turn spawned.
        _ => {
            tracing::error!(
                session_id = %state.session_id,
                "unhandled TurnTrigger variant; dropping turn (this is a build wiring bug)"
            );
            return;
        }
    };

    let turn_id = TurnId::new();

    // Write-before-transition: record TurnStarted before spawning the task.
    {
        let mut rec = state.recorder.lock().await;
        if let Err(e) = rec.record_turn_started(turn_id, user_input).await {
            tracing::error!(
                session_id = %state.session_id,
                turn_id = %turn_id,
                error = %e,
                "failed to record TurnStarted; aborting turn"
            );
            return;
        }
    }

    spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit, deps);
}

/// Record the terminal event after a turn finishes.
async fn on_turn_complete(state: &mut SessionState, turn_id: TurnId, outcome: TurnOutcome) {
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
async fn drain_shutdown(state: &mut SessionState, deadline: Duration) -> ShutdownOutcome {
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

    let in_flight_cancelled = if state.has_active_turn() {
        Some("turn still running at shutdown deadline".into())
    } else {
        None
    };
    ShutdownOutcome::Clean {
        in_flight_cancelled,
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

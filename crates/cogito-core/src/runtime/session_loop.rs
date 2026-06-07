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

use futures::StreamExt as _;

use cogito_protocol::ExecCtx;
use cogito_protocol::MetricsRecorder;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::ContextPipeline;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::job::{JobCompletionEvent, JobError, JobId, JobManager, JobOutcome};
use cogito_protocol::session::SessionMeta;
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolErrorKind, ToolProvider, ToolResult};
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{SessionCommand, ShutdownOutcome, TurnTrigger};
use crate::harness::hooks::CompositeHookPipeline;
use crate::runtime::SessionSpec;
// cogito-context is wired by the Runtime layer (not the Brain/harness layer),
// consistent with ADR-0004: the runtime composes the pipeline from config and
// injects it as a protocol trait object into TurnDeps.
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
    /// Turn paused awaiting a background job. Constructed by
    /// `on_turn_complete` when a turn returns `TurnOutcome::Paused` and
    /// consumed by `handle_command` on the subsequent `JobCompleted`
    /// mailbox command to rebuild a `TurnEntry::FromToolDispatching`.
    PausedOnJob {
        /// The turn that was paused.
        turn_id: TurnId,
        /// The background job this session is waiting on.
        job_id: JobId,
        /// `call_id` of the originating `ToolUseRecorded` — required to
        /// stitch the completed job back into the resumed turn's
        /// `completed_before_pause` tail. Derived from the in-memory
        /// recorder cache at pause time via `lookup_call_id_in_recorder`,
        /// so the live path matches the resume-from-log path
        /// (`harness::resume::lookup_call_id_in_events`).
        call_id: String,
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
    /// Sender half of the job-completion channel, cloned into every
    /// per-turn `TurnDeps` so H08's async dispatcher (Task 12) can register
    /// it as the `on_complete` sink against `JobManager`. Retained on the
    /// session so the channel stays open across turns even when no
    /// `TurnDeps` is alive.
    pub(super) job_completion_tx: mpsc::Sender<JobCompletionEvent>,
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
    /// Hook pipeline shared across all turns in this session.
    pub(super) hooks: Arc<CompositeHookPipeline>,
    /// Metrics sink for this session (defaults to `NoOpMetricsRecorder` until v0.4 wires a real adapter).
    pub(super) metrics: Arc<dyn MetricsRecorder>,
    /// Context pipeline built once at session open from `strategy.context`.
    /// All turns in this session share the same pipeline via `Arc::clone`.
    pub(super) context_pipeline: Arc<ContextPipeline>,
    /// Optional Skill loader provider — injected at Runtime build time and
    /// cloned into every turn's `TurnDeps`. `None` for sessions that do not
    /// use the Skill injector.
    pub(super) skills: Option<Arc<dyn SkillProvider>>,
    /// Single-slot queue for user input received while a turn is in
    /// flight or paused. Latest-wins: a second arrival overwrites the
    /// first and logs a `tracing::warn!`. Drained in `on_turn_complete`
    /// once the turn is fully terminal (not on `Paused`, which leaves
    /// `in_flight = Some(PausedOnJob)`).
    pub(super) pending_user_input: Option<TurnTrigger>,
    /// Subagent nesting depth of this session (0 = top-level). Read from
    /// the seq=0 `SessionMeta` at open; flowed into each turn's `ExecCtx`.
    pub(super) subagent_depth: u32,
}

/// External dependencies injected at spawn time.
pub(super) struct SessionDeps {
    /// Model gateway.
    pub model: Arc<dyn ModelGateway>,
    /// Tool provider.
    pub tools: Arc<dyn ToolProvider>,
    /// Job manager. Used by `SessionCommand::CancelJob` to abort a
    /// background job when the caller cancels a turn that is paused on
    /// one. Task 11 will additionally plumb this into `TurnDeps` so the
    /// async dispatcher path (Task 12) can submit and register jobs.
    pub job_mgr: Arc<dyn JobManager>,
    /// Recursive Brain spawner injected per-turn into `ExecCtx`. `None`
    /// when the Runtime had no spawner (subagent disabled).
    pub brain_spawner: Option<std::sync::Arc<dyn cogito_protocol::subagent::BrainSpawner>>,
    /// Session working tree injected per-turn into `ExecCtx` (ADR-0030 /
    /// ADR-0031). `None` when no workspace was wired; per-session and
    /// swappable mid-session via `SessionSpec`.
    pub workspace: Option<std::sync::Arc<dyn cogito_protocol::workspace::Workspace>>,
}

impl SessionState {
    /// True iff a `TurnDriver` task is currently executing.
    pub(super) fn has_active_turn(&self) -> bool {
        matches!(self.in_flight, Some(InFlight::Active { .. }))
    }

    /// True iff the session is parked waiting on a background job — i.e. a
    /// turn returned `TurnOutcome::Paused` and `on_turn_complete` left
    /// `in_flight = Some(InFlight::PausedOnJob { .. })`. The actor cannot
    /// start a new turn in this state because the paused turn must drain
    /// first via `JobCompleted`.
    pub(super) fn is_paused(&self) -> bool {
        matches!(self.in_flight, Some(InFlight::PausedOnJob { .. }))
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
    mut deps: SessionDeps,
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
    if let Err(outcome) =
        apply_resume_point(&mut state, &initial_events, decision.point, &deps).await
    {
        return outcome;
    }

    // 6. Mailbox loop.
    loop {
        tokio::select! {
            biased;

            // Arm 1: turn completion (highest priority).
            Some((turn_id, outcome)) = state.turn_result_rx.recv() => {
                on_turn_complete(&mut state, turn_id, outcome, &deps).await;
            }

            // Arm 2: caller commands.
            cmd = mailbox_rx.recv() => {
                let Some(cmd) = cmd else { break; };
                if let SessionCommand::UpdateSession(spec) = cmd {
                    apply_session_update(&mut state, &mut deps, *spec);
                } else {
                    let outcome_opt = handle_command(&mut state, cmd, &mailbox_tx, &deps).await;
                    if let Some(outcome) = outcome_opt {
                        return outcome;
                    }
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
    // Release store resources on this exit path too (last handle dropped
    // without an explicit shutdown).
    release_store_resources(&state.store, state.session_id).await;
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
/// - `ResumePausedJob` — restore `InFlight::PausedOnJob` and register
///   `on_complete` with the current `JobManager`. If the manager returns
///   `UnknownJob` (the in-memory map was lost across the restart),
///   synthesize a `Failed { message: "lost across process restart" }`
///   completion on the session's own Arm 3 channel so the same FIFO path
///   carries it as if a live job had just failed.
/// - `ResumeAfterJobCompletion` — translate the already-persisted outcome
///   to a `ToolResult` and spawn `TurnDriver` with
///   `TurnEntry::FromToolDispatching` carrying the resolved completion.
///
/// The line count is structural (one arm per `ResumePoint` variant); each
/// per-arm body is small and splitting the dispatch would only force the
/// reader to chase helpers for what is already a flat match.
#[allow(clippy::too_many_lines)]
async fn apply_resume_point(
    state: &mut SessionState,
    initial_events: &[cogito_protocol::ConversationEvent],
    point: ResumePoint,
    deps: &SessionDeps,
) -> Result<(), ShutdownOutcome> {
    match point {
        ResumePoint::FreshTurn => Ok(()),

        ResumePoint::RestartCurrentTurn { turn_id } => {
            // TODO(ADR-0038): when this is wired to actually re-spawn the turn,
            // seed `model_calls` carefully. The interrupted call's
            // `ModelCallStarted` is already in the log, so
            // `count_model_calls_in_events` counts it; re-issuing that same call
            // increments again in `prompt_built` → an off-by-one over-count.
            // Seed from the log MINUS the restarted (uncompleted) call.
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
                count_model_calls_in_events(initial_events, turn_id),
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
                count_model_calls_in_events(initial_events, turn_id),
                deps,
            );
            Ok(())
        }

        ResumePoint::ResumePausedJob { turn_id, job_id } => {
            // Resolve the originating `call_id` from the persisted log so
            // the eventual `JobCompleted` mailbox command (live or
            // synthetic) can stitch the result back into the resumed
            // turn's `completed_before_pause` via
            // `TurnEntry::FromToolDispatching`.
            let call_id = crate::harness::resume::lookup_call_id_in_events(initial_events, job_id)
                .ok_or_else(|| {
                    ShutdownOutcome::ResumeFailed(format!(
                        "ResumePausedJob: no JobSubmitted for job {job_id}"
                    ))
                })?;
            state.in_flight = Some(InFlight::PausedOnJob {
                turn_id,
                job_id,
                call_id,
            });

            match deps
                .job_mgr
                .on_complete(job_id, state.job_completion_tx.clone())
                .await
            {
                Ok(()) => {
                    // Sink registered; will fire when the job terminates.
                    Ok(())
                }
                Err(JobError::UnknownJob(_)) => {
                    // Lost across process restart. Synthesize a Failed
                    // completion by posting on the session's own Arm 3
                    // channel so it flows through the same FIFO path as a
                    // live completion (`handle_command` consumes the
                    // `JobCompleted` mailbox command, records
                    // `JobCompletedRecorded`, then re-spawns the
                    // TurnDriver with the AsyncFailed `ToolResult`).
                    let synth = JobCompletionEvent {
                        job_id,
                        outcome: JobOutcome::Failed {
                            message: "lost across process restart".into(),
                        },
                    };
                    if let Err(e) = state.job_completion_tx.send(synth).await {
                        return Err(ShutdownOutcome::ResumeFailed(format!(
                            "could not post synthetic JobCompletion: {e}"
                        )));
                    }
                    Ok(())
                }
                Err(e) => Err(ShutdownOutcome::ResumeFailed(e.to_string())),
            }
        }

        ResumePoint::ResumeAfterJobCompletion {
            turn_id,
            call_id,
            outcome,
            ..
        } => {
            // The `JobCompletedRecorded` event is already persisted; no
            // need to re-record it. Per spec §5.1, the canonical ordering
            // is `JobCompletedRecorded < ToolResultRecorded`; the live
            // path writes both under the same recorder lock, but if the
            // actor crashed BETWEEN those two appends, only the former
            // is in the log and we must synthesize the latter here so
            // every `ToolUseRecorded` keeps its matching
            // `ToolResultRecorded`. Skip if the log already has it
            // (crash happened AFTER both appends) to avoid a duplicate.
            let tool_result = outcome_to_tool_result(outcome);
            let already_recorded = initial_events.iter().any(|e| {
                matches!(
                    &e.payload,
                    cogito_protocol::event::EventPayload::ToolResultRecorded { call_id: c, .. }
                        if *c == call_id
                )
            });
            if !already_recorded {
                let mut rec = state.recorder.lock().await;
                if let Err(e) = rec
                    .record_tool_result(turn_id, call_id.clone(), tool_result.clone())
                    .await
                {
                    return Err(ShutdownOutcome::ResumeFailed(e.to_string()));
                }
            }
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromToolDispatching {
                    pending: vec![],
                    completed: vec![(call_id, tool_result)],
                },
                count_model_calls_in_events(initial_events, turn_id),
                deps,
            );
            Ok(())
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
    model_calls: u32,
    deps: &SessionDeps,
) {
    // Swap a fresh CancellationToken into the per-session slot. The slot is
    // an Arc<Mutex<...>> shared with every SessionHandle (built in
    // runtime::builder::open_session), so this swap is immediately visible
    // to `SessionHandle::cancel_turn`. See the `cancel_after_first_turn`
    // regression test for why the Arc must be shared (not sibling-cloned).
    let new_token = CancellationToken::new();
    *state.current_cancel_token.lock() = new_token.clone();

    let exec_ctx = ExecCtx {
        session_id: state.session_id,
        turn_id,
        call_id: None,
        deadline: None,
        cancel: new_token,
        subagent_depth: state.subagent_depth,
        brain_spawner: deps.brain_spawner.clone(),
        workspace: deps.workspace.clone(),
        // Read-only skill bundle roots (ADR-0032), derived from the session's
        // current skill provider (override-aware via `apply_session_update`).
        // Empty when no skills are wired or the provider exposes none.
        skill_roots: state.skills.as_ref().map_or_else(
            || std::sync::Arc::from([]),
            |s| std::sync::Arc::from(s.skill_roots()),
        ),
    };
    let ctx = TurnCtx {
        session_id: state.session_id,
        turn_id,
        exec_ctx,
        strategy: state.strategy.clone(),
        consecutive_tool_errors: 0,
        // Seeded by the caller (ADR-0038): 0 for a fresh turn, or the model
        // calls already made (re-derived from the event log) for a resumed
        // turn, so the iteration budget is honored across pause/resume.
        model_calls,
    };
    // Pipeline was built once at session open in `Runtime::open_session` and
    // stored in `SessionState`. Clone the Arc here so each turn shares the
    // same pipeline without rebuilding it.
    let context_pipeline = Arc::clone(&state.context_pipeline);
    let turn_deps = TurnDeps {
        step: Arc::clone(&state.recorder),
        store: Arc::clone(&state.store),
        model: Arc::clone(&deps.model),
        tools: Arc::clone(&deps.tools),
        hooks: Arc::clone(&state.hooks),
        metrics: Arc::clone(&state.metrics),
        context_pipeline,
        skills: state.skills.clone(),
        job_mgr: Arc::clone(&deps.job_mgr),
        job_completion_tx: state.job_completion_tx.clone(),
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

/// Re-derive the per-turn model-call count from already-loaded events (ADR-0038).
///
/// Counts `ModelCallStarted` events belonging to `turn_id`. A fresh turn has
/// none yet (returns 0); a resumed turn returns the calls made before the pause
/// or crash, so the in-memory iteration counter on `TurnCtx` is reconstructed
/// and the `max_turns` budget is honored across the resume boundary rather than
/// reset. The counter carries the count within a `TurnDriver` task; this runs
/// once per spawn, not per inner-loop iteration.
///
/// Synchronous on purpose: the resume coordinator already loaded the turn's
/// events (`initial_events`), so seeding from them adds no store read and — on
/// the crash-resume path — no extra `await` before the turn driver spawns.
fn count_model_calls_in_events(
    events: &[cogito_protocol::ConversationEvent],
    turn_id: TurnId,
) -> u32 {
    u32::try_from(
        events
            .iter()
            .filter(|e| {
                e.turn_id == Some(turn_id)
                    && matches!(
                        e.payload,
                        cogito_protocol::event::EventPayload::ModelCallStarted { .. }
                    )
            })
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Re-derive the model-call count by replaying the store (ADR-0038). Used by the
/// live job-completion resume path, which does not hold the turn's events in
/// memory. A read error stops the scan early (mirrors H11's history load): a
/// truncated count can only under-count, never over-count.
async fn count_model_calls_from_store(
    store: &dyn ConversationStore,
    session_id: SessionId,
    turn_id: TurnId,
) -> u32 {
    let mut stream = store.replay(session_id, 0);
    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(ev) => events.push(ev),
            Err(_) => break,
        }
    }
    count_model_calls_in_events(&events, turn_id)
}

/// Apply a mid-session provider swap (ADR-0028). Replaces only the
/// provided Arcs; `tenant_id` / `user_id` are intentionally not changed
/// (session identity is fixed at open). Effective at the next turn
/// boundary because `spawn_turn_driver` rebuilds `TurnDeps` from these.
///
/// A skills or strategy change additionally rebuilds `state.context_pipeline`:
/// the `SkillInjector` embedded in the pipeline captures its `SkillProvider`
/// at build time, and the pipeline is derived from `strategy.context`, so a
/// swap that touched either would otherwise leave H11 system-prompt injection
/// using the open-time provider/config. If the rebuild fails (e.g. the new
/// strategy selects `SystemPromptInjectorConfig::Skill` but the swap cleared
/// the skills provider), the previous pipeline is kept and a warning is logged
/// rather than tearing down the session.
fn apply_session_update(state: &mut SessionState, deps: &mut SessionDeps, spec: SessionSpec) {
    if let Some(tools) = spec.tools {
        deps.tools = tools;
    }
    if let Some(workspace) = spec.workspace {
        deps.workspace = Some(workspace);
    }
    let skills_changed = spec.skills.is_some();
    if let Some(skills) = spec.skills {
        state.skills = Some(skills);
    }
    let strategy_changed = spec.strategy.is_some();
    if let Some(strategy) = spec.strategy {
        state.strategy = strategy;
    }
    if skills_changed || strategy_changed {
        match cogito_context::build_pipeline_v2(&state.strategy.context, state.skills.clone()) {
            Ok(pipeline) => state.context_pipeline = Arc::new(pipeline),
            Err(e) => tracing::warn!(
                session_id = %state.session_id,
                error = %e,
                "apply_session_update: context pipeline rebuild failed; keeping previous pipeline",
            ),
        }
    }
}

/// Dispatch one `SessionCommand`. Returns `Some(outcome)` if the loop should
/// exit with that `ShutdownOutcome`, or `None` to keep looping.
///
/// `mailbox_tx` is retained for `JobCompleted` re-injection on Arm 3 of the
/// mailbox loop (which still owns the sender directly) and for future
/// commands that need to enqueue follow-ups; it is not used here today.
///
/// The line count is structural (one arm per `SessionCommand` variant) and
/// breaking it up would only force the reader to hop between helpers; the
/// per-arm bodies are independently small.
#[allow(clippy::too_many_lines)]
async fn handle_command(
    state: &mut SessionState,
    cmd: SessionCommand,
    mailbox_tx: &mpsc::Sender<SessionCommand>,
    deps: &SessionDeps,
) -> Option<ShutdownOutcome> {
    let _ = mailbox_tx; // reserved for future enqueue-follow-up commands
    match cmd {
        SessionCommand::Trigger(trigger) => {
            // Single-slot mid-pause queue (spec §8.4): if a turn is either
            // running or paused on a job, hold the new trigger in the slot
            // rather than starting a turn. A second arrival overwrites the
            // first (latest-wins) and logs a warn so operators can detect
            // dropped input. Drained in `on_turn_complete` when the turn
            // outcome is terminal.
            if state.has_active_turn() || state.is_paused() {
                if state.pending_user_input.is_some() {
                    tracing::warn!(
                        session_id = %state.session_id,
                        "overwriting queued user input (single-slot semantics)"
                    );
                }
                state.pending_user_input = Some(trigger);
            } else {
                try_start_turn(state, trigger, deps).await;
            }
            None
        }
        SessionCommand::JobCompleted { event } => {
            let JobCompletionEvent { job_id, outcome } = event;

            // Verify the session is paused on this exact job. Mismatch /
            // wrong state is logged and dropped — the only safe action when
            // an unexpected completion arrives is to refuse to resume.
            let Some(InFlight::PausedOnJob {
                turn_id,
                job_id: expected,
                call_id,
            }) = std::mem::take(&mut state.in_flight)
            else {
                tracing::error!(
                    session_id = %state.session_id,
                    %job_id,
                    "JobCompleted received but session is not PausedOnJob; dropping"
                );
                return None;
            };

            if expected != job_id {
                tracing::error!(
                    session_id = %state.session_id,
                    expected = %expected,
                    received = %job_id,
                    "JobCompleted job_id mismatch; restoring in_flight and dropping event"
                );
                // Restore the paused state so the legitimate completion
                // can still resume the turn when it eventually arrives.
                state.in_flight = Some(InFlight::PausedOnJob {
                    turn_id,
                    job_id: expected,
                    call_id,
                });
                return None;
            }

            // Write-before-transition: record JobCompleted before re-spawning
            // the TurnDriver so the persisted log reflects the completion
            // even if the actor dies between the record and the spawn.
            let tool_result = outcome_to_tool_result(outcome.clone());
            {
                let mut rec = state.recorder.lock().await;
                if let Err(e) = rec.record_job_completed(turn_id, job_id, outcome).await {
                    tracing::error!(
                        session_id = %state.session_id,
                        %turn_id,
                        error = %e,
                        "failed to record JobCompleted; aborting session"
                    );
                    return Some(ShutdownOutcome::ResumeFailed(e.to_string()));
                }
                // Spec §5.1 ordering invariant:
                // JobCompletedRecorded < ToolResultRecorded. The resumed
                // TurnDriver enters at `ToolDispatching` with an empty
                // `pending` and the already-resolved completion in
                // `completed`, so `tool_dispatching::transit`'s
                // pending-pop loop never records the result. We must do
                // it here so every `ToolUseRecorded` keeps its matching
                // `ToolResultRecorded` in the persisted log.
                if let Err(e) = rec
                    .record_tool_result(turn_id, call_id.clone(), tool_result.clone())
                    .await
                {
                    tracing::error!(
                        session_id = %state.session_id,
                        %turn_id,
                        error = %e,
                        "failed to record ToolResult for async completion; aborting session"
                    );
                    return Some(ShutdownOutcome::ResumeFailed(e.to_string()));
                }
            }

            // Re-derive the iteration count from the store for the live
            // job-completion resume (ADR-0038); no events are held in memory
            // here. The Arc/copy avoid borrowing `state` across the await.
            let store = Arc::clone(&state.store);
            let session_id = state.session_id;
            let model_calls =
                count_model_calls_from_store(store.as_ref(), session_id, turn_id).await;
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromToolDispatching {
                    pending: vec![],
                    completed: vec![(call_id, tool_result)],
                },
                model_calls,
                deps,
            );
            None
        }
        SessionCommand::InternalCancel { ack } => {
            // The cancel token is fired by the handle before sending this
            // command; just acknowledge receipt.
            let _ = ack.send(());
            None
        }
        SessionCommand::Shutdown { deadline, ack } => {
            let outcome = drain_shutdown(state, deadline, deps).await;
            let _ = ack.send(outcome);
            // Caller-requested shutdown is always a "clean" actor exit from
            // the spawn site's perspective; the detailed `ShutdownOutcome`
            // was already delivered to the caller via the oneshot ack above.
            Some(ShutdownOutcome::Clean {
                in_flight_cancelled: None,
            })
        }
        SessionCommand::CancelJob { job_id } => {
            // Best-effort cancel of the background job. The `JobManager`
            // implementation flips the job to `Cancelled` and fires the
            // already-registered completion sink, which the actor
            // dequeues on Arm 3 and re-injects as a `JobCompleted`
            // mailbox command. That command unwinds the paused turn via
            // the normal Arm-2 path, surfacing `ToolResult::Error {
            // kind: Cancelled }` to the next model call.
            if let Err(e) = deps.job_mgr.cancel(job_id).await {
                tracing::warn!(
                    session_id = %state.session_id,
                    %job_id,
                    error = %e,
                    "JobManager::cancel failed; turn may remain paused"
                );
            }
            None
        }
        SessionCommand::SnapshotInFlight { reply } => {
            // Mailbox-probe pattern: the handle asks the actor to look
            // at `state.in_flight` rather than holding a shared mutex,
            // which keeps `SessionState` single-owner per ADR-0006 §"Actor
            // model — why and how". A dropped receiver is harmless.
            let job_id = match &state.in_flight {
                Some(InFlight::PausedOnJob { job_id, .. }) => Some(*job_id),
                _ => None,
            };
            let _ = reply.send(job_id);
            None
        }
        SessionCommand::UpdateSession(_) => {
            // Intercepted in run_session before dispatch; never reaches here.
            None
        }
    }
}

/// Translate a terminal `JobOutcome` into the `ToolResult` that the resumed
/// turn sees in its `completed` list. `JobOutcome` is `#[non_exhaustive]`,
/// so unknown future variants surface as an `AsyncFailed` error rather than
/// a panic — the model still gets a well-formed `ToolResult` and the turn
/// can continue.
fn outcome_to_tool_result(outcome: JobOutcome) -> ToolResult {
    match outcome {
        JobOutcome::Success { result } => result,
        JobOutcome::Failed { message } => ToolResult::Error {
            kind: ToolErrorKind::AsyncFailed,
            message,
            retryable: false,
        },
        JobOutcome::Cancelled => ToolResult::Error {
            kind: ToolErrorKind::Cancelled,
            message: "job cancelled".into(),
            retryable: false,
        },
        // `JobOutcome` is `#[non_exhaustive]`; future variants land as a
        // generic AsyncFailed so the model still gets a structured error
        // rather than a panicking Brain.
        _ => ToolResult::Error {
            kind: ToolErrorKind::AsyncFailed,
            message: "unknown job outcome variant".into(),
            retryable: false,
        },
    }
}

/// Attempt to start a fresh turn from a caller-submitted `TurnTrigger`.
/// No-op if a turn is already in flight. Always uses
/// `TurnEntry::FreshLikeInit` — resume dispatch happens once at actor
/// startup via `apply_resume_point`, not here.
///
/// `TurnTrigger` projection (ADR-0016):
/// - `UserText(text)` -> `user_input = vec![ContentBlock::Text { text }]`,
///   `activate_skills = []`.
/// - `SkillActivation { names, user_text }` -> `user_input` is the
///   single-Text-block wrapping of `user_text` (empty when `user_text` is
///   `None` or empty), `activate_skills = names`.
///
/// Future variants (`UserContent` / `HookFired`) extend this match.
/// `#[non_exhaustive]` forces the `_ =>` arm; we log loudly and drop the
/// trigger rather than panic — a missed variant is a runtime bug, not a
/// turn failure.
async fn try_start_turn(state: &mut SessionState, trigger: TurnTrigger, deps: &SessionDeps) {
    if state.has_active_turn() {
        return;
    }

    // match_wildcard_for_single_variants: required while `TurnTrigger`
    //   is `#[non_exhaustive]` — omitting `_` would be a compile error
    //   even after future variants are added.
    #[allow(clippy::match_wildcard_for_single_variants)]
    let (user_input, activate_skills): (Vec<ContentBlock>, Vec<String>) = match trigger {
        TurnTrigger::UserText(text) => (vec![ContentBlock::Text { text }], Vec::new()),
        TurnTrigger::SkillActivation { names, user_text } => {
            // Empty / missing user_text yields empty user_input; the
            // SkillInjector's suffix is the only user-visible content
            // for the turn in that case.
            let user_input = match user_text {
                Some(t) if !t.is_empty() => vec![ContentBlock::Text { text: t }],
                _ => Vec::new(),
            };
            (user_input, names)
        }
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
        if let Err(e) = rec
            .record_turn_started(turn_id, user_input, activate_skills)
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

    spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit, 0, deps);
}

/// Record the terminal event after a turn finishes.
///
/// `deps` is threaded in so the pending-user-input single-slot queue can
/// be drained via `try_start_turn` once the turn is fully terminal. Drain
/// only fires when `state.in_flight == None` after outcome processing: the
/// `Paused` arm sets `in_flight = Some(PausedOnJob)` and therefore correctly
/// keeps the queued input parked until `JobCompleted` resumes (and later
/// terminates) the turn.
async fn on_turn_complete(
    state: &mut SessionState,
    turn_id: TurnId,
    outcome: TurnOutcome,
    deps: &SessionDeps,
) {
    // For the Paused outcome, resolve the `call_id` from the recorder cache
    // BEFORE acquiring the recorder lock below — `lookup_call_id_in_recorder`
    // takes the same `tokio::sync::Mutex` and we must not double-lock.
    let pause_call_id = match &outcome {
        TurnOutcome::Paused { job_id } => {
            Some(lookup_call_id_in_recorder(&state.recorder, *job_id).await)
        }
        _ => None,
    };

    // Default to clearing in_flight; the Paused arm overrides this below
    // *before* the persisted TurnPaused event is written so that an actor
    // crash between record + assignment can be reconstructed from the log.
    state.in_flight = None;

    let mut rec = state.recorder.lock().await;
    let result: Result<(), _> = match outcome {
        // TODO(double-turn-completed): the TurnDriver's model_completed::transit
        // already writes TurnCompleted via record_turn_completed before returning
        // TurnOutcome::Completed. Calling record_turn_completed again here
        // produces a duplicate persisted event AND a duplicate broadcast.
        // Fix in a separate change; tests/cancel_after_first_turn.rs currently
        // drains 2x TurnCompleted to tolerate this — that workaround must drop
        // to 1 when this is corrected.
        TurnOutcome::Completed => rec
            .record_turn_completed(turn_id, TurnOutcome::Completed)
            .await
            .map(|_| ()),
        // JobSubmitted must precede TurnPaused per H08's
        // write-before-transition contract. If the cache lookup misses, this
        // is an internal-invariant violation — fail the turn loudly rather
        // than store a sentinel `call_id` that would break Task 8's
        // `TurnEntry::FromToolDispatching` (an empty `tool_use_id` would
        // silently reach the model on resume). The persisted `TurnFailed`
        // lets resume-from-log recover cleanly on next restart, where
        // `harness::resume::lookup_call_id_in_events` walks the full log
        // and can succeed where the in-memory cache could not.
        TurnOutcome::Paused { job_id } => {
            if let Some(call_id) = pause_call_id.flatten() {
                state.in_flight = Some(InFlight::PausedOnJob {
                    turn_id,
                    job_id,
                    call_id,
                });
                rec.record_turn_paused(turn_id, job_id).await.map(|_| ())
            } else {
                tracing::error!(
                    session_id = %state.session_id,
                    %turn_id,
                    %job_id,
                    "TurnPaused without a preceding JobSubmitted in recorder cache; \
                     fatal: missing JobSubmitted; failing turn"
                );
                // Leave in_flight = None (already cleared above): the turn
                // is over from the actor's perspective. Resume-from-log
                // on the next session restart will re-derive the correct
                // state from the persisted event sequence.
                rec.record_turn_failed(
                    turn_id,
                    TurnFailureReason::TurnPanicked {
                        location:
                            "session_loop::on_turn_complete: TurnPaused without preceding JobSubmitted"
                                .into(),
                    },
                )
                .await
                .map(|_| ())
            }
        }
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

    // Release the recorder lock before re-entering `try_start_turn`, which
    // re-acquires it to record `TurnStarted`. Without this scope guard the
    // drain would deadlock on the same `tokio::sync::Mutex`.
    drop(rec);

    // Drain the single-slot queue if a user message arrived mid-turn — but
    // ONLY when the outcome was fully terminal. A `Paused` outcome leaves
    // `in_flight = Some(PausedOnJob)` and `is_paused()` will be true; the
    // queued trigger stays parked until `JobCompleted` resumes the turn and
    // the next `on_turn_complete` runs with a terminal outcome. Likewise,
    // the `JobSubmitted`-missing failure path clears `in_flight = None`
    // above, so the drain correctly fires and the user is not stranded.
    if state.in_flight.is_none()
        && let Some(pending) = state.pending_user_input.take()
    {
        try_start_turn(state, pending, deps).await;
    }
}

/// Live-path counterpart to [`crate::harness::resume::lookup_call_id_in_events`].
///
/// Reads the [`StepRecorder`]'s in-memory history cache for the most recent
/// `JobSubmitted { job_id, .. }` whose `job_id` matches and returns its
/// `call_id`. Returns `None` when no matching event is present in the cache
/// (structurally impossible if H08 honored its write-before-transition
/// contract, but treated as a soft failure here — the caller logs and
/// falls back rather than panicking).
async fn lookup_call_id_in_recorder(
    recorder: &Arc<Mutex<StepRecorder>>,
    job_id: JobId,
) -> Option<String> {
    let rec = recorder.lock().await;
    rec.history_cache_iter()
        .rev()
        .find_map(|e| match &e.payload {
            cogito_protocol::event::EventPayload::JobSubmitted {
                call_id,
                job_id: jid,
                ..
            } if *jid == job_id => Some(call_id.clone()),
            _ => None,
        })
}

/// Cancel the running turn and wait (up to `deadline`) for it to finish.
async fn drain_shutdown(
    state: &mut SessionState,
    deadline: Duration,
    deps: &SessionDeps,
) -> ShutdownOutcome {
    let started = Instant::now();
    // Signal the TurnDriver to stop cooperatively.
    state.current_cancel_token.lock().cancel();

    // Poll the turn-result channel until either the turn drains or the
    // deadline expires.
    while state.has_active_turn() && started.elapsed() < deadline {
        let remaining = deadline.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, state.turn_result_rx.recv()).await {
            Ok(Some((turn_id, outcome))) => {
                on_turn_complete(state, turn_id, outcome, deps).await;
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
    // Release store resources before returning so the ack the caller awaits
    // (sent by the `Shutdown` arm after this returns) implies the file handle
    // is already freed. See `release_store_resources`.
    release_store_resources(&state.store, state.session_id).await;
    ShutdownOutcome::Clean {
        in_flight_cancelled,
    }
}

/// Release per-session backend resources (file handle / connection slot) on
/// actor exit. ADR-0034 Option A: keeps `Runtime::close_session` (which frees
/// the in-memory registry slot) coupled with freeing the store's per-session
/// resources, so an idle-handle eviction actually reclaims the handle rather
/// than leaking it for the Runtime's lifetime.
///
/// Best-effort: the actor is exiting regardless, so a flush/close error is
/// logged, not propagated. `ConversationStore::close` permits a later `append`
/// to re-acquire resources, so a subsequent Resume on a fresh actor is
/// unaffected.
async fn release_store_resources(store: &Arc<dyn ConversationStore>, session_id: SessionId) {
    if let Err(e) = store.flush(session_id).await {
        tracing::warn!(session_id = %session_id, error = %e, "flush on actor exit failed");
    }
    if let Err(e) = store.close(session_id).await {
        tracing::warn!(session_id = %session_id, error = %e, "store close on actor exit failed");
    }
}

/// Record the `SessionStarted` event once at session open with an explicit
/// `SessionMeta`. The Runtime builds the meta (from the strategy for a
/// top-level session, or a caller-supplied override for a subagent child
/// that records parent linkage) and hands it here.
pub(super) async fn record_session_started_with_meta(
    recorder: &Arc<Mutex<StepRecorder>>,
    session_id: SessionId,
    meta: SessionMeta,
) {
    let mut rec = recorder.lock().await;
    if let Err(e) = rec.record_session_started(meta).await {
        tracing::error!(
            session_id = %session_id,
            error = %e,
            "failed to record SessionStarted event"
        );
    }
}

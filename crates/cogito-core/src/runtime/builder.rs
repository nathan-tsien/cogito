//! `Runtime` and `RuntimeBuilder` — the process-level DI container and
//! session registry. Callers inject all external dependencies at build time
//! and then open sessions to get back `SessionHandle`s.

use std::sync::Arc;

use cogito_jobs::LocalJobManager;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::job::JobManager;
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolProvider;
use dashmap::DashMap;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::handle::{SessionHandle, SessionShared};
use super::session_loop::{SessionDeps, SessionState, record_session_started_with_meta};
use super::types::{OpenMode, SessionId};
use crate::harness::step_recorder::StepRecorder;

/// The DI container and session registry. One `Runtime` per cogito-using
/// process is the typical pattern; opening N sessions spawns N session-loop
/// tasks (one tokio task per session) on the injected tokio runtime.
pub struct Runtime {
    /// Tokio runtime handle that all per-session loop tasks are spawned onto.
    handle: TokioHandle,
    /// Active sessions keyed by `SessionId`.
    sessions: DashMap<SessionId, SessionHandle>,
    /// Reserved for v0.4 process-level shutdown coordination (ADR-0010).
    #[allow(dead_code)] // v0.4 will wire this through shutdown_all()
    shutdown_token: CancellationToken,
    /// Injected conversation store.
    store: Arc<dyn ConversationStore>,
    /// Injected model gateway.
    model: Arc<dyn ModelGateway>,
    /// Injected tool provider.
    tools: Arc<dyn ToolProvider>,
    /// Default strategy applied to every new session.
    strategy: HarnessStrategy,
    /// Optional Skill loader provider. Required only when the strategy
    /// selects `SystemPromptInjectorConfig::Skill`; otherwise `None`.
    skills: Option<Arc<dyn SkillProvider>>,
    /// Async job manager shared across every session opened on this
    /// runtime. Defaulted to `LocalJobManager::new()` in
    /// `RuntimeBuilder::build`; tests inject a mock via
    /// `RuntimeBuilder::job_manager`. Surface code (CLI / consumer
    /// service) is expected to construct the same `Arc<LocalJobManager>`
    /// they thread into their `BuiltinToolProvider` and pass it here so
    /// that async tool submissions and Brain `on_complete` registrations
    /// resolve against the same manager instance.
    job_mgr: Arc<dyn JobManager>,
    /// Optional strategy registry, used by the subagent spawner to resolve
    /// a `delegate` role into a child `HarnessStrategy`. `None` => delegate
    /// returns `SpawnError::UnknownRole`.
    strategy_registry: Option<Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>>,
}

impl Runtime {
    /// Open or resume a session. See [`OpenMode`] for the three semantics.
    ///
    /// All three modes are dispatched as of Sprint 3 P4.3:
    ///
    /// - `OpenMode::New` — asserts the session does not exist in the store.
    /// - `OpenMode::Resume` — asserts the session exists in the store.
    /// - `OpenMode::Attach` — tolerant: accepts any store state including empty.
    ///
    /// # Errors
    ///
    /// - `RuntimeError::SessionAlreadyOpen` — `id` is already in the in-memory registry.
    /// - `RuntimeError::SessionAlreadyExists` — `OpenMode::New` but store has events for `id`.
    /// - `RuntimeError::ResumeFailed` — `OpenMode::Resume` but store has no events for `id`.
    /// - `RuntimeError::StoreError` — backend I/O or serde failure while reading the store.
    // DI container setup inherently lists many wiring steps; the line count is
    // structural, not complexity that can be usefully extracted.
    pub async fn open_session(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        let strategy = self.strategy.clone();
        self.open_inner(id, mode, strategy, None, 0, true).await
    }

    /// Internal open path shared by [`Runtime::open_session`] (top-level,
    /// `register = true`) and the subagent spawner (child sessions,
    /// `register = false`).
    ///
    /// - `strategy` — the per-session strategy (the default for top-level
    ///   sessions; the resolved child role for subagent sessions).
    /// - `meta_override` — when `Some`, used verbatim as the `SessionMeta`
    ///   recorded for a fresh session (carries parent linkage + depth for a
    ///   subagent child). `None` derives the meta from `strategy`, preserving
    ///   the historical top-level behavior byte-for-byte.
    /// - `subagent_depth` — flowed into every turn's `ExecCtx`.
    /// - `register` — when `false`, the session is not added to the in-memory
    ///   `sessions` registry (used for ephemeral subagent children).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn open_inner(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
        strategy: HarnessStrategy,
        meta_override: Option<cogito_protocol::SessionMeta>,
        subagent_depth: u32,
        register: bool,
    ) -> Result<SessionHandle, RuntimeError> {
        use futures::TryStreamExt as _;

        if register && self.sessions.contains_key(&id) {
            return Err(RuntimeError::SessionAlreadyOpen { id });
        }

        // Read latest_seq once; it serves two purposes:
        //   1. existence check — `Some(_)` means the session has events in the store.
        //      (We use latest_seq, not replay(id, 0), because replay yields events
        //      with seq > 0 strictly and would miss a session that only has the
        //      seq=0 SessionStarted event.)
        //   2. seq_start for StepRecorder — for Resume/Attach against an existing
        //      session, new events must start at latest_seq + 1 to avoid colliding
        //      with persisted events.
        let latest_seq = self
            .store
            .latest_seq(id)
            .await
            .map_err(|e| RuntimeError::StoreError(e.to_string()))?;
        let session_exists = latest_seq.is_some();

        // Collect events with seq > 0 for downstream consumption by P4.4's H03 replay.
        // SessionStarted (seq=0) is excluded here; P4.4 reconstructs context from seq>=1
        // events via H03 apply_resume_point.
        let initial_events: Vec<cogito_protocol::ConversationEvent> = self
            .store
            .replay(id, 0)
            .try_collect()
            .await
            .map_err(|e| RuntimeError::StoreError(e.to_string()))?;

        match mode {
            OpenMode::New => {
                if session_exists {
                    return Err(RuntimeError::SessionAlreadyExists { id });
                }
            }
            OpenMode::Resume => {
                if !session_exists {
                    return Err(RuntimeError::ResumeFailed {
                        id,
                        reason: "no such session in store".into(),
                    });
                }
            }
            OpenMode::Attach => {
                // Tolerant: accept any store state (empty or non-empty).
            }
        }

        // Channels.
        let (mailbox_tx, mailbox_rx) = mpsc::channel::<super::types::SessionCommand>(64);
        let (job_tx, job_rx) = mpsc::channel(32);
        let (broadcast_tx, _) = broadcast::channel::<cogito_protocol::stream::StreamEvent>(256);
        let (turn_result_tx, turn_result_rx) = mpsc::channel::<(
            cogito_protocol::ids::TurnId,
            cogito_protocol::turn::TurnOutcome,
        )>(4);

        // Per-session cancel token; starts as a fresh token.
        let cancel = Arc::new(parking_lot::Mutex::new(CancellationToken::new()));

        // Step recorder shared between actor and TurnDeps. For a fresh session
        // latest_seq is None → seq_start=0. For Resume/Attach against an
        // existing session, seq_start=latest_seq+1 so new events do not collide
        // with persisted ones.
        let seq_start = latest_seq.map_or(0, |s| s + 1);
        let recorder = Arc::new(Mutex::new(StepRecorder::new(
            Arc::clone(&self.store),
            broadcast_tx.clone(),
            id,
            seq_start,
        )));

        // Write SessionStarted exactly once per session, gated on the store
        // existence check. Kept here (not in run_session) so that the session
        // loop stays stateless with respect to session lifecycle: every event
        // the loop writes is correlated with a turn, not the session itself.
        // See run_session's startup-sequence doc.
        if !session_exists {
            // Top-level (no override) derives the meta from `strategy`, which
            // reproduces the historical `record_session_started` payload
            // exactly. A subagent child supplies an override that adds parent
            // linkage + depth.
            let meta = meta_override.unwrap_or_else(|| cogito_protocol::SessionMeta {
                cogito_version: env!("CARGO_PKG_VERSION").into(),
                strategy: Some(strategy.name.clone()),
                model: Some(strategy.model_params.model.clone()),
                ..Default::default()
            });
            record_session_started_with_meta(&recorder, id, meta).await;
        }

        // Build metrics first so the hook pipeline shares the same Arc rather
        // than embedding its own private NoOpMetricsRecorder. Sprint 6 (Context
        // C2) needs to record context-decision metrics directly via
        // TurnDeps.metrics, not via hooks — so keep both fields in sync here.
        let metrics: Arc<dyn cogito_protocol::MetricsRecorder> =
            Arc::new(cogito_protocol::NoOpMetricsRecorder);
        let hooks = Arc::new(
            crate::harness::hooks::CompositeHookPipeline::with_handlers_and_metrics(
                Vec::new(),
                Arc::clone(&metrics),
            ),
        );

        // Build the context pipeline once per session from `strategy.context`.
        // All turns share this same Arc; no per-turn rebuild is needed.
        // `build_pipeline_v2` threads the optional `SkillProvider` into the
        // pipeline so the `SkillInjector` (when selected) gets its handle.
        let context_pipeline = Arc::new(
            cogito_context::build_pipeline_v2(&strategy.context, self.skills.clone()).map_err(
                |e| RuntimeError::ResumeFailed {
                    id,
                    reason: e.to_string(),
                },
            )?,
        );

        let state = SessionState {
            session_id: id,
            strategy: strategy.clone(),
            in_flight: None,
            current_cancel_token: Arc::clone(&cancel),
            job_completion_rx: job_rx,
            // Keep a clone of the sender on the actor state so every
            // per-turn `TurnDeps` (built in `spawn_turn_driver`) can hand
            // it to `JobManager::on_complete`. The `SessionShared` clone
            // (below) is the path used by `SessionHandle::submit` / the
            // legacy external path; both halves point at the same channel.
            job_completion_tx: job_tx.clone(),
            turn_result_rx,
            turn_result_tx,
            broadcast_tx: broadcast_tx.clone(),
            recorder: Arc::clone(&recorder),
            store: Arc::clone(&self.store),
            hooks,
            metrics,
            context_pipeline,
            skills: self.skills.clone(),
            pending_user_input: None,
            subagent_depth,
        };

        let deps = SessionDeps {
            model: Arc::clone(&self.model),
            tools: Arc::clone(&self.tools),
            job_mgr: Arc::clone(&self.job_mgr),
            // Every session (top-level and child) carries a spawner so a child
            // can itself delegate up to the depth limit (enforced by the
            // `delegate` tool, not here). NOTE: the spawned actor task owns this
            // Arc<Runtime> clone, so the Runtime stays alive until each
            // session's actor exits (mailbox close / shutdown) - dropping
            // external Arc<Runtime> handles alone does not tear it down. This is
            // intentional: a child mid-delegate must keep the Runtime alive.
            // There is no Arc cycle (SessionHandle holds no back-reference to
            // Runtime).
            brain_spawner: Some(Arc::new(RuntimeSpawner(Arc::clone(self)))
                as Arc<dyn cogito_protocol::subagent::BrainSpawner>),
        };

        let mailbox_tx_for_loop = mailbox_tx.clone();
        // P4.4: capture the loop's `ShutdownOutcome` so non-Clean exits
        // (resume-failed, JobManager-unavailable) surface in the log even
        // though `open_session` has already returned the handle. The loop
        // task is fire-and-forget; future sprints may add a startup-result
        // channel for synchronous error surfacing.
        let session_id_for_log = id;
        self.handle.spawn(async move {
            let outcome = super::session_loop::run_session(
                state,
                mailbox_rx,
                mailbox_tx_for_loop,
                deps,
                initial_events,
            )
            .await;
            if !matches!(outcome, super::types::ShutdownOutcome::Clean { .. }) {
                tracing::error!(
                    session_id = %session_id_for_log,
                    ?outcome,
                    "actor exited with non-Clean outcome"
                );
            }
        });

        let shared = Arc::new(SessionShared {
            session_id: id,
            mailbox_tx,
            events_tx: broadcast_tx,
            // Share the SAME Arc<Mutex<CancellationToken>> with SessionState
            // so that the actor's per-turn swap (in spawn_turn_driver) is
            // visible to every SessionHandle clone. A sibling clone of the
            // initial token would silently no-op for every cancel after
            // turn 1 — see the cancel_after_first_turn regression test.
            current_cancel_token: Arc::clone(&cancel),
            job_completion_tx: job_tx,
        });
        let handle = SessionHandle::new(shared);
        if register {
            self.sessions.insert(id, handle.clone());
        }
        Ok(handle)
    }

    /// Begin a builder. Caller injects all dependencies.
    #[must_use]
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }
}

/// Builder for [`Runtime`].
#[derive(Default)]
pub struct RuntimeBuilder {
    handle: Option<TokioHandle>,
    shutdown_token: Option<CancellationToken>,
    store: Option<Arc<dyn ConversationStore>>,
    model: Option<Arc<dyn ModelGateway>>,
    tools: Option<Arc<dyn ToolProvider>>,
    strategy: Option<HarnessStrategy>,
    skills: Option<Arc<dyn SkillProvider>>,
    job_mgr: Option<Arc<dyn JobManager>>,
    strategy_registry: Option<Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>>,
}

impl RuntimeBuilder {
    /// Override the tokio `Handle`. Defaults to `Handle::current()` at `build()`.
    #[must_use]
    pub fn handle(mut self, handle: TokioHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Reserve a process-level cancellation token (v0.4 shutdown support).
    #[must_use]
    pub fn shutdown_token(mut self, token: CancellationToken) -> Self {
        self.shutdown_token = Some(token);
        self
    }

    /// Inject the conversation store.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn ConversationStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Inject the model gateway.
    #[must_use]
    pub fn model(mut self, model: Arc<dyn ModelGateway>) -> Self {
        self.model = Some(model);
        self
    }

    /// Inject the tool provider.
    #[must_use]
    pub fn tools(mut self, tools: Arc<dyn ToolProvider>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the default harness strategy applied to every new session.
    #[must_use]
    pub fn strategy(mut self, strategy: HarnessStrategy) -> Self {
        self.strategy = Some(strategy);
        self
    }

    /// Inject a `SkillProvider`. Optional — required only when the strategy
    /// selects `SystemPromptInjectorConfig::Skill`.
    #[must_use]
    pub fn skills(mut self, skills: Arc<dyn SkillProvider>) -> Self {
        self.skills = Some(skills);
        self
    }

    /// Override the default `LocalJobManager`. Surface code passes the
    /// same `Arc<LocalJobManager>` it threads into `RunTestsTool::new`
    /// (or any other async-tool constructor) so async tool submissions
    /// and the Brain's `on_complete` registrations resolve against one
    /// shared manager (see ADR-0008 and ADR-0025). Tests use this hook
    /// to inject a `MockJobManager` for deterministic control over job
    /// completion.
    #[must_use]
    pub fn job_manager(mut self, job_mgr: Arc<dyn JobManager>) -> Self {
        self.job_mgr = Some(job_mgr);
        self
    }

    /// Inject a strategy registry so the subagent `delegate` tool can
    /// resolve roles. Optional - without it, `delegate` errors on any role.
    #[must_use]
    pub fn strategy_registry(
        mut self,
        registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>,
    ) -> Self {
        self.strategy_registry = Some(registry);
        self
    }

    /// Finalize.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeError::NoTokioRuntime` if no `Handle` was injected
    /// and `Handle::try_current()` fails.  Returns `RuntimeError::Missing*`
    /// if a required dependency was not provided.
    pub fn build(self) -> Result<Arc<Runtime>, RuntimeError> {
        let handle = match self.handle {
            Some(h) => h,
            None => TokioHandle::try_current()
                .map_err(|e| RuntimeError::NoTokioRuntime(e.to_string()))?,
        };
        let store = self.store.ok_or(RuntimeError::MissingDependency("store"))?;
        let model = self.model.ok_or(RuntimeError::MissingDependency("model"))?;
        let tools = self.tools.ok_or(RuntimeError::MissingDependency("tools"))?;
        let strategy = self
            .strategy
            .ok_or(RuntimeError::MissingDependency("strategy"))?;

        // Default to an in-process `LocalJobManager`. `LocalJobManager::new`
        // already returns `Arc<LocalJobManager>` which coerces to the trait
        // object via the unsized-coercion impl on `Arc<T>`. The
        // `job_manager` setter takes precedence so surface code can hand
        // the SAME `Arc<LocalJobManager>` to both `BuiltinToolProvider`
        // (typed for `submit`) and the Runtime (typed for `on_complete`).
        let job_mgr: Arc<dyn JobManager> = self
            .job_mgr
            .unwrap_or_else(|| LocalJobManager::new() as Arc<dyn JobManager>);

        Ok(Arc::new(Runtime {
            handle,
            sessions: DashMap::new(),
            shutdown_token: self.shutdown_token.unwrap_or_default(),
            store,
            model,
            tools,
            strategy,
            skills: self.skills,
            job_mgr,
            strategy_registry: self.strategy_registry,
        }))
    }
}

/// Errors from the Runtime layer surface (not from inside a turn).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// `Handle::try_current()` failed at build time.
    #[error("no current tokio runtime: {0}")]
    NoTokioRuntime(String),
    /// The session id was already open in this `Runtime` (in-memory registry).
    #[error("session {id:?} already open in runtime")]
    SessionAlreadyOpen {
        /// The session id that is already open.
        id: SessionId,
    },
    /// The session id already exists in the backing store (`OpenMode::New` collision).
    #[error("session {id:?} already exists in store")]
    SessionAlreadyExists {
        /// The session id that collided.
        id: SessionId,
    },
    /// Resume-phase failure for `OpenMode::Resume` or `OpenMode::Attach`.
    #[error("resume failed for session {id:?}: {reason}")]
    ResumeFailed {
        /// The session id for which resume was attempted.
        id: SessionId,
        /// Human-readable description of why the resume failed.
        reason: String,
    },
    /// A required dependency was not set on the builder.
    #[error("missing required dependency: {0}")]
    MissingDependency(&'static str),
    /// Backend store I/O or serde failure during `open_session`.
    #[error("store error during open: {0}")]
    StoreError(String),
}

/// Backstop deadline for driving a subagent child to a terminal turn.
/// A child that neither completes nor fails within this budget (e.g. a
/// wedged tool) must not hang the parent turn forever. Generous on purpose;
/// per-role configurability is a v0.3 item (ADR-0011).
const CHILD_DRIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Grace period for tearing the child actor down after its turn is terminal.
const CHILD_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Owns an `Arc<Runtime>` so `run_to_completion` has the Arc that
/// `open_inner` needs (and so a spawned child can itself delegate).
pub(crate) struct RuntimeSpawner(pub(crate) Arc<Runtime>);

#[async_trait::async_trait]
impl cogito_protocol::subagent::BrainSpawner for RuntimeSpawner {
    async fn run_to_completion(
        &self,
        req: cogito_protocol::subagent::DelegateRequest,
    ) -> Result<String, cogito_protocol::subagent::SpawnError> {
        use cogito_protocol::stream::StreamEvent;
        use cogito_protocol::subagent::SpawnError;
        use futures::TryStreamExt as _;

        let rt = &self.0; // &Arc<Runtime>

        // 1. Resolve the role -> child strategy.
        let registry = rt
            .strategy_registry
            .as_ref()
            .ok_or_else(|| SpawnError::UnknownRole {
                role: req.role.clone(),
            })?;
        let strategy = registry
            .get(&req.role)
            .map_err(|_| SpawnError::UnknownRole {
                role: req.role.clone(),
            })?;

        // 2. Child SessionMeta (linkage recorded child-side only).
        let child_id = SessionId::new();
        let meta = cogito_protocol::SessionMeta {
            cogito_version: env!("CARGO_PKG_VERSION").into(),
            strategy: Some(strategy.name.clone()),
            model: Some(strategy.model_params.model.clone()),
            parent_session_id: Some(req.parent_session_id),
            parent_call_id: Some(req.parent_call_id.clone()),
            subagent_depth: req.parent_depth + 1,
            ..Default::default()
        };

        // 3. Open the child as an unregistered top-level session.
        let child = rt
            .open_inner(
                child_id,
                OpenMode::New,
                strategy,
                Some(meta),
                req.parent_depth + 1,
                false,
            )
            .await
            .map_err(|e| SpawnError::OpenFailed {
                reason: e.to_string(),
            })?;

        // 4. Drive to a terminal turn via the broadcast stream, bounded by a
        //    backstop deadline so a wedged child can't hang the parent turn.
        //    The child does not inherit the parent cancel token in v0.2, so
        //    this timeout is the only guard against an unbounded child turn.
        let mut rx = child.subscribe();
        child
            .submit_user_text(req.input)
            .await
            .map_err(|e| SpawnError::OpenFailed {
                reason: e.to_string(),
            })?;
        let drive = tokio::time::timeout(CHILD_DRIVE_TIMEOUT, async {
            loop {
                match rx.recv().await {
                    Ok(StreamEvent::TurnFailed { reason, .. }) => return Some(reason),
                    // Terminal completion, or a lagged/closed broadcast: in both
                    // cases stop waiting and fall through to the log replay, which
                    // is the source of truth for the child's final assistant text.
                    Ok(StreamEvent::TurnCompleted { .. }) | Err(_) => return None,
                    // Intermediate events (paused/resumed/deltas) - keep waiting.
                    Ok(_) => {}
                }
            }
        })
        .await;

        // 5. Tear the child actor down regardless of how the drive ended.
        let _ = child.shutdown(CHILD_SHUTDOWN_GRACE).await;
        let Ok(failure) = drive else {
            return Err(SpawnError::Timeout {
                seconds: CHILD_DRIVE_TIMEOUT.as_secs(),
            });
        };
        if let Some(reason) = failure {
            return Err(SpawnError::ChildFailed { reason });
        }

        // 6. Extract the final assistant text from the child log.
        let events: Vec<cogito_protocol::ConversationEvent> = rt
            .store
            .replay(child_id, 0)
            .try_collect()
            .await
            .map_err(|e| SpawnError::OpenFailed {
                reason: e.to_string(),
            })?;
        // A child that completed with no assistant message yields an empty
        // string for v0.2. Distinguishing "completed-empty" from real output
        // (so the parent can surface a clearer signal) is a v0.3 item.
        Ok(last_assistant_text(&events).unwrap_or_default())
    }
}

/// Walk events newest-first; return the last non-empty assistant message
/// text. `EventPayload::AssistantMessageAppended { text }` is the flat-text
/// shape used by the event log.
fn last_assistant_text(events: &[cogito_protocol::ConversationEvent]) -> Option<String> {
    use cogito_protocol::event::EventPayload;
    events.iter().rev().find_map(|ev| match &ev.payload {
        EventPayload::AssistantMessageAppended { text } if !text.is_empty() => Some(text.clone()),
        _ => None,
    })
}

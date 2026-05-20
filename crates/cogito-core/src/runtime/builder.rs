//! `Runtime` and `RuntimeBuilder` — the process-level DI container and
//! session registry. Callers inject all external dependencies at build time
//! and then open sessions to get back `SessionHandle`s.

use std::sync::Arc;

use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolProvider;
use dashmap::DashMap;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::actor::{ActorDeps, ActorState, record_session_started};
use super::handle::{SessionHandle, SessionShared};
use super::types::{OpenMode, SessionId};
use crate::harness::step_recorder::StepRecorder;

/// The DI container and session registry. One `Runtime` per cogito-using
/// process is the typical pattern; opening N sessions spawns N actor tasks
/// on the injected tokio runtime.
pub struct Runtime {
    /// Tokio runtime handle that all `SessionActor` tasks are spawned onto.
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
}

impl Runtime {
    /// Open or attach a session. See [`OpenMode`] for the three semantics.
    ///
    /// v0.1 only implements `OpenMode::New`. `Resume` and `Attach` will be
    /// wired through H03 in Sprint 3.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeError::SessionAlreadyOpen` if `id` is already in the
    /// registry.
    pub async fn open_session(
        self: &Arc<Self>,
        id: SessionId,
        _mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        if self.sessions.contains_key(&id) {
            return Err(RuntimeError::SessionAlreadyOpen { id });
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

        // Step recorder shared between actor and TurnDeps.
        let recorder = Arc::new(Mutex::new(StepRecorder::new(
            Arc::clone(&self.store),
            broadcast_tx.clone(),
            id,
            0, // seq_start = 0 for a fresh session (Resume will pass latest_seq+1)
        )));

        // Write SessionStarted before the actor task starts.
        record_session_started(&recorder, id, &self.strategy).await;

        let state = ActorState {
            session_id: id,
            strategy: self.strategy.clone(),
            in_flight: None,
            current_cancel_token: Arc::clone(&cancel),
            job_completion_rx: job_rx,
            turn_result_rx,
            turn_result_tx,
            broadcast_tx: broadcast_tx.clone(),
            recorder: Arc::clone(&recorder),
            store: Arc::clone(&self.store),
        };

        let deps = ActorDeps {
            model: Arc::clone(&self.model),
            tools: Arc::clone(&self.tools),
        };

        let mailbox_tx_for_actor = mailbox_tx.clone();
        self.handle.spawn(super::actor::actor_main(
            state,
            mailbox_rx,
            mailbox_tx_for_actor,
            deps,
        ));

        let shared = Arc::new(SessionShared {
            session_id: id,
            mailbox_tx,
            events_tx: broadcast_tx,
            current_cancel_token: parking_lot::Mutex::new(cancel.lock().clone()),
            job_completion_tx: job_tx,
        });
        let handle = SessionHandle::new(shared);
        self.sessions.insert(id, handle.clone());
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

        Ok(Arc::new(Runtime {
            handle,
            sessions: DashMap::new(),
            shutdown_token: self.shutdown_token.unwrap_or_default(),
            store,
            model,
            tools,
            strategy,
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
}

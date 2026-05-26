//! `TurnDeps` — protocol-level trait objects injected into every FSM
//! transition. Constructed by `runtime::session_loop::try_start_turn` (or
//! by the test harness) and borrowed by all transition functions.

use std::sync::Arc;

use cogito_protocol::ContextPipeline;
use cogito_protocol::MetricsRecorder;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::job::{JobCompletionEvent, JobManager};
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::tool::ToolProvider;
use tokio::sync::{Mutex, mpsc};

use crate::harness::hooks::CompositeHookPipeline;
use crate::harness::step_recorder::StepRecorder;

/// All external dependencies a transition function may need.
///
/// `step` is wrapped in `Arc<Mutex<...>>` because transitions are `async`
/// and `StepRecorder` takes `&mut self`; sharing across `.await` points
/// requires an async-aware mutex.
///
/// `store` is threaded separately so transitions can call
/// `store.replay(session_id, 0)` to build prompt history without going
/// through the recorder.
pub struct TurnDeps {
    /// The step recorder. Lock before calling any `record_*` method.
    pub step: Arc<Mutex<StepRecorder>>,
    /// The conversation event store, used for history replay in H04.
    pub store: Arc<dyn ConversationStore>,
    /// Gateway for model API calls.
    pub model: Arc<dyn ModelGateway>,
    /// Tool provider exposed to the model.
    pub tools: Arc<dyn ToolProvider>,
    /// Hook pipeline (Sprint 5: lifecycle methods wired in Task 5).
    pub hooks: Arc<CompositeHookPipeline>,
    /// Metrics sink for this turn. Defaults to `NoOpMetricsRecorder`; a real
    /// adapter will be wired in v0.4. Sprint 6 (Context C2) records
    /// context-decision metrics directly via this field (not via hooks), so
    /// the field is intentionally separate from `hooks.metrics` — both share
    /// the same `Arc` as of Sprint 5 (builder.rs wires them together).
    pub metrics: Arc<dyn MetricsRecorder>,
    /// H11 context-management pipeline. Built from `strategy.context` at
    /// session open. Task 31 will move construction to `SessionShared`; for
    /// Sprint 6 it is built in `spawn_turn_driver` from the per-turn strategy.
    pub context_pipeline: Arc<ContextPipeline>,
    /// Optional Skill loader provider. `None` for sessions whose strategy
    /// does NOT select `SystemPromptInjectorConfig::Skill`. H06 uses it to
    /// gate sigil detection; H11's `SkillInjector` holds its own `Arc`
    /// internally.
    pub skills: Option<Arc<dyn SkillProvider>>,
    /// Async job manager shared across all sessions. H08's async tool
    /// dispatch path (Task 12) submits jobs against this manager and
    /// registers `job_completion_tx` as the completion sink so terminal
    /// outcomes land on the session loop's job-completion arm.
    pub job_mgr: Arc<dyn JobManager>,
    /// Per-session completion sink. Cloned from `SessionState` on every
    /// turn spawn; the dispatcher hands this sender to
    /// `JobManager::on_complete(job_id, sink)` so the `JobCompletionEvent`
    /// is routed back to the session's mailbox loop (Arm 3 in
    /// `run_session`).
    pub job_completion_tx: mpsc::Sender<JobCompletionEvent>,
}

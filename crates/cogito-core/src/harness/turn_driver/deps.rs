//! `TurnDeps` — protocol-level trait objects injected into every FSM
//! transition. Constructed by `runtime::session_loop::try_start_turn` (or
//! by the test harness) and borrowed by all transition functions.

use std::sync::Arc;

use cogito_protocol::MetricsRecorder;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::tool::ToolProvider;
use tokio::sync::Mutex;

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
}

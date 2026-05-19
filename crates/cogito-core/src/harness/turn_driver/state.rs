//! `TurnState` FSM enum + `TurnCtx` shared invariants.
//!
//! Hybrid form: `TurnCtx` carries the session/turn identifiers that every
//! transition needs; state-specific data is stored as enum payload fields.

use std::collections::VecDeque;

use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelOutput};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::job::JobId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::{ToolDescriptor, ToolResult};
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use futures::stream::BoxStream;

use crate::harness::resume::ResumeDecision;
use crate::harness::tool_resolver::ToolInvocation;

/// Maximum number of consecutive inner-loop iterations where every tool call
/// returned a validation/dispatch error before the FSM gives up.
///
/// This prevents infinite loops when a model repeatedly calls tools with bad
/// arguments (e.g. missing required fields) and never makes progress.
pub const MAX_CONSECUTIVE_TOOL_ERRORS: u32 = 4;

/// Immutable identifiers carried through every FSM state.
#[derive(Clone)]
pub struct TurnCtx {
    /// Session this turn belongs to.
    pub session_id: SessionId,
    /// Unique identifier for this turn.
    pub turn_id: TurnId,
    /// Execution context threaded through tool calls and model calls.
    pub exec_ctx: ExecCtx,
    /// Per-turn strategy (model, tools, hooks configuration).
    pub strategy: HarnessStrategy,
    /// Number of consecutive inner-loop rounds where every resolved tool call
    /// was an error (validation failure, dispatch error, hook rejection).
    /// Resets to zero whenever at least one tool call succeeds in a round.
    /// When it reaches [`MAX_CONSECUTIVE_TOOL_ERRORS`] the FSM transitions to
    /// `Failed` rather than sending another request to the model.
    pub consecutive_tool_errors: u32,
}

/// One state in the H01 FSM.
///
/// Terminal states: `Completed`, `Paused`, `Failed`.
/// Non-terminal states carry a `TurnCtx` and any state-specific data.
pub enum TurnState {
    /// Starting state. The resume coordinator has not yet run.
    Init {
        /// Shared turn context.
        ctx: TurnCtx,
        /// Resume decision (always `FreshTurn` in Sprint 2).
        resume: ResumeDecision,
    },
    /// Context management (H11) has been entered. Sprint 2 is a pass-through.
    ContextManaged {
        /// Shared turn context.
        ctx: TurnCtx,
    },
    /// H04 composed the prompt; awaiting gateway stream open.
    PromptBuilt {
        /// Shared turn context.
        ctx: TurnCtx,
        /// Fully-composed model input.
        input: cogito_protocol::gateway::ModelInput,
        /// Tool surface active for this call.
        surface: Vec<ToolDescriptor>,
    },
    /// Gateway stream is open; H06 is consuming events.
    ModelCalling {
        /// Shared turn context.
        ctx: TurnCtx,
        /// Live event stream from the provider.
        stream: BoxStream<'static, Result<ModelEvent, ModelError>>,
        /// Tool surface active for this call.
        surface: Vec<ToolDescriptor>,
    },
    /// H06 has sealed the output; H07 resolver has not yet run.
    ModelCompleted {
        /// Shared turn context.
        ctx: TurnCtx,
        /// Sealed assistant output.
        output: ModelOutput,
        /// Tool surface active for this call.
        surface: Vec<ToolDescriptor>,
    },
    /// H07+H08 are dispatching queued tool calls one by one.
    ToolDispatching {
        /// Shared turn context.
        ctx: TurnCtx,
        /// Tool calls not yet dispatched.
        pending: VecDeque<ToolInvocation>,
        /// Tool calls that have already returned a result.
        completed: Vec<(String, ToolResult)>,
        /// Tool surface snapshot for this dispatch round.
        surface: Vec<ToolDescriptor>,
    },
    /// Turn ended normally; model returned `end_turn` with no tool calls.
    Completed {
        /// All content blocks the model emitted in the final response.
        final_assistant_content: Vec<ContentBlock>,
    },
    /// Turn is suspended waiting for an async job.
    Paused {
        /// The async job this turn is waiting on.
        job_id: JobId,
    },
    /// Turn ended with an unrecoverable error.
    Failed {
        /// Structured reason for the failure.
        reason: TurnFailureReason,
    },
}

impl TurnState {
    /// Convert a terminal state into the protocol-level `TurnOutcome`.
    ///
    /// Calling this on a non-terminal state is a programming error; it
    /// returns `TurnOutcome::Failed` with `TurnFailureReason::TurnPanicked`
    /// so the caller can record it without panicking.
    pub fn into_outcome(self) -> TurnOutcome {
        match self {
            TurnState::Completed { .. } => TurnOutcome::Completed,
            TurnState::Paused { job_id } => TurnOutcome::Paused { job_id },
            TurnState::Failed { reason } => TurnOutcome::Failed {
                reason,
                // Sprint 2 limitation: the event_id of the TurnFailed
                // event is not plumbed back through record_turn_failed yet.
                // Sprint 3 will thread the real EventId.
                recorded_event_id: "unknown".into(),
            },
            _ => TurnOutcome::Failed {
                reason: TurnFailureReason::TurnPanicked {
                    location: "into_outcome called on non-terminal state".into(),
                },
                recorded_event_id: "unknown".into(),
            },
        }
    }
}

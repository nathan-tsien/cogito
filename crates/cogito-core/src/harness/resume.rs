//! H03 Resume Coordinator — Sprint 2 stub. Always returns `FreshTurn`.
//! Sprint 3 implements the decision table from ADR-0003.

use cogito_protocol::event::ConversationEvent;
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::tool::{ToolDescriptor, ToolResult};

use crate::harness::tool_resolver::ToolInvocation;

/// What `enter_turn` should do as the starting state of the FSM.
#[derive(Debug, Clone)]
pub enum ResumeDecision {
    /// Fresh user input; start at `Init`.
    FreshTurn,
    /// Resume mid-turn at `ToolDispatching` with the prior pending /
    /// completed sets. (Sprint 3 will actually emit this.)
    ResumeFromToolDispatching {
        /// Tool calls that have not yet been dispatched.
        pending: Vec<ToolInvocation>,
        /// Tool calls that completed, paired with their results.
        completed: Vec<(String, ToolResult)>,
        /// Snapshot of the tool surface at the time the turn was paused.
        surface_snapshot: Vec<ToolDescriptor>,
    },
    /// Resume at `ModelCompleted` carrying a previously-fully-streamed
    /// output. (Sprint 3 will actually emit this.)
    ResumeFromModelCompleted {
        /// The full model output that was streamed before the crash.
        output: ModelOutput,
        /// Snapshot of the tool surface at the time of the model call.
        surface_snapshot: Vec<ToolDescriptor>,
    },
}

/// Errors from `replay`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    /// The event log is in an unexpected or contradictory state.
    #[error("malformed event log: {0}")]
    Malformed(String),
    /// The event log was written by a newer schema version.
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
}

/// Sprint 2 stub: always returns `FreshTurn` regardless of the event log.
/// Sprint 3 implements the real decision table.
///
/// # Errors
///
/// Currently does not error; signature reserves room for Sprint 3.
pub fn replay(_events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    Ok(ResumeDecision::FreshTurn)
}

//! Context Management protocol surface — traits, event-emitting types, config.
//!
//! See `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
//! and ADR-0008 for the full design. Implementations live in `cogito-context`.

use thiserror::Error;

use crate::gateway::ModelError;
use crate::store::StoreError;

/// Failure mode for any of the four Context-Management traits.
///
/// Per ADR-0008, H11 records the error into
/// `ContextDecisionRecorded.errors` and continues the turn (degrade, not block).
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum ContextError {
    /// Compactor's summarization model call failed.
    #[error("summarization model call failed: {0}")]
    SummarizationModelFailed(#[from] ModelError),

    /// An invariant was violated (range bounds, turn alignment,
    /// duplicate compaction for turn).
    #[error("invariant violated: {0}")]
    InvariantViolated(String),

    /// Operation was aborted (cancel-token fired).
    #[error("operation aborted")]
    Aborted,

    /// Underlying conversation store rejected the write.
    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

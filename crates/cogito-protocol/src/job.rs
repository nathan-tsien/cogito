//! Async job lifecycle contract.
//!
//! `JobManager` exposes status/result/cancel + an `on_complete` callback
//! registration. Submission lives on the concrete `LocalJobManager` type
//! in cogito-jobs (only async-tool implementations submit jobs; Brain
//! only observes via this trait). See spec §6.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use ulid::Ulid;

use crate::tool::ToolResult;

/// Opaque job identifier. Currently a Ulid so order corresponds to
/// submission time within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
// schemars attribute lives on the inner field; the struct is
// `#[serde(transparent)]` so the wire schema is the inner type's schema
// (a String rendering of the ULID). Mirrors the pattern used in `ids.rs`.
pub struct JobId(#[schemars(with = "String")] Ulid);

impl Default for JobId {
    fn default() -> Self {
        Self(Ulid::new())
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Accepted by the manager but not yet scheduled.
    Pending,
    /// A worker is actively executing the job.
    Running,
    /// Reached a terminal successful state.
    Completed,
    /// Reached a terminal error state.
    Failed,
    /// Reached a terminal cancelled state (via `JobManager::cancel`).
    Cancelled,
}

/// Terminal outcome of a job, delivered through `on_complete` and
/// `result`.
///
/// **Serde representation note**: internally-tagged with `tag = "kind"`.
/// All variants are unit or struct (no newtype-with-primitive), and no
/// inner field collides with the tag name, so internal tagging is safe.
///
/// Note: `PartialEq` is derived but not `Eq` because `ToolResult::Output`
/// wraps `serde_json::Value`, which does not implement `Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobOutcome {
    /// Tool result produced by the job. The wire format matches
    /// `ToolResult::Output`; the actor wraps it back into `ToolResult`
    /// when resuming the turn.
    Success {
        /// The terminal tool result for this job.
        result: ToolResult,
    },
    /// Job failed; the tool will see `ToolResult::Error { kind: AsyncFailed }`.
    Failed {
        /// Human-readable error description.
        message: String,
    },
    /// Job was cancelled before completion (by `JobManager::cancel`).
    Cancelled,
}

/// Event sent by `JobManager` to the registered sink when a job reaches
/// a terminal state. The actor translates this into a
/// `SessionCommand::JobCompleted` to keep the FIFO mailbox invariant.
///
/// Note: `PartialEq` is derived but not `Eq`; see `JobOutcome` for the
/// reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JobCompletionEvent {
    /// Identifier of the completed job.
    pub job_id: JobId,
    /// Terminal outcome of the job.
    pub outcome: JobOutcome,
}

/// Error kind for `JobManager` operations.
///
/// Marked `#[non_exhaustive]` so v0.4 distributed backends can add variants
/// (e.g. `AlreadyCompleted`, `PermissionDenied`, `Timeout`) without forcing
/// a SemVer-major bump on downstream `match` arms.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JobError {
    /// `JobManager` does not know about this `JobId` (typo, expired, or
    /// state lost across restart).
    #[error("unknown job: {0}")]
    UnknownJob(JobId),
    /// Backing store / queue / broker is unreachable.
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
}

/// Brain-facing contract for tracking async work.
///
/// Implementations live in `cogito-jobs` (v0.1 local) and
/// `cogito-jobs-distributed` (v0.4 Redis-backed). Submission is *not*
/// part of this trait — only async tool implementations submit jobs,
/// and they hold a reference to the concrete `LocalJobManager` type.
#[async_trait]
pub trait JobManager: Send + Sync {
    /// Query the current state of a job.
    async fn status(&self, job_id: JobId) -> Result<JobStatus, JobError>;

    /// Retrieve the terminal outcome. Errors if the job has not completed.
    async fn result(&self, job_id: JobId) -> Result<JobOutcome, JobError>;

    /// Best-effort cancellation. The job may already be terminal.
    async fn cancel(&self, job_id: JobId) -> Result<(), JobError>;

    /// Register a one-shot completion callback. When the job reaches a
    /// terminal state, the manager sends exactly one `JobCompletionEvent`
    /// on `sink` and drops the sender. If `sink` is closed (e.g., the
    /// actor died), the implementation may silently drop the event.
    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<(), JobError>;
}

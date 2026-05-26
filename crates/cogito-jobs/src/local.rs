//! In-memory `JobManager` implementation. Jobs run as `tokio::task`s; their
//! lifecycle is tracked in a `HashMap<JobId, JobLifecycle>` behind a
//! `parking_lot::Mutex`. See
//! `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md` §6
//! for the design rationale.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::job::{
    JobCompletionEvent, JobError, JobId, JobManager, JobOutcome, JobStatus,
};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// Local job manager. One instance per `Runtime`; cloned via `Arc`.
///
/// Jobs run as `tokio::task`s on the ambient runtime. Lifecycle state
/// (status, terminal outcome, registered completion sink, abort handle)
/// lives in an in-memory map; nothing is persisted, so a process
/// restart loses every running job.
pub struct LocalJobManager {
    jobs: Mutex<HashMap<JobId, JobLifecycle>>,
}

/// Per-job lifecycle record kept in the in-memory map.
struct JobLifecycle {
    status: JobStatus,
    outcome: Option<JobOutcome>,
    on_complete_sink: Option<mpsc::Sender<JobCompletionEvent>>,
    abort_handle: Option<AbortHandle>,
}

impl LocalJobManager {
    /// Construct an empty manager wrapped in an `Arc`.
    ///
    /// `submit` requires `self: &Arc<Self>` so the spawned task can hold
    /// a strong reference back to the manager for completion bookkeeping;
    /// returning an `Arc` here saves every caller from an explicit wrap.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            jobs: Mutex::new(HashMap::new()),
        })
    }

    /// Submit a future as an async job. Returns immediately with the new
    /// `JobId`; the future runs on the current Tokio runtime. When the
    /// future resolves, the outcome is stored and any registered
    /// `on_complete` sink fires.
    ///
    /// Not part of the `JobManager` trait: only async tool implementations
    /// submit jobs, and they hold an `Arc<LocalJobManager>` directly.
    pub fn submit<F>(self: &Arc<Self>, fut: F) -> JobId
    where
        F: Future<Output = JobOutcome> + Send + 'static,
    {
        let job_id = JobId::default();
        let this = Arc::clone(self);
        // Insert the lifecycle entry before spawning so that any caller
        // observing `status` immediately after `submit` sees `Running`
        // even if the spawned task has not been polled yet. The abort
        // handle is patched in once we have it.
        self.jobs.lock().insert(
            job_id,
            JobLifecycle {
                status: JobStatus::Running,
                outcome: None,
                on_complete_sink: None,
                abort_handle: None,
            },
        );
        let handle = tokio::spawn(async move {
            let outcome = fut.await;
            this.complete_internal(job_id, outcome).await;
        });
        // Race note: between the insert above and this patch, a racing
        // cancel(job_id) could observe `abort_handle: None` and skip the
        // `abort()` call. In that case the spawned task runs to its first
        // `await` and then attempts `complete_internal`, which observes the
        // already-`Cancelled` status and bails (see `complete_internal`'s
        // idempotency check). The user-visible behavior is identical: cancel
        // flips status to `Cancelled` and fires the sink with `Cancelled`,
        // and the spawned future's eventual outcome is discarded. The only
        // cost is a few extra microseconds of wall-clock work on the
        // about-to-be-orphaned task, which is acceptable.
        if let Some(entry) = self.jobs.lock().get_mut(&job_id) {
            entry.abort_handle = Some(handle.abort_handle());
        }
        job_id
    }

    /// Internal: called from the spawned task when the job future resolves.
    /// Transitions to terminal status and fires the registered sink.
    ///
    /// Idempotent: if `cancel` (or any other terminal transition) raced
    /// ahead, the existing terminal state is preserved and the sink is not
    /// re-fired. This makes it safe for the spawned task to complete its
    /// natural body even after a cancel; its outcome is simply discarded.
    async fn complete_internal(&self, job_id: JobId, outcome: JobOutcome) {
        let sink = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.get_mut(&job_id) else {
                tracing::warn!(%job_id, "complete_internal for unknown job; dropping");
                return;
            };
            // Idempotency: if cancel (or a prior completion) already moved
            // the job to a terminal state, do not overwrite the outcome and
            // do not re-fire the sink.
            if matches!(
                job.status,
                JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
            ) {
                return;
            }
            // `JobOutcome` is `#[non_exhaustive]`. The wildcard arm exists
            // as a forward-compat catch-all (e.g. future `TimedOut` /
            // `Preempted` variants land on `Failed` here); a `tracing::warn!`
            // surfaces the drift so telemetry catches it.
            let new_status = match &outcome {
                JobOutcome::Success { .. } => JobStatus::Completed,
                JobOutcome::Failed { .. } => JobStatus::Failed,
                JobOutcome::Cancelled => JobStatus::Cancelled,
                other => {
                    tracing::warn!(
                        ?other,
                        "unknown JobOutcome variant; mapping to JobStatus::Failed"
                    );
                    JobStatus::Failed
                }
            };
            job.status = new_status;
            job.outcome = Some(outcome.clone());
            job.abort_handle = None;
            job.on_complete_sink.take()
        };
        if let Some(sink) = sink {
            // Best-effort delivery: per the contract, a dropped receiver
            // is not a failure, so the send result is discarded.
            let _ = sink.send(JobCompletionEvent { job_id, outcome }).await;
        }
    }
}

#[async_trait]
impl JobManager for LocalJobManager {
    async fn status(&self, job_id: JobId) -> Result<JobStatus, JobError> {
        self.jobs
            .lock()
            .get(&job_id)
            .map(|j| j.status)
            .ok_or(JobError::UnknownJob(job_id))
    }

    async fn result(&self, job_id: JobId) -> Result<JobOutcome, JobError> {
        let jobs = self.jobs.lock();
        let Some(job) = jobs.get(&job_id) else {
            return Err(JobError::UnknownJob(job_id));
        };
        job.outcome.clone().ok_or_else(|| {
            // `JobError` currently has no dedicated "still running"
            // variant; `BackendUnavailable` is the closest match and the
            // contract suite (Task 2) only asserts `is_err()` here.
            JobError::BackendUnavailable(format!("job {job_id} has not yet completed"))
        })
    }

    // TODO(subprocess-cancel-orphan): abort_handle.abort() terminates the task
    // at its next .await — any subprocess the task spawned via tokio::process
    // is orphaned. Async tools that spawn OS processes (e.g., RunTestsTool)
    // must currently rely on the process's own SIGKILL handling. Proper fix:
    // add a per-job CancellationToken stored alongside abort_handle and have
    // cancel() signal it BEFORE calling abort, so the future has one yield
    // point to clean up child processes.
    async fn cancel(&self, job_id: JobId) -> Result<(), JobError> {
        let (abort, sink) = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.get_mut(&job_id) else {
                return Err(JobError::UnknownJob(job_id));
            };
            // Already terminal: no-op. A second cancel must not re-fire the
            // sink or clobber a `Completed` / `Failed` outcome.
            if matches!(
                job.status,
                JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
            ) {
                return Ok(());
            }
            job.status = JobStatus::Cancelled;
            job.outcome = Some(JobOutcome::Cancelled);
            let abort = job.abort_handle.take();
            let sink = job.on_complete_sink.take();
            (abort, sink)
        };
        if let Some(abort) = abort {
            abort.abort();
        }
        if let Some(sink) = sink {
            // Best-effort delivery, matching `complete_internal`.
            let _ = sink
                .send(JobCompletionEvent {
                    job_id,
                    outcome: JobOutcome::Cancelled,
                })
                .await;
        }
        Ok(())
    }

    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<(), JobError> {
        let already_terminal = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.get_mut(&job_id) else {
                return Err(JobError::UnknownJob(job_id));
            };
            let is_terminal = matches!(
                job.status,
                JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
            );
            if is_terminal {
                job.outcome.clone()
            } else {
                job.on_complete_sink = Some(sink.clone());
                None
            }
        };
        if let Some(outcome) = already_terminal {
            let _ = sink.send(JobCompletionEvent { job_id, outcome }).await;
        }
        Ok(())
    }
}

//! Test-only `JobManager` for chaos test `PausedOnJob` scenarios. Honors the
//! two contracts from Sprint 3 spec §8.4:
//!
//! 1. If a job has already completed by the time `on_complete` is called,
//!    the sink fires immediately with the stored outcome.
//! 2. If the job has not yet completed, the sink is stored and fires
//!    exactly once when `complete()` is invoked.
//!
//! Sprint 3 itself does not wire `JobManager` into the Runtime (v0.1 has no
//! injection point), but this mock is needed for the chaos test
//! infrastructure and for Sprint 4 when `JobManager` is wired into the
//! actor's `PausedOnJob` path.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::job::{
    JobCompletionEvent, JobError, JobId, JobManager, JobOutcome, JobStatus, LocalJobSubmitter,
};
use futures::future::BoxFuture;
use tokio::sync::{Mutex, mpsc};

/// Mock implementation of [`JobManager`] for tests.
///
/// Lifecycle: caller `register`s a job (transitions it to `Running`), then
/// at some point calls `complete(job_id, outcome)` which transitions the
/// job to a terminal status and fires the registered sink (if any).
#[derive(Default, Clone)]
pub struct MockJobManager {
    jobs: Arc<Mutex<HashMap<JobId, JobLifecycle>>>,
}

/// Per-job state tracked by the mock manager.
struct JobLifecycle {
    /// Current lifecycle state. Starts as `Running` after `register`.
    status: JobStatus,
    /// Terminal outcome; populated by `complete`.
    outcome: Option<JobOutcome>,
    /// Sink registered via `on_complete` while the job was non-terminal.
    on_complete_sink: Option<mpsc::Sender<JobCompletionEvent>>,
}

impl MockJobManager {
    /// Construct an empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new job in `Running` state. Tests call this before
    /// the job is referenced via `on_complete` or `complete`.
    pub async fn register(&self, job_id: JobId) {
        self.jobs.lock().await.insert(
            job_id,
            JobLifecycle {
                status: JobStatus::Running,
                outcome: None,
                on_complete_sink: None,
            },
        );
    }

    /// Test-side API: mark a registered job as terminal and fire any
    /// registered `on_complete` sink. Subsequent `status`/`result` queries
    /// will reflect the terminal state.
    ///
    /// The `outcome` variant determines the resulting `JobStatus`:
    /// `Success` -> `Completed`, `Failed` -> `Failed`, `Cancelled` ->
    /// `Cancelled`. Unknown future variants default to `Failed` so the
    /// test harness still reaches a terminal status without compile
    /// breakage when the protocol evolves (`JobOutcome` is
    /// `#[non_exhaustive]`).
    pub async fn complete(&self, job_id: JobId, outcome: JobOutcome) {
        let mut jobs = self.jobs.lock().await;
        let Some(job) = jobs.get_mut(&job_id) else {
            return;
        };
        // `JobOutcome` is `#[non_exhaustive]`. The explicit `Failed` arm
        // and the wildcard arm coincide today, but we want them spelled
        // out separately: the wildcard exists purely as a forward-compat
        // fallback (e.g. future `TimedOut` / `Preempted` variants land on
        // `Failed` here), and `match_same_arms` would have us collapse
        // the meaningful `Failed` arm into it.
        #[allow(clippy::match_same_arms)]
        let new_status = match outcome {
            JobOutcome::Success { .. } => JobStatus::Completed,
            JobOutcome::Failed { .. } => JobStatus::Failed,
            JobOutcome::Cancelled => JobStatus::Cancelled,
            _ => JobStatus::Failed,
        };
        job.status = new_status;
        job.outcome = Some(outcome.clone());
        if let Some(sink) = job.on_complete_sink.take() {
            // Best-effort send; if the sink is dropped on the caller
            // side, silently swallow the error (the spec allows it).
            let _ = sink.send(JobCompletionEvent { job_id, outcome }).await;
        }
    }
}

#[async_trait]
impl JobManager for MockJobManager {
    async fn status(&self, job_id: JobId) -> Result<JobStatus, JobError> {
        let jobs = self.jobs.lock().await;
        jobs.get(&job_id)
            .map(|j| j.status)
            .ok_or(JobError::UnknownJob(job_id))
    }

    async fn result(&self, job_id: JobId) -> Result<JobOutcome, JobError> {
        let jobs = self.jobs.lock().await;
        match jobs.get(&job_id) {
            Some(job) => job
                .outcome
                .clone()
                .ok_or(JobError::BackendUnavailable(format!(
                    "job {job_id} has not yet completed"
                ))),
            None => Err(JobError::UnknownJob(job_id)),
        }
    }

    async fn cancel(&self, job_id: JobId) -> Result<(), JobError> {
        // Best-effort: only register cancellation; do NOT fire on_complete
        // sink from here. The test driver explicitly calls `complete` if
        // it wants the sink to fire with a Cancelled outcome.
        let mut jobs = self.jobs.lock().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.status = JobStatus::Cancelled;
            Ok(())
        } else {
            Err(JobError::UnknownJob(job_id))
        }
    }

    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<(), JobError> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs.get_mut(&job_id).ok_or(JobError::UnknownJob(job_id))?;

        // Contract 1: job already terminal -> fire sink immediately.
        // Contract 2: job still in-flight -> store sink; complete() fires
        // it later.
        let is_terminal = matches!(
            job.status,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
        );
        if is_terminal {
            if let Some(outcome) = job.outcome.clone() {
                // Best-effort send; if the sink is dropped on the caller
                // side, silently swallow the error (the spec allows it).
                let _ = sink.send(JobCompletionEvent { job_id, outcome }).await;
            }
            // If outcome is None despite terminal status, that is a
            // self-inconsistent state (test driver bug); silently ignore.
        } else {
            job.on_complete_sink = Some(sink);
        }
        Ok(())
    }
}

#[async_trait]
impl LocalJobSubmitter for MockJobManager {
    async fn submit_boxed(self: Arc<Self>, fut: BoxFuture<'static, JobOutcome>) -> JobId {
        // Spawn-and-complete shim: MockJobManager is normally driven from
        // test code via `register` + `complete`, but tools that hold
        // `Arc<dyn LocalJobSubmitter>` call `submit_boxed`. We honor it
        // by registering a new job, spawning the future, and calling
        // `complete` ourselves when it resolves. Tests retain the
        // explicit `register` / `complete` API for cases that need
        // fine-grained timing control.
        let job_id = JobId::default();
        self.register(job_id).await;
        let mgr = Arc::clone(&self);
        tokio::spawn(async move {
            let outcome = fut.await;
            mgr.complete(job_id, outcome).await;
        });
        job_id
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cogito_protocol::test_support::contract_job_manager::{
        JobManagerHarness, run_contract_suite,
    };

    /// Harness that drives [`MockJobManager`] through the shared
    /// `JobManager` contract suite. `arm` registers a fresh job and
    /// stashes the outcome that `fire` will later hand to
    /// [`MockJobManager::complete`].
    struct MockHarness {
        manager: Arc<MockJobManager>,
        pending: Mutex<HashMap<JobId, JobOutcome>>,
    }

    #[async_trait]
    impl JobManagerHarness for MockHarness {
        type Manager = MockJobManager;

        fn new() -> Arc<Self> {
            Arc::new(Self {
                manager: Arc::new(MockJobManager::new()),
                pending: Mutex::new(HashMap::new()),
            })
        }

        fn manager(&self) -> Arc<Self::Manager> {
            Arc::clone(&self.manager)
        }

        async fn arm(&self, outcome: JobOutcome) -> JobId {
            let job_id = JobId::default();
            self.manager.register(job_id).await;
            self.pending.lock().await.insert(job_id, outcome);
            job_id
        }

        async fn fire(&self, job_id: JobId) {
            let outcome = self
                .pending
                .lock()
                .await
                .remove(&job_id)
                .expect("fire called on an un-armed job");
            self.manager.complete(job_id, outcome).await;
        }
    }

    #[tokio::test]
    async fn mock_job_manager_satisfies_job_manager_contract() {
        run_contract_suite::<MockHarness>().await;
    }
}

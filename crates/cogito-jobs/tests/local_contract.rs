#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Drives the shared `JobManager` contract suite against `LocalJobManager`.
//! The harness arms a job by submitting a future gated on a oneshot
//! channel; `fire` releases the gate so the job resolves with the
//! caller-chosen outcome.

use async_trait::async_trait;
use cogito_jobs::LocalJobManager;
use cogito_protocol::job::{JobId, JobManager, JobOutcome, JobStatus};
use cogito_protocol::test_support::contract_job_manager::{JobManagerHarness, run_contract_suite};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

struct LocalHarness {
    mgr: Arc<LocalJobManager>,
    armed: Mutex<Vec<(JobId, tokio::sync::oneshot::Sender<()>)>>,
}

#[async_trait]
impl JobManagerHarness for LocalHarness {
    type Manager = LocalJobManager;

    fn new() -> Arc<Self> {
        Arc::new(Self {
            mgr: LocalJobManager::new(),
            armed: Mutex::new(Vec::new()),
        })
    }

    fn manager(&self) -> Arc<Self::Manager> {
        Arc::clone(&self.mgr)
    }

    async fn arm(&self, outcome: JobOutcome) -> JobId {
        let outcome_clone = outcome.clone();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let job_id = self.mgr.submit(async move {
            let _ = gate_rx.await;
            outcome_clone
        });
        self.armed.lock().await.push((job_id, gate_tx));
        job_id
    }

    async fn fire(&self, job_id: JobId) {
        let mut guard = self.armed.lock().await;
        if let Some(pos) = guard.iter().position(|(id, _)| *id == job_id) {
            let (_, tx) = guard.remove(pos);
            let _ = tx.send(());
        }
        drop(guard);
        // The spawned task needs scheduler time after the gate fires to
        // observe the outcome and update the lifecycle map. The contract
        // suite asserts terminal state immediately after `fire` returns,
        // so block here until status flips (or the bounded timeout
        // elapses, in which case the contract assertion will surface
        // the real failure).
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match self.mgr.status(job_id).await {
                Ok(s) if !matches!(s, JobStatus::Pending | JobStatus::Running) => break,
                _ if std::time::Instant::now() >= deadline => break,
                _ => tokio::task::yield_now().await,
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_job_manager_satisfies_contract() {
    run_contract_suite::<LocalHarness>().await;
}

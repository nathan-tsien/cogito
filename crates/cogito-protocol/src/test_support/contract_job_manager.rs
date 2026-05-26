//! `JobManager` contract suite.
//!
//! Every [`crate::job::JobManager`] implementation in the workspace must
//! pass [`run_contract_suite`]. The suite covers the two
//! `on_complete`-firing contracts from the original Sprint 3 mock plus
//! the status/result lifecycle and the silent-drop-of-receiver
//! behaviour.
//!
//! Implementations supply a [`JobManagerHarness`] that knows how to
//! - construct a fresh manager,
//! - arm a job that will eventually deliver a caller-chosen
//!   [`JobOutcome`], and
//! - fire that completion at a moment the test controls.
//!
//! The harness abstraction keeps the contract free of any submission
//! API: today `MockJobManager` exposes `register`/`complete`; tomorrow
//! `LocalJobManager` will submit a future that resolves to the same
//! outcome. The contract only cares about post-arming observable
//! behaviour.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::job::{JobError, JobId, JobManager, JobOutcome, JobStatus};
use crate::tool::ToolResult;

/// Test driver that wraps a concrete [`JobManager`] implementation.
///
/// The harness owns whatever state is needed to inject a deterministic
/// terminal outcome at a precise moment, so the contract tests can
/// reliably observe both the "completed before `on_complete`" and the
/// "completed after `on_complete`" paths.
#[async_trait]
pub trait JobManagerHarness: Send + Sync + Sized + 'static {
    /// Concrete `JobManager` type under test.
    type Manager: JobManager + 'static;

    /// Build a fresh harness. Each contract test calls this so cases do
    /// not share mutable state.
    fn new() -> Arc<Self>;

    /// Borrow the manager exposed to the suite. The contract only
    /// exercises trait methods, so this is the only access path the
    /// tests touch.
    fn manager(&self) -> Arc<Self::Manager>;

    /// Register a job with the manager and remember the outcome that
    /// [`Self::fire`] should later deliver. The job must be observable
    /// via the trait API (e.g. `status` returns `Running`) immediately
    /// after this call.
    async fn arm(&self, outcome: JobOutcome) -> JobId;

    /// Drive the previously-armed job to its terminal state, firing any
    /// registered `on_complete` sink with the outcome supplied to
    /// [`Self::arm`].
    async fn fire(&self, job_id: JobId);
}

/// Run the entire `JobManager` contract suite against `H`.
pub async fn run_contract_suite<H: JobManagerHarness>() {
    contract_fires_immediately::<H>().await;
    contract_fires_after_completion::<H>().await;
    contract_status_and_result_lifecycle::<H>().await;
    contract_sink_dropped_is_silent::<H>().await;
}

/// `on_complete` must fire immediately if the job has already reached a
/// terminal state by the time the callback is registered.
pub async fn contract_fires_immediately<H: JobManagerHarness>() {
    let harness = H::new();
    let mgr = harness.manager();
    let job_id = harness
        .arm(JobOutcome::Success {
            result: ToolResult::text("ok"),
        })
        .await;
    harness.fire(job_id).await;

    let (tx, mut rx) = mpsc::channel(1);
    mgr.on_complete(job_id, tx)
        .await
        .expect("on_complete should succeed for known job");
    let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("sink should fire within timeout")
        .expect("sender should not be dropped before delivering event");
    assert_eq!(evt.job_id, job_id);
    assert!(matches!(evt.outcome, JobOutcome::Success { .. }));
}

/// `on_complete` registered before completion must fire exactly once
/// when the job reaches a terminal state.
pub async fn contract_fires_after_completion<H: JobManagerHarness>() {
    let harness = H::new();
    let mgr = harness.manager();
    let job_id = harness.arm(JobOutcome::Cancelled).await;

    let (tx, mut rx) = mpsc::channel(1);
    mgr.on_complete(job_id, tx)
        .await
        .expect("on_complete should succeed for known job");
    assert!(
        rx.try_recv().is_err(),
        "no event should arrive before the job is fired"
    );

    harness.fire(job_id).await;
    let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("sink should fire after completion within timeout")
        .expect("sender should not be dropped before delivering event");
    assert_eq!(evt.job_id, job_id);
    assert!(matches!(evt.outcome, JobOutcome::Cancelled));
}

/// `status` and `result` must reflect the lifecycle: unknown jobs error
/// with [`JobError::UnknownJob`], running jobs report `Running` and
/// refuse `result`, and terminal jobs surface the stored outcome.
pub async fn contract_status_and_result_lifecycle<H: JobManagerHarness>() {
    let harness = H::new();
    let mgr = harness.manager();

    let unknown = JobId::default();
    assert!(matches!(
        mgr.status(unknown).await,
        Err(JobError::UnknownJob(_))
    ));
    assert!(matches!(
        mgr.result(unknown).await,
        Err(JobError::UnknownJob(_))
    ));

    let job_id = harness
        .arm(JobOutcome::Failed {
            message: "boom".into(),
        })
        .await;
    assert!(matches!(
        mgr.status(job_id)
            .await
            .expect("status should succeed for known job"),
        JobStatus::Running
    ));
    // Implementations may surface "not yet complete" through any
    // non-`UnknownJob` error variant (`JobError` is `#[non_exhaustive]`);
    // the contract only forbids returning a phantom outcome.
    assert!(
        mgr.result(job_id).await.is_err(),
        "result should error while the job is still running"
    );

    harness.fire(job_id).await;
    assert!(matches!(
        mgr.status(job_id)
            .await
            .expect("status should succeed after completion"),
        JobStatus::Failed
    ));
    assert!(matches!(
        mgr.result(job_id)
            .await
            .expect("result should succeed after completion"),
        JobOutcome::Failed { .. }
    ));
}

/// Dropping the receiver before completion must not panic the manager
/// nor surface a hard error: the spec allows silently swallowing the
/// undeliverable event.
pub async fn contract_sink_dropped_is_silent<H: JobManagerHarness>() {
    let harness = H::new();
    let mgr = harness.manager();
    let job_id = harness
        .arm(JobOutcome::Success {
            result: ToolResult::text("ok"),
        })
        .await;

    let (tx, rx) = mpsc::channel(1);
    mgr.on_complete(job_id, tx)
        .await
        .expect("on_complete should succeed for known job");
    drop(rx);

    // No panic, no error: firing is best-effort once the sink has gone
    // away. The manager must still transition the job to terminal so
    // subsequent `status` / `result` calls succeed.
    harness.fire(job_id).await;
    assert!(
        !matches!(
            mgr.status(job_id)
                .await
                .expect("status should succeed after completion"),
            JobStatus::Pending | JobStatus::Running
        ),
        "job should be terminal even when the sink was dropped"
    );
    assert!(
        mgr.result(job_id).await.is_ok(),
        "result should be retrievable after completion regardless of sink state"
    );
}

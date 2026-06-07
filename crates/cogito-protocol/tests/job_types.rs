//! Tests for JobManager-adjacent value types. The trait itself is exercised
//! via contract tests in concrete implementor crates (cogito-jobs).

use cogito_protocol::job::{JobCompletionEvent, JobId, JobOutcome, JobStatus};
use cogito_protocol::tool::ToolResult;

#[test]
fn job_id_default_is_unique() {
    let a = JobId::default();
    let b = JobId::default();
    assert_ne!(a, b, "two default-constructed JobIds must collide-resist");
}

#[test]
fn job_status_serde_covers_all_variants() -> serde_json::Result<()> {
    for status in [
        JobStatus::Pending,
        JobStatus::Running,
        JobStatus::AwaitingInput,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ] {
        let json = serde_json::to_string(&status)?;
        let back: JobStatus = serde_json::from_str(&json)?;
        assert_eq!(status, back);
    }
    Ok(())
}

#[test]
fn job_status_awaiting_input_wire_name() -> serde_json::Result<()> {
    // ADR-0039 Decision 5: observation-only state a HITL-capable JobManager
    // MAY report for a job parked on a human. The wire name must be the
    // snake_case rendering so dashboards/operators read a stable token.
    let json = serde_json::to_string(&JobStatus::AwaitingInput)?;
    assert_eq!(json, "\"awaiting_input\"");
    let back: JobStatus = serde_json::from_str(&json)?;
    assert_eq!(JobStatus::AwaitingInput, back);
    Ok(())
}

#[test]
fn job_completion_event_carries_job_id_and_outcome() -> serde_json::Result<()> {
    let event = JobCompletionEvent {
        job_id: JobId::default(),
        outcome: JobOutcome::Cancelled,
    };
    let json = serde_json::to_string(&event)?;
    let back: JobCompletionEvent = serde_json::from_str(&json)?;
    assert_eq!(event, back);
    Ok(())
}

#[test]
fn job_outcome_success_carries_tool_result() -> serde_json::Result<()> {
    let outcome = JobOutcome::Success {
        result: ToolResult::text("done"),
    };
    let json = serde_json::to_string(&outcome)?;
    let back: JobOutcome = serde_json::from_str(&json)?;
    assert_eq!(outcome, back);
    Ok(())
}

#[test]
fn job_outcome_failed_carries_message() -> serde_json::Result<()> {
    let outcome = JobOutcome::Failed {
        message: "whisper API 503".into(),
    };
    let json = serde_json::to_string(&outcome)?;
    let back: JobOutcome = serde_json::from_str(&json)?;
    assert_eq!(outcome, back);
    Ok(())
}

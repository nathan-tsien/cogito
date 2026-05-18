//! `TurnOutcome` and `TurnFailureReason` serde stability.

use cogito_protocol::job::JobId;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};

#[test]
fn completed_outcome_roundtrips() -> serde_json::Result<()> {
    let outcome = TurnOutcome::Completed;
    let json = serde_json::to_string(&outcome)?;
    let back: TurnOutcome = serde_json::from_str(&json)?;
    assert_eq!(outcome, back);
    Ok(())
}

#[test]
fn paused_carries_job_id() -> serde_json::Result<()> {
    let outcome = TurnOutcome::Paused {
        job_id: JobId::default(),
    };
    let json = serde_json::to_string(&outcome)?;
    let back: TurnOutcome = serde_json::from_str(&json)?;
    assert_eq!(outcome, back);
    Ok(())
}

#[test]
fn failed_carries_reason_and_event_id() -> serde_json::Result<()> {
    let outcome = TurnOutcome::Failed {
        reason: TurnFailureReason::TurnTimedOut,
        recorded_event_id: "event-42".into(),
    };
    let json = serde_json::to_string(&outcome)?;
    let back: TurnOutcome = serde_json::from_str(&json)?;
    assert_eq!(outcome, back);
    Ok(())
}

#[test]
fn all_failure_reasons_roundtrip() -> serde_json::Result<()> {
    let reasons = [
        TurnFailureReason::StoreUnavailable,
        TurnFailureReason::ModelGatewayFailed {
            message: "503".into(),
        },
        TurnFailureReason::TurnPanicked {
            location: "stream demux".into(),
        },
        TurnFailureReason::TurnTimedOut,
        TurnFailureReason::HookRejected {
            hook_name: "sensitive-content".into(),
            message: "regex matched".into(),
        },
    ];
    for r in reasons {
        let json = serde_json::to_string(&r)?;
        let back: TurnFailureReason = serde_json::from_str(&json)?;
        assert_eq!(r, back);
    }
    Ok(())
}

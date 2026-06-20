//! H03 Resume Coordinator — pure function from event log to resume decision.
//!
//! Sprint 3 implements the full decision table per spec §4–§5.
//! Pure function: same input → same output, no I/O, no clock, no random.
//! The actor calls `replay()` on startup and uses the result to bootstrap
//! either the FSM (via `TurnEntry`) or its own `InFlight` state.

use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::ids::TurnId;
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::tool::ToolResult;

/// Output of H03 Resume Coordinator. Pure projection from the event log.
/// Never persisted (see spec §6 落盘语义).
#[derive(Debug, Clone, PartialEq)]
pub struct ResumeDecision {
    /// What state to resume into.
    pub point: ResumePoint,
    /// `seq` of the last event in the log when this decision was computed.
    /// `None` iff the log is empty (which also implies `point == FreshTurn`).
    /// Actor initializes its event-seq generator to `last_event_seq + 1`.
    pub last_event_seq: Option<u64>,
}

/// Resume entry point. Six variants covering every valid log shape.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ResumePoint {
    /// Empty log, or last turn ended in `TurnCompleted` / `TurnFailed`.
    /// Actor idles until the next caller `submit` (`SessionCommand::Trigger`).
    FreshTurn,

    /// In-flight turn where the most recent model call did not complete
    /// (no `ModelCallCompleted` after the latest `ModelCallStarted`).
    /// FSM enters `Init`; H04 rebuilds prompt from the event log; one
    /// model call gets re-billed.
    RestartCurrentTurn {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
    },

    /// Most recent `ModelCallCompleted` is the latest event in the turn
    /// AND no `ToolUseRecorded` follows. Actor crashed between writing
    /// the sealing event and writing `TurnCompleted`. FSM enters
    /// `ModelCompleted` with output rebuilt from events; fast-paths to
    /// `Completed` without re-calling the model.
    ResumeFromModelCompleted {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
        /// Model output rebuilt from the event log.
        rebuilt_output: ModelOutput,
    },

    /// Tool dispatch round in progress. May have 0+ completed results.
    /// FSM enters `ToolDispatching`. `enter_turn` re-runs H07 on `pending`
    /// to re-validate against current schemas, and triggers H10+H05 to
    /// rebuild the tool surface.
    ResumeFromToolDispatching {
        /// The turn that was in progress when the crash occurred.
        turn_id: TurnId,
        /// `ToolUseRecorded` since the latest `ModelCallCompleted` with no
        /// matching `ToolResultRecorded`. Order preserved from the log.
        pending: Vec<ResumePendingCall>,
        /// `(call_id, ToolResult)` pairs already in the log.
        completed: Vec<(String, ToolResult)>,
    },

    /// Turn paused on an async job. `TurnPaused` is the latest event;
    /// no `JobCompletedRecorded { job_id }` follows. Actor enters
    /// `InFlight::PausedOnJob` and re-registers `on_complete`.
    ResumePausedJob {
        /// The turn that was paused.
        turn_id: TurnId,
        /// The async job this turn is waiting on.
        job_id: JobId,
    },

    /// Async job completed but Brain didn't consume the
    /// `JobCompletedRecorded` event before the crash. FSM enters
    /// `ToolDispatching` with the just-completed result injected as the
    /// last entry of `completed_before_pause` + `call_id` resolved.
    ResumeAfterJobCompletion {
        /// The turn that was paused.
        turn_id: TurnId,
        /// The async job that completed.
        job_id: JobId,
        /// Outcome of the completed job.
        outcome: JobOutcome,
        /// Resolved via `lookup_call_id_in_events` — the most recent
        /// `JobSubmitted { job_id, .. }` carries the originating
        /// `call_id`. See the Sprint 8 spec
        /// `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`
        /// §9.1.
        call_id: String,
        /// Tool calls dispatched and completed before the pause.
        completed_before_pause: Vec<(String, ToolResult)>,
        /// Tool calls declared by the model but not yet dispatched at
        /// pause time. (Sprint 3 always empty; Sprint 4 may be non-empty.)
        pending_after_pause: Vec<ResumePendingCall>,
    },
}

/// Raw tool-call triple recovered from a `ToolUseRecorded` event.
/// Pre-validation — `enter_turn` re-runs through H07 before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePendingCall {
    /// The call ID from the original tool-use event.
    pub call_id: String,
    /// The tool name from the original tool-use event.
    pub tool_name: String,
    /// The raw arguments from the original tool-use event.
    pub args: serde_json::Value,
}

/// Errors from `replay`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    /// Event log contradicts itself (e.g., `JobCompletedRecorded` with no
    /// matching prior `TurnPaused`; nested `TurnStarted` without
    /// terminator).
    #[error("malformed event log: {0}")]
    Malformed(String),
    /// Event log was written by a newer schema version than this build supports.
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    /// A tool referenced by a recovered call is no longer registered.
    /// (Sprint 3 returns this from `enter_turn` re-validation, not from
    /// `replay()` itself; reserved here for completeness.)
    #[error("tool `{tool_name}` (call_id `{call_id}`) no longer registered")]
    ToolUnavailable {
        /// The call ID of the unavailable tool invocation.
        call_id: String,
        /// The name of the tool that is no longer registered.
        tool_name: String,
    },
    /// Persisted tool args fail current schema validation.
    #[error("tool `{tool_name}` schema rejects persisted args: {reason}")]
    ToolSchemaDrift {
        /// The name of the tool whose schema drifted.
        tool_name: String,
        /// Description of why the persisted args fail validation.
        reason: String,
    },
}

/// Replays the event log and returns a `ResumeDecision` describing the
/// exact state to resume into. Pure function: same input always produces
/// the same output; no I/O, no clock, no randomness.
///
/// Implements the 9-row decision table from spec §5 (Sprint 3). The
/// paused-job arms derive `call_id` via [`lookup_call_id_in_events`]
/// against the Sprint 8 `JobSubmitted` event; see the Sprint 8 spec
/// `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md` §9.1.
///
/// # Errors
///
/// Returns `ResumeError::UnsupportedSchema` if any event has a
/// `schema_version` newer than this build supports, or
/// `ResumeError::Malformed` if the log is structurally inconsistent
/// (e.g., `JobCompletedRecorded` with no preceding `TurnPaused`, or
/// `TurnPaused { job_id }` without a preceding
/// `JobSubmitted { job_id, .. }`).
pub fn replay(events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    // 1. Schema check (must come first).
    if let Some(e) = events.iter().find(|e| e.schema_version > SCHEMA_VERSION) {
        return Err(ResumeError::UnsupportedSchema(e.schema_version));
    }

    let last_event_seq = events.last().map(|e| e.seq);

    if events.is_empty() {
        return Ok(ResumeDecision {
            point: ResumePoint::FreshTurn,
            last_event_seq,
        });
    }

    // 2. Detect malformed: JobCompletedRecorded without preceding TurnPaused
    //    for the same job_id.
    if let Some(last) = events.last() {
        if let EventPayload::JobCompletedRecorded { job_id, .. } = &last.payload {
            let paused_before = events[..events.len() - 1].iter().rev().any(
                |e| matches!(&e.payload, EventPayload::TurnPaused { job_id: jid } if jid == job_id),
            );
            if !paused_before {
                return Err(ResumeError::Malformed(format!(
                    "JobCompletedRecorded for job_id={job_id:?} with no preceding TurnPaused"
                )));
            }
        }
    }

    // 3. Detect malformed: nested TurnStarted without intervening
    //    TurnCompleted/TurnFailed.
    check_no_nested_turn_started(events)?;

    // 4. Find the latest turn-boundary event.
    let boundary_idx = events
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, e)| match &e.payload {
            EventPayload::TurnStarted { .. }
            | EventPayload::TurnCompleted { .. }
            | EventPayload::TurnFailed { .. }
            | EventPayload::TurnPaused { .. } => Some(i),
            _ => None,
        });

    let Some(boundary_idx) = boundary_idx else {
        // Only SessionStarted (or other pre-turn events) — FreshTurn
        return Ok(ResumeDecision {
            point: ResumePoint::FreshTurn,
            last_event_seq,
        });
    };

    let boundary = &events[boundary_idx];

    match &boundary.payload {
        EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. } => {
            Ok(ResumeDecision {
                point: ResumePoint::FreshTurn,
                last_event_seq,
            })
        }
        EventPayload::TurnPaused { job_id } => {
            resume_from_turn_paused(events, boundary, boundary_idx, *job_id, last_event_seq)
        }
        EventPayload::TurnStarted { .. } => {
            let turn_id = boundary
                .turn_id
                .ok_or_else(|| ResumeError::Malformed("TurnStarted without turn_id".into()))?;
            let turn_slice = &events[boundary_idx..];
            resume_from_turn_started(turn_id, turn_slice, last_event_seq)
        }
        // Unreachable in practice: boundary_idx was found via the matched-set above.
        // Defensive path to avoid clippy::unreachable on the match arm.
        _ => Err(ResumeError::Malformed(format!(
            "unexpected boundary event payload: {:?}",
            boundary.payload
        ))),
    }
}

/// Checks that no `TurnStarted` event is nested inside another open turn (i.e.,
/// there is no second `TurnStarted` before the first turn's `TurnCompleted` or
/// `TurnFailed`). `TurnPaused` does NOT close a turn.
///
/// # Errors
///
/// Returns `ResumeError::Malformed` when a nested `TurnStarted` is detected.
fn check_no_nested_turn_started(events: &[ConversationEvent]) -> Result<(), ResumeError> {
    let mut in_turn = false;
    for e in events {
        match &e.payload {
            EventPayload::TurnStarted { .. } => {
                if in_turn {
                    return Err(ResumeError::Malformed(
                        "nested TurnStarted without intervening TurnCompleted/TurnFailed".into(),
                    ));
                }
                in_turn = true;
            }
            EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. } => {
                in_turn = false;
            }
            // TurnPaused does NOT close the turn (paused-then-resumed continues the
            // same turn).
            _ => {}
        }
    }
    Ok(())
}

/// Handles the `TurnPaused` boundary case: classifies into either
/// `ResumePausedJob` (no `JobCompletedRecorded` for this `job_id` yet) or
/// `ResumeAfterJobCompletion` (the job finished but the actor crashed
/// before consuming the result). `call_id` is derived via
/// [`lookup_call_id_in_events`]; see Sprint 8 spec §9.1.
fn resume_from_turn_paused(
    events: &[ConversationEvent],
    boundary: &ConversationEvent,
    boundary_idx: usize,
    job_id: JobId,
    last_event_seq: Option<u64>,
) -> Result<ResumeDecision, ResumeError> {
    let turn_id = boundary
        .turn_id
        .ok_or_else(|| ResumeError::Malformed("TurnPaused without turn_id".into()))?;
    let job_done = events[boundary_idx + 1..]
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::JobCompletedRecorded {
                job_id: jid,
                outcome,
            } if *jid == job_id => Some(outcome.clone()),
            _ => None,
        });
    match job_done {
        None => Ok(ResumeDecision {
            point: ResumePoint::ResumePausedJob { turn_id, job_id },
            last_event_seq,
        }),
        Some(outcome) => {
            let call_id = lookup_call_id_in_events(events, job_id).ok_or_else(|| {
                ResumeError::Malformed(format!(
                    "TurnPaused for job {job_id:?} has no preceding JobSubmitted"
                ))
            })?;
            let (completed_before_pause, pending_after_pause) =
                collect_paused_call_context(events, boundary_idx);
            Ok(ResumeDecision {
                point: ResumePoint::ResumeAfterJobCompletion {
                    turn_id,
                    job_id,
                    outcome,
                    call_id,
                    completed_before_pause,
                    pending_after_pause,
                },
                last_event_seq,
            })
        }
    }
}

/// Handles the `TurnStarted` boundary case: classifies the in-progress turn
/// into `RestartCurrentTurn`, `ResumeFromModelCompleted`, or
/// `ResumeFromToolDispatching` based on the model-call and tool-dispatch
/// events found in `turn_slice` (events from `TurnStarted` onward).
fn resume_from_turn_started(
    turn_id: TurnId,
    turn_slice: &[ConversationEvent],
    last_event_seq: Option<u64>,
) -> Result<ResumeDecision, ResumeError> {
    let model_started_pos = turn_slice
        .iter()
        .rposition(|e| matches!(e.payload, EventPayload::ModelCallStarted { .. }));
    let model_completed_pos = turn_slice
        .iter()
        .rposition(|e| matches!(e.payload, EventPayload::ModelCallCompleted { .. }));

    match (model_started_pos, model_completed_pos) {
        (None, _) | (Some(_), None) => Ok(ResumeDecision {
            point: ResumePoint::RestartCurrentTurn { turn_id },
            last_event_seq,
        }),
        (Some(started), Some(completed)) if started > completed => Ok(ResumeDecision {
            point: ResumePoint::RestartCurrentTurn { turn_id },
            last_event_seq,
        }),
        (Some(started), Some(completed)) => {
            // H06 stream_demux writes `ToolUseRecorded` during stream consumption
            // (when each `ToolUseCompleted` model event arrives) and writes
            // `ModelCallCompleted` only at the final `MessageCompleted` event.
            // So in the log: `ToolUseRecorded` events appear BETWEEN
            // `ModelCallStarted` and `ModelCallCompleted`, while
            // `ToolResultRecorded` events appear AFTER `ModelCallCompleted`
            // (written by the actor when each dispatched tool returns).
            //
            // Scan the two regions separately:
            let within_call = &turn_slice[started + 1..=completed];
            let after_mcc = &turn_slice[completed + 1..];

            let mut tool_results: Vec<(String, ToolResult)> = Vec::new();
            let mut completed_ids: Vec<String> = Vec::new();

            for e in after_mcc {
                if let EventPayload::ToolResultRecorded { call_id, result } = &e.payload {
                    tool_results.push((call_id.clone(), result.clone()));
                    completed_ids.push(call_id.clone());
                }
            }

            let mut pending: Vec<ResumePendingCall> = Vec::new();
            for e in within_call {
                if let EventPayload::ToolUseRecorded {
                    call_id,
                    tool_name,
                    args,
                    ..
                } = &e.payload
                {
                    if !completed_ids.iter().any(|id| id == call_id) {
                        pending.push(ResumePendingCall {
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                            args: args.clone(),
                        });
                    }
                }
            }

            if pending.is_empty() && tool_results.is_empty() {
                let rebuilt_output = rebuild_model_output(&turn_slice[started..=completed])?;
                // Spec §4.2: ResumeFromModelCompleted requires non-ToolUse stop_reason.
                // ToolUse with zero ToolUseRecorded means the model promised tools but
                // wrote none — malformed log.
                if matches!(
                    rebuilt_output.stop_reason,
                    cogito_protocol::gateway::StopReason::ToolUse
                ) {
                    return Err(ResumeError::Malformed(
                        "ModelCallCompleted has stop_reason=ToolUse but no ToolUseRecorded follows"
                            .into(),
                    ));
                }
                Ok(ResumeDecision {
                    point: ResumePoint::ResumeFromModelCompleted {
                        turn_id,
                        rebuilt_output,
                    },
                    last_event_seq,
                })
            } else {
                Ok(ResumeDecision {
                    point: ResumePoint::ResumeFromToolDispatching {
                        turn_id,
                        pending,
                        completed: tool_results,
                    },
                    last_event_seq,
                })
            }
        }
    }
}

/// Rebuilds a `ModelOutput` from the event sub-slice between
/// `ModelCallStarted` and `ModelCallCompleted` (inclusive).
fn rebuild_model_output(slice: &[ConversationEvent]) -> Result<ModelOutput, ResumeError> {
    use cogito_protocol::gateway::Usage;

    let mut content: Vec<cogito_protocol::ContentBlock> = Vec::new();
    let mut stop_reason = None;
    let mut usage: Option<Usage> = None;

    for e in slice {
        match &e.payload {
            EventPayload::AssistantMessageAppended { text, .. } => {
                content.push(cogito_protocol::ContentBlock::Text { text: text.clone() });
            }
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
                ..
            } => {
                content.push(cogito_protocol::ContentBlock::ToolUse {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                });
            }
            EventPayload::ModelCallCompleted {
                stop_reason: sr,
                usage: u,
            } => {
                stop_reason = Some(*sr);
                usage = Some(u.clone());
            }
            _ => {}
        }
    }

    Ok(ModelOutput {
        content,
        stop_reason: stop_reason.ok_or_else(|| {
            ResumeError::Malformed("rebuild_model_output: no ModelCallCompleted".into())
        })?,
        usage: usage.unwrap_or_default(),
    })
}

/// Find the `call_id` associated with `job_id` by scanning events in
/// reverse for the most recent `JobSubmitted { job_id, .. }`.
///
/// Returns `None` if no such event exists — a structurally malformed log
/// (every `TurnPaused { job_id }` must be preceded by a matching
/// `JobSubmitted { job_id, .. }` per H08's write-before-transition
/// contract; see Sprint 8 spec
/// `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md` §9.1).
/// Callers should surface this as `ResumeError::Malformed`.
pub(crate) fn lookup_call_id_in_events(
    events: &[ConversationEvent],
    job_id: JobId,
) -> Option<String> {
    events.iter().rev().find_map(|e| match &e.payload {
        EventPayload::JobSubmitted {
            call_id,
            job_id: jid,
            ..
        } if *jid == job_id => Some(call_id.clone()),
        _ => None,
    })
}

/// Collects completed tool results within the paused turn so the caller
/// can rehydrate `completed_before_pause`. `pending_after_pause` is
/// returned alongside for parity with the `ResumeAfterJobCompletion`
/// shape; Sprint 3 invariant (≤1 async dispatch per turn) keeps it empty.
///
/// Scans from the latest `TurnStarted` up to `paused_idx`.
/// `ToolResultRecorded` events are written by the actor after
/// `ModelCallCompleted` as tools return, so the scan must cover the
/// whole turn-after-start region rather than anchoring on
/// `ModelCallCompleted`.
fn collect_paused_call_context(
    events: &[ConversationEvent],
    paused_idx: usize,
) -> (Vec<(String, ToolResult)>, Vec<ResumePendingCall>) {
    let turn_start_idx = events[..paused_idx]
        .iter()
        .rposition(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .map_or(0, |i| i + 1);
    let dispatch_slice = &events[turn_start_idx..paused_idx];

    let mut completed: Vec<(String, ToolResult)> = Vec::new();
    for e in dispatch_slice {
        if let EventPayload::ToolResultRecorded { call_id, result } = &e.payload {
            completed.push((call_id.clone(), result.clone()));
        }
    }

    // Sprint 3 invariant: ≤1 async dispatch per turn, so pending_after_pause
    // is always empty. Keeping the structure for Sprint 4 forward
    // compatibility.
    let pending_after_pause: Vec<ResumePendingCall> = Vec::new();

    (completed, pending_after_pause)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::ContentBlock;
    use cogito_protocol::SessionMeta;
    use cogito_protocol::gateway::{StopReason, Usage};
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::job::{JobId, JobOutcome};
    use cogito_protocol::tool::ToolResult;
    use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};

    fn evt(seq: u64, payload: EventPayload, turn: Option<TurnId>) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: turn,
            seq,
            ts: Utc::now(),
            payload,
        }
    }

    #[test]
    fn empty_log_returns_fresh_turn() {
        let d = replay(&[]).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, None);
    }

    #[test]
    fn only_session_started_returns_fresh_turn() {
        let events = vec![evt(
            0,
            EventPayload::SessionStarted {
                meta: SessionMeta::default(),
            },
            None,
        )];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, Some(0));
    }

    #[test]
    fn turn_completed_returns_fresh_turn() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::SessionStarted {
                    meta: SessionMeta::default(),
                },
                None,
            ),
            evt(
                1,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                2,
                EventPayload::TurnCompleted {
                    outcome: TurnOutcome::Completed,
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, Some(2));
    }

    #[test]
    fn turn_failed_returns_fresh_turn() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::TurnFailed {
                    reason: TurnFailureReason::TurnTimedOut,
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
    }

    #[test]
    fn turn_started_no_model_call_returns_restart() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::PromptComposed {
                    model: "m".into(),
                    surface_size: 0,
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::RestartCurrentTurn { turn_id } => assert_eq!(turn_id, t),
            other => panic!("expected RestartCurrentTurn, got {other:?}"),
        }
    }

    #[test]
    fn model_call_started_no_completed_returns_restart() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::RestartCurrentTurn { .. }));
    }

    #[test]
    fn model_call_completed_no_tool_use_returns_resume_from_model_completed() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::AssistantMessageAppended {
                    text: "hi".into(),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 5,
                        output_tokens: 5,
                    },
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromModelCompleted {
                turn_id,
                rebuilt_output,
            } => {
                assert_eq!(turn_id, t);
                assert_eq!(rebuilt_output.stop_reason, StopReason::EndTurn);
                assert!(
                    rebuilt_output
                        .content
                        .iter()
                        .any(|b| matches!(b, ContentBlock::Text { text } if text == "hi"))
                );
            }
            other => panic!("expected ResumeFromModelCompleted, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_recorded_no_results_returns_resume_from_tool_dispatching() {
        // Event order matches H06 stream_demux: ToolUseRecorded events are
        // written during stream consumption, BEFORE the sealing
        // ModelCallCompleted event. Resume scans `[started+1..=completed]`
        // for tool uses and `[completed+1..]` for tool results.
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ToolUseRecorded {
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"p": "/x"}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromToolDispatching {
                turn_id,
                pending,
                completed,
            } => {
                assert_eq!(turn_id, t);
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].call_id, "c1");
                assert!(completed.is_empty());
            }
            other => panic!("expected ResumeFromToolDispatching, got {other:?}"),
        }
    }

    #[test]
    fn partial_tool_results_returns_resume_from_tool_dispatching_with_split() {
        // Both ToolUseRecorded events go before ModelCallCompleted
        // (matching H06 ordering); ToolResultRecorded goes after.
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ToolUseRecorded {
                    call_id: "c1".into(),
                    tool_name: "tool_a".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ToolUseRecorded {
                    call_id: "c2".into(),
                    tool_name: "tool_b".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                4,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            evt(
                5,
                EventPayload::ToolResultRecorded {
                    call_id: "c1".into(),
                    result: ToolResult::text("ok"),
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromToolDispatching {
                pending, completed, ..
            } => {
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].call_id, "c2");
                assert_eq!(completed.len(), 1);
                assert_eq!(completed[0].0, "c1");
            }
            other => panic!("expected partial split, got {other:?}"),
        }
    }

    #[test]
    fn turn_paused_no_job_completed_returns_resume_paused_job() {
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(1, EventPayload::TurnPaused { job_id: j }, Some(t)),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumePausedJob { turn_id, job_id } => {
                assert_eq!(turn_id, t);
                assert_eq!(job_id, j);
            }
            other => panic!("expected ResumePausedJob, got {other:?}"),
        }
    }

    #[test]
    fn job_completed_after_paused_returns_resume_after_job_completion() {
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ToolUseRecorded {
                    call_id: "c_async".into(),
                    tool_name: "long_tool".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            evt(
                4,
                EventPayload::JobSubmitted {
                    call_id: "c_async".into(),
                    job_id: j,
                    tool_name: "long_tool".into(),
                },
                Some(t),
            ),
            evt(5, EventPayload::TurnPaused { job_id: j }, Some(t)),
            evt(
                6,
                EventPayload::JobCompletedRecorded {
                    job_id: j,
                    outcome: JobOutcome::Cancelled,
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeAfterJobCompletion {
                turn_id,
                job_id,
                call_id,
                ..
            } => {
                assert_eq!(turn_id, t);
                assert_eq!(job_id, j);
                assert_eq!(call_id, "c_async");
            }
            other => panic!("expected ResumeAfterJobCompletion, got {other:?}"),
        }
    }

    #[test]
    fn resume_after_job_completion_reads_call_id_from_job_submitted() {
        // Sprint 8 §9.1: ResumeAfterJobCompletion.call_id is derived from
        // the JobSubmitted event, not by scanning for the latest unmatched
        // ToolUseRecorded. Prove that by using a JobSubmitted whose call_id
        // differs from the preceding ToolUseRecorded — the JobSubmitted
        // value must win.
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(
                0,
                EventPayload::SessionStarted {
                    meta: SessionMeta::default(),
                },
                None,
            ),
            evt(
                1,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ToolUseRecorded {
                    call_id: "call_async".into(),
                    tool_name: "run_tests".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                4,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            evt(
                5,
                EventPayload::JobSubmitted {
                    call_id: "call_async".into(),
                    job_id: j,
                    tool_name: "run_tests".into(),
                },
                Some(t),
            ),
            evt(6, EventPayload::TurnPaused { job_id: j }, Some(t)),
            evt(
                7,
                EventPayload::JobCompletedRecorded {
                    job_id: j,
                    outcome: JobOutcome::Success {
                        result: ToolResult::text("done"),
                    },
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeAfterJobCompletion { call_id, .. } => {
                assert_eq!(call_id, "call_async");
            }
            other => panic!("expected ResumeAfterJobCompletion, got {other:?}"),
        }
    }

    #[test]
    fn resume_after_job_completion_without_job_submitted_is_malformed() {
        // Sprint 8 §9.1: a TurnPaused { job_id } without a preceding
        // JobSubmitted { job_id, .. } is structurally malformed — no
        // guessing via ToolUseRecorded scanning.
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ToolUseRecorded {
                    call_id: "c_async".into(),
                    tool_name: "long_tool".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            // Missing: JobSubmitted { job_id: j, .. }
            evt(4, EventPayload::TurnPaused { job_id: j }, Some(t)),
            evt(
                5,
                EventPayload::JobCompletedRecorded {
                    job_id: j,
                    outcome: JobOutcome::Cancelled,
                },
                Some(t),
            ),
        ];
        let err = replay(&events).unwrap_err();
        match err {
            ResumeError::Malformed(msg) => {
                assert!(
                    msg.contains("JobSubmitted"),
                    "expected error to mention JobSubmitted, got: {msg}"
                );
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_schema_version_returns_error() {
        let mut e = evt(
            0,
            EventPayload::SessionStarted {
                meta: SessionMeta::default(),
            },
            None,
        );
        e.schema_version = SCHEMA_VERSION + 1;
        let err = replay(&[e]).unwrap_err();
        assert!(matches!(err, ResumeError::UnsupportedSchema(_)));
    }

    #[test]
    fn job_completed_without_matching_paused_is_malformed() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::JobCompletedRecorded {
                    job_id: JobId::default(),
                    outcome: JobOutcome::Cancelled,
                },
                Some(t),
            ),
        ];
        let err = replay(&events).unwrap_err();
        assert!(matches!(err, ResumeError::Malformed(_)));
    }

    // Fix I-1: nested TurnStarted without intervening terminator is malformed.
    #[test]
    fn nested_turn_started_is_malformed() {
        let t1 = TurnId::new();
        let t2 = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t1),
            ),
            evt(
                1,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t2),
            ),
        ];
        let err = replay(&events).unwrap_err();
        assert!(matches!(err, ResumeError::Malformed(_)));
    }

    // Fix I-2: ModelCallCompleted with stop_reason=ToolUse but no ToolUseRecorded is malformed.
    #[test]
    fn model_completed_tool_use_stop_reason_without_tools_is_malformed() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            // No ToolUseRecorded follows — model promised tools but wrote none.
        ];
        let err = replay(&events).unwrap_err();
        assert!(matches!(err, ResumeError::Malformed(_)));
    }

    // Fix I-3: result matched by call_id, not position — c2 result leaves c1 pending.
    #[test]
    fn tool_split_matches_by_call_id_not_position() {
        let t = TurnId::new();
        let events = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![],
                    activate_skills: vec![],
                },
                Some(t),
            ),
            evt(
                1,
                EventPayload::ModelCallStarted { model: "m".into() },
                Some(t),
            ),
            evt(
                2,
                EventPayload::ToolUseRecorded {
                    call_id: "c1".into(),
                    tool_name: "tool_a".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                3,
                EventPayload::ToolUseRecorded {
                    call_id: "c2".into(),
                    tool_name: "tool_b".into(),
                    args: serde_json::json!({}),
                    message_id: None,
                },
                Some(t),
            ),
            evt(
                4,
                EventPayload::ModelCallCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
                Some(t),
            ),
            // Result is for c2 (the later-declared call) — proves matching is by id, not
            // position.
            evt(
                5,
                EventPayload::ToolResultRecorded {
                    call_id: "c2".into(),
                    result: ToolResult::text("ok"),
                },
                Some(t),
            ),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromToolDispatching {
                pending, completed, ..
            } => {
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].call_id, "c1");
                assert_eq!(completed.len(), 1);
                assert_eq!(completed[0].0, "c2");
            }
            other => panic!("expected ResumeFromToolDispatching, got {other:?}"),
        }
    }
}

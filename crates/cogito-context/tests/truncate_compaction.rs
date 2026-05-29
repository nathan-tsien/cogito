//! Integration tests for `TruncateCompactor` — Tasks 24 and 25.
//!
//! Task 24 tests: skeleton + idempotency.
//! Task 25 tests: full algorithm (steps 2-7).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    unused_assignments
)]

use chrono::Utc;
use cogito_context::compactor::truncate::TruncateCompactor;
use cogito_protocol::context::{
    CompactionKind, CompactionReplacement, Compactor, TokenThreshold, TruncateConfig,
};
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::gateway::Usage;
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::TurnOutcome;
use cogito_test_fixtures::context::{DummyGateway, InMemoryRecorder};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn make_event(
    seq: u64,
    session_id: SessionId,
    turn_id: Option<TurnId>,
    payload: EventPayload,
) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id,
        seq,
        ts: Utc::now(),
        payload,
    }
}

fn user_block(s: &str) -> cogito_protocol::content::ContentBlock {
    cogito_protocol::content::ContentBlock::Text { text: s.into() }
}

/// Append a single completed turn (`TurnStarted` + `AssistantMessageAppended` +
/// `TurnCompleted`) and return the `TurnId`.
fn push_completed_turn(
    events: &mut Vec<ConversationEvent>,
    seq: &mut u64,
    session_id: SessionId,
    user_text: &str,
    assistant_text: &str,
) -> TurnId {
    let tid = TurnId::new();
    events.push(make_event(
        *seq,
        session_id,
        Some(tid),
        EventPayload::TurnStarted {
            user_input: vec![user_block(user_text)],
            activate_skills: vec![],
        },
    ));
    *seq += 1;
    events.push(make_event(
        *seq,
        session_id,
        Some(tid),
        EventPayload::AssistantMessageAppended {
            text: assistant_text.into(),
        },
    ));
    *seq += 1;
    events.push(make_event(
        *seq,
        session_id,
        Some(tid),
        EventPayload::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ));
    // Advance seq past the last event of this turn so the next turn starts at
    // the correct position. `seq` is updated here and read by the caller for
    // inserting the next turn.
    *seq = seq.saturating_add(1);
    tid
}

/// Build a `CompactionInput` and call `maybe_compact`.
async fn run_compact(
    compactor: &TruncateCompactor,
    history: &[ConversationEvent],
    turn_id: TurnId,
    last_usage: Option<Usage>,
) -> (
    Vec<cogito_protocol::context::CompactionApplied>,
    InMemoryRecorder,
) {
    let strategy = HarnessStrategy::default_with_model("test");
    let gateway = DummyGateway;
    let mut recorder = InMemoryRecorder::default();

    let input = cogito_protocol::context::CompactionInput {
        session_id: SessionId::new(),
        turn_id,
        history,
        strategy: &strategy,
        last_usage,
        model_gateway: &gateway,
        recorder: &mut recorder,
    };

    let result = compactor.maybe_compact(input).await.unwrap();
    (result, recorder)
}

// ---------------------------------------------------------------------------
// Task 24: skeleton + idempotency tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn truncate_no_op_when_no_history() {
    let config = TruncateConfig {
        max_tokens: TokenThreshold::Absolute(100),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);
    let current_turn = TurnId::new();

    let (result, recorder) = run_compact(&compactor, &[], current_turn, None).await;

    assert!(result.is_empty(), "empty session must return vec![]");
    assert!(
        recorder.events.is_empty(),
        "no events must be written for empty session"
    );
}

#[tokio::test]
async fn truncate_idempotent_when_compaction_already_for_turn() {
    let session_id = SessionId::new();
    let current_turn = TurnId::new();
    let existing_event_id = EventId::new();
    let existing_range = (1u64, 5u64);

    // Pre-seed a ContextCompacted for the current turn in history.
    let history = vec![ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: existing_event_id,
        session_id,
        turn_id: Some(current_turn),
        seq: 6,
        ts: Utc::now(),
        payload: EventPayload::ContextCompacted {
            turn_id: current_turn,
            replaced_seq_range: existing_range,
            produced_by: "truncate".into(),
            replacement: CompactionReplacement::Drop,
            token_estimate_before: Some(1000),
            token_estimate_after: Some(200),
        },
    }];

    let config = TruncateConfig {
        max_tokens: TokenThreshold::Absolute(100),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);

    let (result, recorder) = run_compact(&compactor, &history, current_turn, None).await;

    assert_eq!(result.len(), 1, "must return exactly one CompactionApplied");
    assert_eq!(
        result[0].event_id, existing_event_id,
        "must reuse existing event_id"
    );
    assert_eq!(
        result[0].replaced_seq_range, existing_range,
        "must echo existing range"
    );
    assert!(
        matches!(result[0].kind, CompactionKind::Truncate),
        "kind must be Truncate"
    );
    assert!(
        recorder.events.is_empty(),
        "idempotency path must not write any new events"
    );
}

// ---------------------------------------------------------------------------
// Task 25: full algorithm tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn truncate_no_op_when_too_few_turns() {
    // 2 turns total; keep_first_user=true + keep_recent_turns=5 -> nothing to drop.
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;

    let _t1 = push_completed_turn(&mut events, &mut seq, session_id, "q1", "a1");
    let t2 = push_completed_turn(&mut events, &mut seq, session_id, "q2", "a2");

    let config = TruncateConfig {
        // Use a very small threshold so the token check passes — but there
        // should still be no drop because the retain window covers all turns.
        max_tokens: TokenThreshold::Absolute(0),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);

    let (result, recorder) = run_compact(&compactor, &events, t2, None).await;

    assert!(
        result.is_empty(),
        "too few droppable turns must return vec![], got: {result:?}"
    );
    assert!(
        recorder.events.is_empty(),
        "no events written when nothing to drop"
    );
}

#[tokio::test]
async fn truncate_writes_event_for_long_session() {
    // 30 turns; threshold = 0 (force compaction), keep_recent_turns=5.
    // Expect one ContextCompacted dropped, starting at turn 2 (first_keep_idx=1
    // because keep_first_user=true), ending at turn 25 (30-5=25).
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;
    let mut turn_ids: Vec<TurnId> = Vec::new();

    for i in 1..=30u32 {
        let tid = push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("question {i}"),
            &format!("answer {i}"),
        );
        turn_ids.push(tid);
    }

    // Use last turn as current turn (not yet in history, but we use index 29).
    let current_turn = turn_ids[29];

    let config = TruncateConfig {
        max_tokens: TokenThreshold::Absolute(0),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);

    // Provide last_usage so the token estimate path uses the absolute threshold.
    let last_usage = Some(Usage {
        input_tokens: 999_999,
        output_tokens: 0,
    });

    let (result, recorder) = run_compact(&compactor, &events, current_turn, last_usage).await;

    assert_eq!(result.len(), 1, "must write exactly one ContextCompacted");
    assert!(
        matches!(result[0].kind, CompactionKind::Truncate),
        "kind must be Truncate"
    );
    assert!(
        !recorder.events.is_empty(),
        "one event must be written to the recorder"
    );

    // The drop range must start at turn 2 (index 1) and end before turn 26 (index 25).
    let (drop_start, drop_end) = result[0].replaced_seq_range;
    // Turn 2 starts at seq 3 (each turn uses 3 events; turn 1 is seq 0-2, turn 2 is 3-5).
    let turn2_start_seq = 3u64;
    // Turn 25 ends at seq (25*3 - 1) = 74.
    let turn25_end_seq = 74u64;
    assert_eq!(
        drop_start, turn2_start_seq,
        "drop must start at turn 2 (seq {turn2_start_seq}), got {drop_start}"
    );
    assert_eq!(
        drop_end, turn25_end_seq,
        "drop must end at turn 25 (seq {turn25_end_seq}), got {drop_end}"
    );
}

#[tokio::test]
async fn truncate_advances_start_past_covered_prefix() {
    // Build a 20-turn session. Pre-seed a ContextCompacted covering turns 1-10
    // (seq 0-29). Then run truncation again on a session that still needs more
    // compaction. Assert the new compaction starts at turn 11.
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;
    let mut turn_ids: Vec<TurnId> = Vec::new();

    for i in 1..=20u32 {
        let tid = push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("question {i}"),
            &format!("answer {i}"),
        );
        turn_ids.push(tid);
    }

    // Turns 1-10 = seqs 0-29 (10 turns * 3 events each).
    // Seed a prior ContextCompacted (from a different turn) covering seq 0-29.
    let prior_compact_turn = TurnId::new();
    events.push(ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(prior_compact_turn),
        seq,
        ts: Utc::now(),
        payload: EventPayload::ContextCompacted {
            turn_id: prior_compact_turn,
            replaced_seq_range: (0, 29),
            produced_by: "truncate".into(),
            replacement: CompactionReplacement::Drop,
            token_estimate_before: Some(5000),
            token_estimate_after: Some(1000),
        },
    });
    seq += 1;

    // Current turn (the one invoking compaction now) is distinct from the previous.
    let current_turn = TurnId::new();

    let config = TruncateConfig {
        max_tokens: TokenThreshold::Absolute(0),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);

    let last_usage = Some(Usage {
        input_tokens: 999_999,
        output_tokens: 0,
    });

    let (result, recorder) = run_compact(&compactor, &events, current_turn, last_usage).await;

    // Must produce at least one compaction.
    assert_eq!(
        result.len(),
        1,
        "must write one new ContextCompacted, got: {result:?}"
    );
    let (new_start, _new_end) = result[0].replaced_seq_range;
    // Turn 11 starts at seq 30 (turns 1-10 cover seqs 0-29).
    assert_eq!(
        new_start, 30,
        "new compaction must start at turn 11 (seq 30), got {new_start}"
    );
    assert!(
        !recorder.events.is_empty(),
        "new event must have been written"
    );
}

#[tokio::test]
async fn truncate_no_op_when_all_drop_candidates_covered() {
    // Build a 10-turn session. Pre-seed a ContextCompacted covering all droppable
    // turns (turns 2-5 with keep_first=true, keep_recent=5). Assert no new compaction.
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;

    for i in 1..=10u32 {
        push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("question {i}"),
            &format!("answer {i}"),
        );
    }

    // With keep_first_user=true, keep_recent_turns=5, total=10:
    // droppable = turns [1, 5) = turns 2-5 = seq 3-14.
    // Seed a prior compaction covering seq 3-14.
    let prior_compact_turn = TurnId::new();
    events.push(ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(prior_compact_turn),
        seq,
        ts: Utc::now(),
        payload: EventPayload::ContextCompacted {
            turn_id: prior_compact_turn,
            replaced_seq_range: (3, 14),
            produced_by: "truncate".into(),
            replacement: CompactionReplacement::Drop,
            token_estimate_before: Some(2000),
            token_estimate_after: Some(400),
        },
    });
    seq += 1;

    let current_turn = TurnId::new();

    let config = TruncateConfig {
        max_tokens: TokenThreshold::Absolute(0),
        keep_first_user: true,
        keep_recent_turns: 5,
    };
    let compactor = TruncateCompactor::new(config);

    let last_usage = Some(Usage {
        input_tokens: 999_999,
        output_tokens: 0,
    });

    let (result, recorder) = run_compact(&compactor, &events, current_turn, last_usage).await;

    assert!(
        result.is_empty(),
        "all droppable turns already covered — must return vec![], got: {result:?}"
    );
    assert!(
        recorder.events.is_empty(),
        "no new event written when nothing to drop"
    );
}

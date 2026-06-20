//! Integration tests for `StandardProjector` — §5.3 multi-compaction projection.
//!
//! Covers the spec's two-compaction trace:
//! - C1: `Drop` covering turns t1-t20 (seq 1-40)
//! - C2: `Summary` covering turns t21-t60 and C1's own seq (seq 41-122)
//! - C1's seq falls inside C2's range -> C1's replacement is suppressed.
//! - t61 has `SystemPromptInjected` with suffix "Today is 2026-05-23."
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::no_effect_underscore_binding,
    clippy::doc_overindented_list_items
)]

use chrono::Utc;
use cogito_context::projector::standard::StandardProjector;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::{CompactionReplacement, HistoryProjector, ProjectedMessage};
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::TurnOutcome;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Build a minimal `ConversationEvent` envelope.
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

fn text_block(s: &str) -> ContentBlock {
    ContentBlock::Text { text: s.into() }
}

/// Append a single completed turn (`TurnStarted` + `AssistantMessageAppended` + `TurnCompleted`)
/// to `events`, advancing the seq counter.
///
/// Returns the `TurnId` used.
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
            user_input: vec![text_block(user_text)],
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
            message_id: None,
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
    *seq += 1;

    tid
}

/// Build the §5.3 two-compaction session trace.
///
/// seq 0: `SessionStarted`. seq 1-60: turns t1-t20 (20 turns x 3 events).
/// seq 61: C1 `ContextCompacted(Drop)`, covers (1, 60).
/// seq 62-181: turns t21-t60 (40 turns x 3 events).
/// seq 182: `ContextManageEntered`. seq 183: `SystemPromptInjected` (t61).
/// seq 184: `ToolFilterOverridden`. seq 185: C2 `ContextCompacted(Summary)`, covers (61, 184).
/// C1's own seq 61 is inside C2's range, so C1 is suppressed in projection.
/// seq 186: `ContextDecisionRecorded`. seq 187: `TurnStarted`(t61).
///
/// Returns `(events, t61_id)`.
fn build_two_compaction_session() -> (Vec<ConversationEvent>, TurnId) {
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;

    // seq 0: SessionStarted
    events.push(make_event(
        seq,
        session_id,
        None,
        EventPayload::SessionStarted {
            meta: cogito_protocol::session::SessionMeta {
                cogito_version: "0.1.0".into(),
                ..Default::default()
            },
        },
    ));
    seq += 1;

    // seq 1-60: turns t1..t20 (3 events each = 60 events, seq 1-60)
    let seq_t1_start = seq; // = 1
    for i in 1..=20u32 {
        push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("user turn {i}"),
            &format!("assistant reply {i}"),
        );
    }
    let seq_t20_end = seq - 1; // = 60

    // seq 61: C1 ContextCompacted(Drop) covering turns t1-t20 (seq 1-60)
    let c1_seq = seq; // = 61
    let c1_turn = TurnId::new(); // nominal compaction turn (t31 in spec, but exact id doesn't matter)
    events.push(make_event(
        c1_seq,
        session_id,
        Some(c1_turn),
        EventPayload::ContextCompacted {
            turn_id: c1_turn,
            replaced_seq_range: (seq_t1_start, seq_t20_end),
            produced_by: "truncate".into(),
            replacement: CompactionReplacement::Drop,
            token_estimate_before: Some(5200),
            token_estimate_after: Some(800),
        },
    ));
    seq += 1;

    // seq 62-181: turns t21..t60 (40 completed turns, 3 events each = 120 events)
    for i in 21..=60u32 {
        push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("user turn {i}"),
            &format!("assistant reply {i}"),
        );
    }
    // t61 context-management block
    let t61_id = TurnId::new();

    // seq 182: ContextManageEntered
    events.push(make_event(
        seq,
        session_id,
        Some(t61_id),
        EventPayload::ContextManageEntered {},
    ));
    seq += 1;

    // seq 183: SystemPromptInjected for t61
    let spi_seq = seq;
    let spi_event_id = EventId::new();
    events.push(ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: spi_event_id,
        session_id,
        turn_id: Some(t61_id),
        seq: spi_seq,
        ts: Utc::now(),
        payload: EventPayload::SystemPromptInjected {
            turn_id: t61_id,
            suffix: "Today is 2026-05-23.".into(),
            contributors: vec!["date".into()],
            produced_by: "date-injector".into(),
        },
    });
    seq += 1;

    // seq 184: ToolFilterOverridden for t61
    let tfo_seq = seq;
    let tfo_event_id = EventId::new();
    events.push(ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: tfo_event_id,
        session_id,
        turn_id: Some(t61_id),
        seq: tfo_seq,
        ts: Utc::now(),
        payload: EventPayload::ToolFilterOverridden {
            turn_id: t61_id,
            mode: cogito_protocol::context::ToolFilterOverrideMode::Inherit,
            contributors: vec![],
            produced_by: "noop".into(),
        },
    });
    seq += 1;

    // seq 185: C2 ContextCompacted(Summary) covering (seq_t21_start, tfo_seq)
    // This range (62, 184) includes C1's seq 61? No — 61 < 62 so C1 is NOT in this range.
    //
    // Per spec: C2 covers turns t21-t60 AND the intervening C1 event.
    // C1 is at seq 61, t21_start is 62. To cover C1, the range start must be <= 61.
    // We use (c1_seq, tfo_seq) = (61, 184) so C1 seq 61 is at the boundary (inclusive).
    let c2_seq = seq;
    let c2_turn_id = t61_id;
    let c2_replaced_start = c1_seq; // = 61 (covers C1 itself)
    let c2_replaced_end = tfo_seq; // = 184

    let c2_event_id = EventId::new();
    events.push(ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: c2_event_id,
        session_id,
        turn_id: Some(t61_id),
        seq: c2_seq,
        ts: Utc::now(),
        payload: EventPayload::ContextCompacted {
            turn_id: c2_turn_id,
            replaced_seq_range: (c2_replaced_start, c2_replaced_end),
            produced_by: "summarize".into(),
            replacement: CompactionReplacement::Summary {
                text: "weather then follow-up forecasts. The user asked about climate trends and we discussed scenarios.".into(),
                model: "claude-haiku-4-5".into(),
            },
            token_estimate_before: Some(8400),
            token_estimate_after: Some(2300),
        },
    });
    seq += 1;

    // seq 186: ContextDecisionRecorded
    events.push(make_event(
        seq,
        session_id,
        Some(t61_id),
        EventPayload::ContextDecisionRecorded {
            turn_id: t61_id,
            compactions: vec![c2_event_id],
            system_prompt_event: spi_event_id,
            tool_filter_event: tfo_event_id,
            errors: cogito_protocol::context::ContextDecisionErrors::default(),
        },
    ));
    seq += 1;

    // seq 187: TurnStarted for t61 (the actual user message)
    events.push(make_event(
        seq,
        session_id,
        Some(t61_id),
        EventPayload::TurnStarted {
            user_input: vec![text_block("What are the latest climate projections?")],
            activate_skills: vec![],
        },
    ));

    (events, t61_id)
}

// ---------------------------------------------------------------------------
// Test 1: full §5.3 two-compaction session projects correctly
// ---------------------------------------------------------------------------

#[test]
fn projects_two_compaction_session_correctly() {
    let (events, current_turn) = build_two_compaction_session();

    let mut strategy = HarnessStrategy::default_with_model("test");
    strategy.system_prompt = "You are a helpful assistant.".into();

    let msgs = StandardProjector.project(&events, &strategy, current_turn);

    // System message must include both the base prompt and the injected suffix.
    let ProjectedMessage::System(sys) = &msgs[0] else {
        panic!("expected System as first message, got: {:?}", msgs[0]);
    };
    assert!(
        sys.contains("You are a helpful assistant."),
        "system prompt missing base text: {sys:?}"
    );
    assert!(
        sys.contains("Today is 2026-05-23."),
        "system prompt missing injected suffix: {sys:?}"
    );
    assert!(
        sys.contains("\n\n"),
        "system prompt and suffix must be separated by a blank line: {sys:?}"
    );

    // C2's Summary block must appear as a User message.
    let summary_msg = msgs
        .iter()
        .find_map(|m| match m {
            ProjectedMessage::User(t) if t.contains("<conversation_summary>") => Some(t),
            _ => None,
        })
        .expect("C2 Summary replacement must be projected as a User message");
    assert!(
        summary_msg.contains("weather then follow-up forecasts"),
        "summary text missing in projection: {summary_msg:?}"
    );

    // Exactly one conversation_summary block (C1 Drop is suppressed because its seq
    // falls within C2's covered range).
    let summary_count = msgs
        .iter()
        .filter(|m| matches!(m, ProjectedMessage::User(t) if t.contains("<conversation_summary>")))
        .count();
    assert_eq!(
        summary_count, 1,
        "expected exactly one summary block; C1 (Drop) must be suppressed when covered by C2"
    );

    // t1-t20 user messages must not survive (covered by both C1 and C2).
    let t1_t20_leaked = msgs
        .iter()
        .any(|m| matches!(m, ProjectedMessage::User(t) if t.contains("user turn 1") || t.contains("user turn 10")));
    assert!(
        !t1_t20_leaked,
        "turn t1-t20 messages must not appear in projection (covered)"
    );

    // t21-t60 user messages must not survive (covered by C2).
    let t21_t60_leaked = msgs
        .iter()
        .any(|m| matches!(m, ProjectedMessage::User(t) if t.contains("user turn 21") || t.contains("user turn 50")));
    assert!(
        !t21_t60_leaked,
        "turn t21-t60 messages must not appear in projection (covered by C2)"
    );

    // t61's actual user input must appear after the summary.
    let t61_pos = msgs.iter().position(|m| {
        matches!(m, ProjectedMessage::User(t) if t.contains("What are the latest climate projections?"))
    });
    let summary_pos = msgs.iter().position(
        |m| matches!(m, ProjectedMessage::User(t) if t.contains("<conversation_summary>")),
    );
    assert!(
        t61_pos.is_some(),
        "t61 TurnStarted user input must be projected"
    );
    assert!(
        summary_pos.unwrap() < t61_pos.unwrap(),
        "C2 Summary must appear before t61's TurnStarted in projection"
    );
}

// ---------------------------------------------------------------------------
// Test 2: covered compaction event is suppressed
// ---------------------------------------------------------------------------

#[test]
fn covered_compaction_event_is_suppressed() {
    // Minimal three-event setup:
    // - seq 0-1: early turn (will be covered by C1)
    // - seq 2:   C1 ContextCompacted(Summary), covers (0, 1)
    // - seq 3-4: middle turn (will be covered by C2)
    // - seq 5:   C2 ContextCompacted(Summary), covers (2, 4)
    //            C1's seq 2 is inside (2, 4), so C1's Summary must NOT appear.
    // - seq 6:   TurnStarted for current turn
    //
    // Expected: exactly one summary block (from C2, not C1).
    let session_id = SessionId::new();
    let turn_a = TurnId::new();
    let turn_b = TurnId::new();
    let current = TurnId::new();

    let mk = |seq, tid: Option<TurnId>, payload| make_event(seq, session_id, tid, payload);

    let events = vec![
        mk(
            0,
            Some(turn_a),
            EventPayload::TurnStarted {
                user_input: vec![text_block("early user")],
                activate_skills: vec![],
            },
        ),
        mk(
            1,
            Some(turn_a),
            EventPayload::AssistantMessageAppended {
                text: "early assistant".into(),
                message_id: None,
            },
        ),
        // C1: Summary covering (0, 1)
        mk(
            2,
            Some(turn_b),
            EventPayload::ContextCompacted {
                turn_id: turn_b,
                replaced_seq_range: (0, 1),
                produced_by: "summarize".into(),
                replacement: CompactionReplacement::Summary {
                    text: "C1 summary text".into(),
                    model: "claude-haiku-4-5".into(),
                },
                token_estimate_before: None,
                token_estimate_after: None,
            },
        ),
        mk(
            3,
            Some(turn_b),
            EventPayload::TurnStarted {
                user_input: vec![text_block("middle user")],
                activate_skills: vec![],
            },
        ),
        mk(
            4,
            Some(turn_b),
            EventPayload::AssistantMessageAppended {
                text: "middle assistant".into(),
                message_id: None,
            },
        ),
        // C2: Summary covering (2, 4) — includes C1's seq 2
        mk(
            5,
            Some(current),
            EventPayload::ContextCompacted {
                turn_id: current,
                replaced_seq_range: (2, 4),
                produced_by: "summarize".into(),
                replacement: CompactionReplacement::Summary {
                    text: "C2 summary text".into(),
                    model: "claude-haiku-4-5".into(),
                },
                token_estimate_before: None,
                token_estimate_after: None,
            },
        ),
        mk(
            6,
            Some(current),
            EventPayload::TurnStarted {
                user_input: vec![text_block("current user")],
                activate_skills: vec![],
            },
        ),
    ];

    let strategy = HarnessStrategy::default_with_model("test");
    let msgs = StandardProjector.project(&events, &strategy, current);

    // Only C2's summary should appear.
    let summaries: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            ProjectedMessage::User(t) if t.contains("<conversation_summary>") => Some(t.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(
        summaries.len(),
        1,
        "exactly one summary block expected; C1 must be suppressed. Got: {summaries:?}"
    );
    assert!(
        summaries[0].contains("C2 summary text"),
        "surviving summary must be C2's, got: {:?}",
        summaries[0]
    );
    assert!(
        !summaries[0].contains("C1 summary text"),
        "C1 summary must not appear"
    );
}

// ---------------------------------------------------------------------------
// Test 3: current-turn events post-compaction are emitted in order
// ---------------------------------------------------------------------------

#[test]
fn current_turn_events_post_compaction_are_emitted_in_order() {
    // Layout:
    // - seq 0-1: early turn (covered by C)
    // - seq 2:   C ContextCompacted(Summary), covers (0, 1)
    // - seq 3:   SystemPromptInjected for current turn
    // - seq 4:   TurnStarted for current turn (user message)
    // - seq 5:   AssistantMessageAppended for current turn
    //
    // Expected order in projection:
    // 1. System (with injected suffix)
    // 2. User (from C's Summary)
    // 3. User (from TurnStarted at seq 4)
    // 4. Assistant (from AssistantMessageAppended at seq 5)
    let session_id = SessionId::new();
    let early_turn = TurnId::new();
    let current = TurnId::new();

    let mk = |seq, tid: Option<TurnId>, payload| make_event(seq, session_id, tid, payload);

    let events = vec![
        mk(
            0,
            Some(early_turn),
            EventPayload::TurnStarted {
                user_input: vec![text_block("early question")],
                activate_skills: vec![],
            },
        ),
        mk(
            1,
            Some(early_turn),
            EventPayload::AssistantMessageAppended {
                text: "early answer".into(),
                message_id: None,
            },
        ),
        mk(
            2,
            Some(current),
            EventPayload::ContextCompacted {
                turn_id: current,
                replaced_seq_range: (0, 1),
                produced_by: "summarize".into(),
                replacement: CompactionReplacement::Summary {
                    text: "Prior discussion about early topics.".into(),
                    model: "claude-haiku-4-5".into(),
                },
                token_estimate_before: None,
                token_estimate_after: None,
            },
        ),
        mk(
            3,
            Some(current),
            EventPayload::SystemPromptInjected {
                turn_id: current,
                suffix: "Today is 2026-05-23.".into(),
                contributors: vec!["date".into()],
                produced_by: "date-injector".into(),
            },
        ),
        mk(
            4,
            Some(current),
            EventPayload::TurnStarted {
                user_input: vec![text_block("current question")],
                activate_skills: vec![],
            },
        ),
        mk(
            5,
            Some(current),
            EventPayload::AssistantMessageAppended {
                text: "current answer".into(),
                message_id: None,
            },
        ),
    ];

    let mut strategy = HarnessStrategy::default_with_model("test");
    strategy.system_prompt = "You are a helpful assistant.".into();

    let msgs = StandardProjector.project(&events, &strategy, current);

    // Minimum expected: System, User(summary), User(current), Assistant(current).
    assert!(
        msgs.len() >= 4,
        "expected at least 4 messages, got {}: {msgs:?}",
        msgs.len()
    );

    // System is first.
    assert!(
        matches!(&msgs[0], ProjectedMessage::System(_)),
        "first message must be System"
    );

    // Locate key positions.
    let summary_pos = msgs.iter().position(
        |m| matches!(m, ProjectedMessage::User(t) if t.contains("<conversation_summary>")),
    );
    let current_user_pos = msgs
        .iter()
        .position(|m| matches!(m, ProjectedMessage::User(t) if t.contains("current question")));
    let current_asst_pos = msgs
        .iter()
        .position(|m| matches!(m, ProjectedMessage::Assistant(_)));

    assert!(summary_pos.is_some(), "summary block must be present");
    assert!(
        current_user_pos.is_some(),
        "current user message must be present"
    );
    assert!(
        current_asst_pos.is_some(),
        "current assistant message must be present"
    );

    let sp = summary_pos.unwrap();
    let cup = current_user_pos.unwrap();
    let cap = current_asst_pos.unwrap();

    assert!(
        sp < cup,
        "summary (pos {sp}) must precede current user (pos {cup})"
    );
    assert!(
        cup < cap,
        "current user (pos {cup}) must precede current assistant (pos {cap})"
    );

    // No early-turn events should survive (they were covered by C).
    let early_leaked = msgs
        .iter()
        .any(|m| matches!(m, ProjectedMessage::User(t) if t.contains("early question")));
    assert!(!early_leaked, "covered early user message must not appear");
}

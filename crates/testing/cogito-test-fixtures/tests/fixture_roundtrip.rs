//! Asserts the checked-in sample fixture parses back into the in-code
//! canonical session. Fails if either drifts — guarding both the JSONL
//! wire shape and the in-code builder against silent divergence.

use cogito_protocol::ConversationEvent;
use cogito_test_fixtures::fixtures::{canonical_sample_session, canonical_skill_session};

/// Verify the truncate-compaction sample fixture parses without error.
/// This does not check against an in-code builder (there is none for this
/// fixture) — it only guards against JSONL syntax or schema drift.
#[test]
fn truncate_fixture_parses() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-truncate-v1.jsonl");
    let text = std::fs::read_to_string(&path)?;
    let events: Vec<ConversationEvent> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()?;
    // The fixture has 34 lines: 1 session + 4 turns * ~8 events each.
    assert!(
        events.len() >= 20,
        "fixture has too few events: {}",
        events.len()
    );
    // Exactly one context_compacted event should be present (turn 3, seq 19).
    let compacted = events
        .iter()
        .filter(|e| {
            matches!(
                e.payload,
                cogito_protocol::EventPayload::ContextCompacted { .. }
            )
        })
        .count();
    assert_eq!(compacted, 1, "expected exactly one ContextCompacted");
    Ok(())
}

#[test]
fn sample_fixture_parses_to_canonical_session() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-v1.jsonl");
    let text = std::fs::read_to_string(&path)?;
    let parsed: Vec<ConversationEvent> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()?;
    let expected = canonical_sample_session();
    assert_eq!(
        parsed, expected,
        "fixture file drifted from canonical session; \
         regenerate via `cargo run -p cogito-test-fixtures --bin write-sample`",
    );
    Ok(())
}

/// Verify the Sprint 7 skill fixture parses round-trip and matches the
/// in-code builder. Per Task 23 of the Sprint 7 plan, this guards both
/// the new `TurnStarted.activate_skills` field and the
/// `SkillActivated` payload against silent wire-shape drift.
#[test]
fn sample_skill_v1_roundtrips() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-skill-v1.jsonl");
    let text = std::fs::read_to_string(&path)?;
    let mut parsed: Vec<ConversationEvent> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let ev: ConversationEvent = serde_json::from_str(line)?;
        let reserialized = serde_json::to_string(&ev)?;
        let again: ConversationEvent = serde_json::from_str(&reserialized)?;
        assert_eq!(ev, again, "event survives serialize/deserialize roundtrip");
        parsed.push(ev);
    }
    let expected = canonical_skill_session();
    assert_eq!(
        parsed, expected,
        "skill fixture file drifted from canonical session; \
         regenerate via `cargo run -p cogito-test-fixtures --bin write-sample`",
    );
    Ok(())
}

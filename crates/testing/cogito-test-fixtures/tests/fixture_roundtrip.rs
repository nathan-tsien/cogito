//! Asserts the checked-in sample fixture parses back into the in-code
//! canonical session. Fails if either drifts — guarding both the JSONL
//! wire shape and the in-code builder against silent divergence.

use cogito_protocol::ConversationEvent;
use cogito_test_fixtures::fixtures::canonical_sample_session;

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

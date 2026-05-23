//! Canonical sample fixtures used by both contract tests and external
//! readers as a worked example of the v1 JSONL schema.
//!
//! The functions in this module mint a deterministic, byte-reproducible
//! session that exercises every variant of [`EventPayload`]. The
//! companion `write-sample` binary serializes it to the checked-in
//! `fixtures/sessions/sample-v1.jsonl` file, and
//! `tests/fixture_roundtrip.rs` locks the file shape against the
//! in-code builder.

#![allow(clippy::expect_used, clippy::unwrap_used)]
// Justification: this module is test infrastructure that mints fixture
// data from hard-coded inputs. Any error here is a programmer error
// (an invalid ULID literal, an unsupported timestamp), not a runtime
// failure path — surfacing it as a panic during fixture construction
// makes the broken test data obvious. Mirrors `store_contract.rs`.

use chrono::{TimeZone, Utc};
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::tool::ToolResult;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use cogito_protocol::{
    ContentBlock, ConversationEvent, EventId, EventPayload, SCHEMA_VERSION, SessionId, SessionMeta,
    TurnId,
};
use ulid::Ulid;

/// Build the canonical sample session: one session covering all known
/// [`EventPayload`] variants in their natural turn order, with
/// deterministic identifiers and timestamps so the JSONL file is
/// byte-reproducible.
///
/// # Deterministic ULID inputs
///
/// The three top-level IDs (`SessionId`, `TurnId`, `JobId`) are minted
/// from fixed Crockford base32 strings. The per-event `EventId` is
/// minted with [`Ulid::from_parts`] using the sequence number as the
/// timestamp portion, so events sort identically to their `seq` field.
///
/// # Determinism caveat — `JobId`
///
/// [`JobId`] does not expose a public `From<Ulid>` constructor; it is a
/// `#[serde(transparent)]` newtype. We deserialize a fixed ULID string
/// through serde to produce a deterministic value without modifying
/// the protocol crate.
///
/// # Panics
///
/// Panics if the hard-coded ULID literals, the fixed UTC timestamp, or
/// the JSON-side `JobId` deserialization ever stop being valid. These
/// are all programmer-error inputs baked into the fixture; failing
/// loudly here means a maintainer broke the fixture, not a runtime
/// edge case.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn canonical_sample_session() -> Vec<ConversationEvent> {
    // Deterministic IDs from fixed Crockford base32 ULIDs.
    // Crockford excludes I, L, O, U — the literals below are hand-picked
    // to be both human-recognizable and valid.
    let sid = SessionId::from(
        Ulid::from_string("01J9C0R0K0SESS10NSESS10NSE").expect("fixed session ulid"),
    );
    let turn =
        TurnId::from(Ulid::from_string("01J9C0R0K0TRN0TRN0TRN0TRN0").expect("fixed turn ulid"));
    // JobId has no public Ulid constructor; deserialize through its
    // serde(transparent) shape to keep determinism without poking at
    // protocol internals.
    let job: JobId = serde_json::from_value(serde_json::Value::String(
        "01J9C0R0K0J0BJ0BJ0BJ0BJ0BJ".into(),
    ))
    .expect("fixed job ulid deserializes through serde(transparent)");

    // Per-event EventId derived from a monotonic counter packed into
    // the ULID timestamp portion so order matches `seq`.
    let mut counter: u64 = 0;
    let mut next_event_id = || {
        counter += 1;
        EventId::from(Ulid::from_parts(counter, 0))
    };

    // Fixed base timestamp: 2026-05-18T10:00:00Z. `timestamp_opt` +
    // `single()` avoids the deny-listed `unwrap` on chrono's
    // `MappedLocalTime`. The literal seconds-since-epoch value is
    // 2026-05-18T10:00:00Z = 1_779_098_400.
    let ts0 = Utc
        .timestamp_opt(1_779_098_400, 0)
        .single()
        .expect("fixed UTC timestamp is unambiguous");

    let mut envelope =
        |seq: u64, turn_id: Option<TurnId>, payload: EventPayload| ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: next_event_id(),
            session_id: sid,
            turn_id,
            seq,
            ts: ts0 + chrono::Duration::milliseconds(i64::try_from(seq * 100).unwrap_or(0)),
            payload,
        };

    vec![
        envelope(
            0,
            None,
            EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    strategy: Some("default".into()),
                    model: Some("claude-sonnet-4-6".into()),
                    user_id: Some("u_42".into()),
                    ..Default::default()
                },
            },
        ),
        envelope(
            1,
            Some(turn),
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text {
                    text: "read /tmp/x".into(),
                }],
                activate_skills: vec![],
            },
        ),
        envelope(
            2,
            Some(turn),
            EventPayload::AssistantMessageAppended {
                text: "Reading /tmp/x now.".into(),
            },
        ),
        envelope(
            3,
            Some(turn),
            EventPayload::ToolUseRecorded {
                call_id: "toolu_01".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({"path": "/tmp/x"}),
            },
        ),
        envelope(
            4,
            Some(turn),
            EventPayload::ToolResultRecorded {
                call_id: "toolu_01".into(),
                result: ToolResult::text("file contents"),
            },
        ),
        envelope(5, Some(turn), EventPayload::TurnPaused { job_id: job }),
        envelope(
            6,
            Some(turn),
            EventPayload::JobCompletedRecorded {
                job_id: job,
                // JobOutcome has no Default impl; the simplest unit
                // variant exercises the wire shape. (Mirrors the
                // precedent in event.rs's `all_nine_variants_roundtrip`
                // test.)
                outcome: JobOutcome::Cancelled,
            },
        ),
        envelope(
            7,
            Some(turn),
            EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ),
        envelope(
            8,
            Some(turn),
            EventPayload::TurnFailed {
                // TurnFailureReason has no `Cancelled` variant — the
                // turn-cancelled state lives on `TurnOutcome::Cancelled`,
                // not as a failure. Use `TurnTimedOut` to give the
                // fixture a representative failure shape (mirrors the
                // precedent in event.rs's roundtrip test).
                reason: TurnFailureReason::TurnTimedOut,
            },
        ),
        // Sprint 2: exercise the new context-management transition events.
        envelope(9, Some(turn), EventPayload::ContextManageEntered {}),
        // Sprint 3: exercise the new model-call sealing event.
        envelope(
            10,
            Some(turn),
            EventPayload::ModelCallCompleted {
                stop_reason: cogito_protocol::gateway::StopReason::ToolUse,
                usage: cogito_protocol::gateway::Usage {
                    input_tokens: 120,
                    output_tokens: 45,
                },
            },
        ),
        // ADR-0019: exercise the new thinking-block persistence event.
        envelope(
            11,
            Some(turn),
            EventPayload::ThinkingBlockRecorded {
                text: "I should grep for the symbol.".into(),
                provider_opaque: Some(serde_json::json!({"signature": "abc123"})),
            },
        ),
    ]
}

/// Absolute path to a recorded SSE fixture under `fixtures/sse/`.
///
/// Returns a [`std::path::PathBuf`] rooted at this crate's source tree so the
/// path is valid regardless of where the test binary runs.
#[must_use]
pub fn sse_fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sse")
        .join(name)
}

/// Serialize the canonical sample to JSONL bytes. Each event becomes
/// one line; the final byte is a trailing newline.
///
/// # Panics
///
/// Panics if `serde_json::to_vec` fails on any event. The canonical
/// fixture is built from concrete values that all derive `Serialize`
/// without fallible paths, so this is treated as a programmer error.
#[must_use]
pub fn canonical_sample_jsonl() -> Vec<u8> {
    let mut buf = Vec::new();
    for event in canonical_sample_session() {
        let mut line = serde_json::to_vec(&event).expect("event serializes");
        line.push(b'\n');
        buf.extend_from_slice(&line);
    }
    buf
}

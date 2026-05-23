//! Cross-provider threshold scaling tests for `TruncateCompactor` — Task 26.
//!
//! Verifies that `TokenThreshold::Ratio` scales correctly with different model
//! context windows (Opus 1 M, Sonnet 200 k, vLLM 32 k) and that
//! `TokenThreshold::Absolute` ignores `model_limits` entirely.
//!
//! Observable behavior is used throughout: rather than inspecting computed
//! thresholds directly, each test seeds `last_usage.input_tokens` just above
//! or just below the expected threshold and asserts whether compaction fires.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use chrono::Utc;
use cogito_context::compactor::truncate::TruncateCompactor;
use cogito_mock_model::MockModelGateway;
use cogito_protocol::context::{CompactionInput, Compactor, TokenThreshold, TruncateConfig};
use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::gateway::Usage;
use cogito_protocol::ids::{EventId, SessionId, TurnId};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::TurnOutcome;
use cogito_test_fixtures::context::InMemoryRecorder;

// ---------------------------------------------------------------------------
// Fixture helpers (shared with truncate_compaction.rs pattern)
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

/// Append one completed turn (`TurnStarted` + `AssistantMessageAppended` +
/// `TurnCompleted`) into `events` and advance `*seq`.
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
    *seq = seq.saturating_add(1);
    tid
}

/// Build a history of `n_turns` completed turns and call `maybe_compact`
/// with `last_usage.input_tokens = input_tokens` against `gateway`.
///
/// Returns `(compaction_results, recorder)`.
async fn run_with_gateway(
    compactor: &TruncateCompactor,
    gateway: &MockModelGateway,
    n_turns: u32,
    input_tokens: u32,
) -> (
    Vec<cogito_protocol::context::CompactionApplied>,
    InMemoryRecorder,
) {
    let session_id = SessionId::new();
    let mut events: Vec<ConversationEvent> = Vec::new();
    let mut seq: u64 = 0;
    let mut last_tid = TurnId::new();

    for i in 1..=n_turns {
        last_tid = push_completed_turn(
            &mut events,
            &mut seq,
            session_id,
            &format!("question {i}"),
            &format!("answer {i}"),
        );
    }

    let strategy = HarnessStrategy::default_with_model("test");
    let mut recorder = InMemoryRecorder::default();

    let input = CompactionInput {
        session_id,
        turn_id: last_tid,
        history: &events,
        strategy: &strategy,
        last_usage: Some(Usage {
            input_tokens,
            output_tokens: 0,
        }),
        model_gateway: gateway,
        recorder: &mut recorder,
    };

    let result = compactor.maybe_compact(input).await.unwrap();
    (result, recorder)
}

// ---------------------------------------------------------------------------
// Ratio threshold — 1 M context window (e.g. Claude Opus)
//
// threshold = 1_000_000 * 0.75 - 8_192 = 741_808
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ratio_threshold_compacts_when_tokens_exceed_1m_window() {
    let gateway = MockModelGateway::new().with_context_window(1_000_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 741_810 > 741_808 — must compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 741_810).await;
    assert!(
        !result.is_empty(),
        "should compact when input_tokens (741_810) exceeds threshold (741_808)"
    );
}

#[tokio::test]
async fn ratio_threshold_skips_when_tokens_below_1m_window() {
    let gateway = MockModelGateway::new().with_context_window(1_000_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 741_800 < 741_808 — must NOT compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 741_800).await;
    assert!(
        result.is_empty(),
        "should NOT compact when input_tokens (741_800) is below threshold (741_808)"
    );
}

// ---------------------------------------------------------------------------
// Ratio threshold — 200 k context window (e.g. Claude Sonnet)
//
// threshold = 200_000 * 0.75 - 8_192 = 141_808
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ratio_threshold_compacts_when_tokens_exceed_200k_window() {
    let gateway = MockModelGateway::new().with_context_window(200_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 141_810 > 141_808 — must compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 141_810).await;
    assert!(
        !result.is_empty(),
        "should compact when input_tokens (141_810) exceeds threshold (141_808) for 200 k window"
    );
}

#[tokio::test]
async fn ratio_threshold_skips_when_tokens_below_200k_window() {
    let gateway = MockModelGateway::new().with_context_window(200_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 141_800 < 141_808 — must NOT compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 141_800).await;
    assert!(
        result.is_empty(),
        "should NOT compact when input_tokens (141_800) is below threshold (141_808) for 200 k window"
    );
}

// ---------------------------------------------------------------------------
// Ratio threshold — 32_768 context window (e.g. vLLM 32 k model)
//
// threshold = 32_768 * 0.75 - 8_192 = 24_576 - 8_192 = 16_384
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ratio_threshold_compacts_when_tokens_exceed_32k_window() {
    let gateway = MockModelGateway::new().with_context_window(32_768);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 16_386 > 16_384 — must compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 16_386).await;
    assert!(
        !result.is_empty(),
        "should compact when input_tokens (16_386) exceeds threshold (16_384) for 32 k window"
    );
}

#[tokio::test]
async fn ratio_threshold_skips_when_tokens_below_32k_window() {
    let gateway = MockModelGateway::new().with_context_window(32_768);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio {
            of_context_window: 0.75,
            safety_headroom: 8_192,
        },
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 16_382 < 16_384 — must NOT compact.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 16_382).await;
    assert!(
        result.is_empty(),
        "should NOT compact when input_tokens (16_382) is below threshold (16_384) for 32 k window"
    );
}

// ---------------------------------------------------------------------------
// Absolute threshold — ignores model_limits regardless of window size
//
// Gateway has a 1 M window (ratio would give ~741 k threshold).
// Absolute(50_000) must fire at 50_001 and be silent at 49_999.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn absolute_threshold_compacts_above_limit_ignoring_large_window() {
    // A 1 M window with Ratio 0.75 would yield a threshold of ~741 k.
    // Absolute(50_000) must override that and fire at 50_001.
    let gateway = MockModelGateway::new().with_context_window(1_000_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Absolute(50_000),
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 50_001).await;
    assert!(
        !result.is_empty(),
        "Absolute(50_000) must compact at 50_001 regardless of the 1 M window"
    );
}

#[tokio::test]
async fn absolute_threshold_skips_below_limit_ignoring_large_window() {
    let gateway = MockModelGateway::new().with_context_window(1_000_000);
    let compactor = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Absolute(50_000),
        keep_first_user: true,
        keep_recent_turns: 5,
    });

    // 49_999 is below Absolute(50_000); the 1 M ratio threshold (~741 k)
    // is irrelevant — no compaction must occur.
    let (result, _recorder) = run_with_gateway(&compactor, &gateway, 30, 49_999).await;
    assert!(
        result.is_empty(),
        "Absolute(50_000) must NOT compact at 49_999 regardless of the 1 M window"
    );
}

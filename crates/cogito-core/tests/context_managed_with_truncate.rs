//! Integration test: `CompactorConfig::Truncate` with a tiny threshold produces
//! at least one `ContextCompacted` event over 12 turns.
//!
//! Strategy:
//! - `max_tokens = Absolute(100)` (~400 visible chars to trigger).
//! - `keep_first_user = true` (first turn is preserved).
//! - `keep_recent_turns = 2` (only keep 2 most-recent completed turns).
//! - Each user message is ~200 chars; assistant reply is ~80 chars.
//!   From turn 3 onwards the estimate (~280 chars / 4 = 70 tokens per turn)
//!   accumulates: by turn 2 we already have ~140 tokens in history which
//!   exceeds the 100-token threshold. Since `keep_first_user=true` and
//!   `keep_recent_turns=2`, from turn 4 onwards turn 2 is eligible to be dropped.
//!
//! Assertions:
//! - At least one `ContextCompacted` event exists in the log.
//! - Every `ContextCompacted` event has `replacement = Drop`.
//! - Every `ContextCompacted` event has a `replaced_seq_range.0` that is the
//!   seq of some `TurnStarted` event (compaction starts at turn boundaries).
//! - At least one `ContextDecisionRecorded.compactions` is non-empty (the
//!   summary for the turn when compaction happened includes the event id).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::EventPayload;
use cogito_protocol::context::{
    CompactionReplacement, CompactorConfig, ContextConfig, HistoryProjectorConfig,
    SystemPromptInjectorConfig, TokenThreshold, ToolFilterOverriderConfig, TruncateConfig,
};
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

/// Assistant reply constant for every turn.
const ASSISTANT_REPLY: &str =
    "This is a short assistant response that is roughly one hundred characters long.!";

/// Build a reply script for one turn.
fn make_reply() -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: ASSISTANT_REPLY.into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: ASSISTANT_REPLY.into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 50,
                output_tokens: 25,
            },
        },
    ]
}

/// Build a 200-character user message for turn `n`.
fn user_msg(n: u32) -> String {
    // Fixed prefix + padding to reach ~200 chars.
    let base = format!("Turn {n} user message: this text is padded to reach two hundred chars.");
    let padding = "x".repeat(200usize.saturating_sub(base.len()));
    format!("{base}{padding}")
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn truncate_compactor_produces_at_least_one_compacted_event()
-> Result<(), Box<dyn std::error::Error>> {
    const TURNS: u32 = 12;

    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    // Pre-load TURNS replies.
    for _ in 0..TURNS {
        mock.push_reply(make_reply());
    }

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Configure a TruncateCompactor with a very small absolute threshold.
    // At ~70 tokens per turn (280 chars / 4), turn 2 already exceeds the
    // 100-token threshold. keep_recent_turns=2 means from turn 4 onwards,
    // turn 2 is eligible to be dropped (turn 1 is preserved by keep_first_user,
    // and turns N-1 and N are the recent tail).
    let context_config = ContextConfig {
        compactor: CompactorConfig::Truncate(TruncateConfig {
            max_tokens: TokenThreshold::Absolute(100),
            keep_first_user: true,
            keep_recent_turns: 2,
        }),
        history_projector: HistoryProjectorConfig::Standard,
        system_prompt_injector: SystemPromptInjectorConfig::None,
        tool_filter_overrider: ToolFilterOverriderConfig::None,
    };

    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.context = context_config;

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn cogito_protocol::store::ConversationStore>)
        .model(mock as Arc<dyn cogito_protocol::gateway::ModelGateway>)
        .tools(tools)
        .strategy(strategy)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    // Drive TURNS sequential turns, waiting for each TurnCompleted before
    // submitting the next so events are written in a deterministic order.
    for n in 1..=TURNS {
        let mut events = handle.subscribe();
        handle.submit_user_text(user_msg(n)).await?;

        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(StreamEvent::TurnCompleted { .. }) => return true,
                    Ok(StreamEvent::TurnFailed { .. }) | Err(_) => return false,
                    Ok(_) => {}
                }
            }
        })
        .await
        .unwrap_or(false);

        assert!(got, "turn {n} did not complete within 5s");
    }

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    // Replay and collect all events.
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };

    // Collect all TurnStarted seqs for boundary checks.
    let turn_started_seqs: Vec<u64> = persisted
        .iter()
        .filter_map(|e| {
            if matches!(e.payload, EventPayload::TurnStarted { .. }) {
                Some(e.seq)
            } else {
                None
            }
        })
        .collect();

    // -- Assertion 1: at least one ContextCompacted event must exist --
    let compacted_events: Vec<_> = persisted
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::ContextCompacted { .. }))
        .collect();

    assert!(
        !compacted_events.is_empty(),
        "expected at least one ContextCompacted event over {TURNS} turns with \
         Absolute(1_000) threshold; total events in log: {}",
        persisted.len()
    );

    // -- Assertion 2: every ContextCompacted has replacement = Drop --
    for ev in &compacted_events {
        match &ev.payload {
            EventPayload::ContextCompacted { replacement, .. } => {
                assert!(
                    matches!(replacement, CompactionReplacement::Drop),
                    "TruncateCompactor must produce Drop replacement, got {replacement:?}"
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    // -- Assertion 3: every ContextCompacted.replaced_seq_range.0 is a TurnStarted seq --
    for ev in &compacted_events {
        match &ev.payload {
            EventPayload::ContextCompacted {
                replaced_seq_range, ..
            } => {
                let start_seq = replaced_seq_range.0;
                assert!(
                    turn_started_seqs.contains(&start_seq),
                    "ContextCompacted.replaced_seq_range.0={start_seq} must match a TurnStarted seq; \
                     TurnStarted seqs: {turn_started_seqs:?}"
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    // -- Assertion 4: at least one ContextDecisionRecorded has non-empty compactions --
    let compacted_event_ids: std::collections::HashSet<_> =
        compacted_events.iter().map(|e| e.event_id).collect();

    let decision_with_compaction = persisted.iter().find(|e| {
        matches!(
            &e.payload,
            EventPayload::ContextDecisionRecorded { compactions, .. }
            if compactions.iter().any(|id| compacted_event_ids.contains(id))
        )
    });

    assert!(
        decision_with_compaction.is_some(),
        "expected at least one ContextDecisionRecorded with non-empty compactions \
         referencing a ContextCompacted event id"
    );

    Ok(())
}

//! `TruncateCompactor` — adaptive sliding-window compactor.
//!
//! Algorithm full spec: ADR-0008 §"`TruncateCompactor` adaptive threshold".
//!
//! Steps:
//! 1. Idempotency: if a `ContextCompacted` for this `turn_id` already exists,
//!    return its descriptor without doing further work.
//! 2. Compute the token threshold from `TruncateConfig.max_tokens`.
//! 3. Estimate current token usage from the last model call or by counting chars.
//!    If estimated usage < threshold, return `vec![]` (no compaction needed).
//! 4. Collect turn boundaries and currently-covered seq ranges.
//! 5. Determine the retain window: `keep_first_user` and `keep_recent_turns`.
//!    If nothing is eligible to be dropped, return `vec![]`.
//! 6. Find the first and last uncovered turn in the droppable window.
//!    If everything droppable is already covered, return `vec![]`.
//! 7. Write a `ContextCompacted` event via the recorder and return a
//!    `CompactionApplied` descriptor.

use async_trait::async_trait;
use cogito_protocol::context::{
    CompactionApplied, CompactionInput, CompactionKind, CompactionReplacement, Compactor,
    ContextError, TokenEstimates, TokenThreshold, TruncateConfig,
};
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::ids::TurnId;
use cogito_protocol::store::EventRecorder;

/// Sliding-window truncation compactor.
///
/// Drops the oldest droppable turns from the history when the estimated token
/// count exceeds the configured `max_tokens` threshold. Preserves
/// `keep_first_user` (the first turn) and the `keep_recent_turns` most-recent
/// completed turns unconditionally.
#[derive(Clone, Debug)]
pub struct TruncateCompactor {
    config: TruncateConfig,
}

impl TruncateCompactor {
    /// Construct a new `TruncateCompactor` with the given config.
    #[must_use]
    pub fn new(config: TruncateConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Compactor for TruncateCompactor {
    async fn maybe_compact(
        &self,
        input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError> {
        // Step 1: idempotency — if already compacted for this turn, return descriptor.
        for ev in input.history {
            if let EventPayload::ContextCompacted {
                turn_id,
                replaced_seq_range,
                ..
            } = &ev.payload
            {
                if *turn_id == input.turn_id {
                    return Ok(vec![CompactionApplied::new(
                        ev.event_id,
                        *replaced_seq_range,
                        CompactionKind::Truncate,
                    )]);
                }
            }
        }

        // Step 2: compute effective token threshold.
        let max_tokens = match &self.config.max_tokens {
            TokenThreshold::Absolute(n) => *n,
            TokenThreshold::Ratio {
                of_context_window,
                safety_headroom,
            } => {
                let limits = input.model_gateway.model_limits();
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let raw =
                    (limits.context_window_tokens as f64 * f64::from(*of_context_window)) as u64;
                raw.saturating_sub(*safety_headroom)
            }
            // Non-exhaustive: treat unknown variants as no-threshold (never compact).
            _ => return Ok(vec![]),
        };

        // Step 3: estimate current token usage; skip compaction if under threshold.
        let estimated = input.last_usage.as_ref().map_or_else(
            || estimate_visible_tokens(input.history),
            |u| u64::from(u.input_tokens),
        );
        if estimated < max_tokens {
            return Ok(vec![]);
        }

        // Step 4: collect turn boundaries and existing covered ranges.
        let turn_boundaries = collect_turn_boundaries(input.history);
        let covered = collect_covered_ranges(input.history);

        // Step 5: determine drop window bounds within `turn_boundaries`.
        let total = turn_boundaries.len();
        let first_keep_idx = usize::from(self.config.keep_first_user);
        let last_keep_idx = total.saturating_sub(self.config.keep_recent_turns as usize);
        if first_keep_idx >= last_keep_idx {
            return Ok(vec![]);
        }

        // Step 6: find first/last uncovered turn in `[first_keep_idx, last_keep_idx)`.
        let mut drop_start_seq: Option<u64> = None;
        let mut drop_end_seq: Option<u64> = None;
        for (_tid, turn_start, turn_end) in turn_boundaries
            .iter()
            .take(last_keep_idx)
            .skip(first_keep_idx)
        {
            let fully_covered = (*turn_start..=*turn_end).all(|s| is_covered(s, &covered));
            if !fully_covered {
                if drop_start_seq.is_none() {
                    drop_start_seq = Some(*turn_start);
                }
                drop_end_seq = Some(*turn_end);
            }
        }
        let (Some(start), Some(end)) = (drop_start_seq, drop_end_seq) else {
            return Ok(vec![]);
        };

        // Step 7: persist a `ContextCompacted` event and return the descriptor.
        let token_estimates = TokenEstimates {
            before: Some(estimated),
            after: Some(estimated.saturating_sub(estimate_dropped_tokens(
                input.history,
                start,
                end,
            ))),
        };
        let event_id = EventRecorder::record_context_compacted(
            input.recorder,
            input.turn_id,
            (start, end),
            "truncate",
            CompactionReplacement::Drop,
            token_estimates,
        )
        .await
        .map_err(ContextError::Storage)?;

        Ok(vec![CompactionApplied::new(
            event_id,
            (start, end),
            CompactionKind::Truncate,
        )])
    }

    fn id(&self) -> &'static str {
        "truncate"
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// One covered interval (inclusive on both ends).
#[derive(Clone, Copy)]
struct SeqRange {
    start: u64,
    end: u64,
}

/// Build the list of all `ContextCompacted.replaced_seq_range` intervals found
/// in `events`. Each interval represents a previously-compacted range.
fn collect_covered_ranges(events: &[ConversationEvent]) -> Vec<SeqRange> {
    let mut ranges = Vec::new();
    for event in events {
        if let EventPayload::ContextCompacted {
            replaced_seq_range, ..
        } = &event.payload
        {
            ranges.push(SeqRange {
                start: replaced_seq_range.0,
                end: replaced_seq_range.1,
            });
        }
    }
    ranges
}

/// Returns `true` if `seq` falls inside any covered range.
fn is_covered(seq: u64, covered: &[SeqRange]) -> bool {
    covered.iter().any(|r| seq >= r.start && seq <= r.end)
}

/// Collect turn boundaries from `events`.
///
/// Each entry is `(turn_id, start_seq, end_seq)` where `start_seq` is the seq
/// of the `TurnStarted` event and `end_seq` is the seq of the last event that
/// belongs to the same turn before the next `TurnStarted` or
/// `ContextManageEntered` event.
///
/// Only turns whose `TurnStarted` seq can be determined are included (i.e.
/// turns that have an explicit `TurnStarted` event in `events`).
fn collect_turn_boundaries(events: &[ConversationEvent]) -> Vec<(TurnId, u64, u64)> {
    // Collect all TurnStarted positions with their seq numbers.
    let starts: Vec<(TurnId, u64)> = events
        .iter()
        .filter_map(|ev| {
            if let (EventPayload::TurnStarted { .. }, Some(tid)) = (&ev.payload, ev.turn_id) {
                Some((tid, ev.seq))
            } else {
                None
            }
        })
        .collect();

    if starts.is_empty() {
        return Vec::new();
    }

    // For each TurnStarted, find the seq of the last event before the next
    // TurnStarted or ContextManageEntered (which marks the end of this turn).
    let mut boundaries = Vec::with_capacity(starts.len());
    for i in 0..starts.len() {
        let (tid, start_seq) = starts[i];
        // The next turn boundary is either the next TurnStarted seq or the
        // next ContextManageEntered seq, whichever comes first.
        let next_boundary_seq = if i + 1 < starts.len() {
            // Look for the next ContextManageEntered between this TurnStarted
            // and the next TurnStarted.
            let next_turn_seq = starts[i + 1].1;
            let next_context_manage_seq = events
                .iter()
                .find(|ev| {
                    ev.seq > start_seq
                        && ev.seq < next_turn_seq
                        && matches!(ev.payload, EventPayload::ContextManageEntered { .. })
                })
                .map(|ev| ev.seq);
            next_context_manage_seq.unwrap_or(next_turn_seq)
        } else {
            // Last turn: look for the next ContextManageEntered after TurnStarted.
            events
                .iter()
                .find(|ev| {
                    ev.seq > start_seq
                        && matches!(ev.payload, EventPayload::ContextManageEntered { .. })
                })
                .map_or(u64::MAX, |ev| ev.seq)
        };

        // The end_seq is the largest seq that belongs to this turn — the event
        // just before the next boundary, if any, or the last event overall.
        let end_seq = events
            .iter()
            .filter(|ev| ev.seq > start_seq && ev.seq < next_boundary_seq)
            .map(|ev| ev.seq)
            .max()
            .unwrap_or(start_seq);

        boundaries.push((tid, start_seq, end_seq));
    }

    boundaries
}

/// Estimate the number of tokens in visible (non-covered) events.
///
/// Uses a naive `chars / 4` heuristic. This is only used when `last_usage` is
/// unavailable (e.g. the very first turn).
fn estimate_visible_tokens(events: &[ConversationEvent]) -> u64 {
    let covered = collect_covered_ranges(events);
    let total_chars: usize = events
        .iter()
        .filter(|ev| !is_covered(ev.seq, &covered))
        .map(event_char_count)
        .sum();
    (total_chars / 4) as u64
}

/// Estimate the token count for events in the range `[start, end]`.
///
/// Used to produce an `after` estimate: `before - dropped_estimate`.
fn estimate_dropped_tokens(events: &[ConversationEvent], start: u64, end: u64) -> u64 {
    let total_chars: usize = events
        .iter()
        .filter(|ev| ev.seq >= start && ev.seq <= end)
        .map(event_char_count)
        .sum();
    (total_chars / 4) as u64
}

/// Approximate char count for an event payload. Used for token estimation.
fn event_char_count(event: &ConversationEvent) -> usize {
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::tool::ToolResult;

    match &event.payload {
        EventPayload::TurnStarted { user_input, .. } => user_input
            .iter()
            .map(|b| match b {
                ContentBlock::Text { text } => text.len(),
                _ => 0,
            })
            .sum(),
        EventPayload::AssistantMessageAppended { text, .. } => text.len(),
        EventPayload::ToolUseRecorded {
            args, tool_name, ..
        } => tool_name.len() + args.to_string().len(),
        EventPayload::ToolResultRecorded { result, .. } => match result {
            ToolResult::Output(vals) => vals.iter().map(|v| v.to_string().len()).sum(),
            ToolResult::Error { message, .. } => message.len(),
            _ => 0,
        },
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use chrono::Utc;
    use cogito_protocol::context::{CompactionKind, Compactor, TokenThreshold, TruncateConfig};
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;
    use cogito_test_fixtures::context::{DummyGateway, InMemoryRecorder};

    use super::*;

    #[tokio::test]
    async fn truncate_no_op_when_no_history() {
        let strategy = HarnessStrategy::default_with_model("test");
        let gateway = DummyGateway;
        let mut recorder = InMemoryRecorder::default();
        let current_turn = TurnId::new();
        let input = CompactionInput {
            session_id: SessionId::new(),
            turn_id: current_turn,
            history: &[],
            strategy: &strategy,
            last_usage: None,
            model_gateway: &gateway,
            recorder: &mut recorder,
        };
        let config = TruncateConfig {
            max_tokens: TokenThreshold::Absolute(100),
            keep_first_user: true,
            keep_recent_turns: 2,
        };
        let result = TruncateCompactor::new(config)
            .maybe_compact(input)
            .await
            .unwrap();
        assert!(
            result.is_empty(),
            "empty history must return vec![], got: {result:?}"
        );
        assert!(
            recorder.events.is_empty(),
            "no events must be written for empty history"
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
                replacement: cogito_protocol::context::CompactionReplacement::Drop,
                token_estimate_before: Some(1000),
                token_estimate_after: Some(200),
            },
        }];

        let strategy = HarnessStrategy::default_with_model("test");
        let gateway = DummyGateway;
        let mut recorder = InMemoryRecorder::default();
        let input = CompactionInput {
            session_id,
            turn_id: current_turn,
            history: &history,
            strategy: &strategy,
            last_usage: None,
            model_gateway: &gateway,
            recorder: &mut recorder,
        };

        let config = TruncateConfig {
            max_tokens: TokenThreshold::Absolute(100),
            keep_first_user: true,
            keep_recent_turns: 2,
        };
        let result = TruncateCompactor::new(config)
            .maybe_compact(input)
            .await
            .unwrap();

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
            "idempotency path must not write new events"
        );
    }
}

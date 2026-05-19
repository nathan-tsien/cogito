//! `append_throughput` — measures `JsonlStore::append` latency and
//! throughput for the v0.1 dev-grade baseline. Results inform
//! `docs/quality/v0.1-jsonl-baseline.md`. **Not** a production SLO
//! lock — see ADR-0005 §3 footnote and ADR-0007.

// Criterion benches mint fresh resources per measurement and surface
// setup failures as test failures rather than runtime panics; the
// workspace `expect_used` deny applies to library/binary code, not to
// benchmark fixtures, so we opt out for this file. We also opt out of
// `missing_docs` because the `criterion_group!`/`criterion_main!` macros
// expand to undocumented harness items.
#![allow(clippy::expect_used)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::Utc;
use cogito_protocol::{
    ConversationEvent, ConversationStore, EventId, EventPayload, SCHEMA_VERSION, SessionId, TurnId,
};
use cogito_store_jsonl::JsonlStore;
use criterion::{Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;

fn build_event(session_id: SessionId, turn_id: TurnId, seq: u64) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload: EventPayload::ToolUseRecorded {
            call_id: "toolu_bench".into(),
            tool_name: "noop_bench".into(),
            args: serde_json::json!({
                "param_a": "value-with-some-bytes",
                "param_b": 42,
                "param_c": [1, 2, 3, 4, 5, 6, 7, 8],
            }),
        },
    }
}

fn bench_append(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    c.bench_function("jsonl_append_single_event", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let tmp = tempfile::tempdir().expect("tmp dir");
            let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path()));
            let sid = SessionId::new();
            let tid = TurnId::new();
            let start = std::time::Instant::now();
            for seq in 0..iters {
                let event = build_event(sid, tid, seq);
                store.append(&event).await.expect("append");
            }
            start.elapsed()
        });
    });
}

criterion_group!(benches, bench_append);
criterion_main!(benches);

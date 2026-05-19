//! Shared `ConversationStore` contract test suite.
//!
//! Every `ConversationStore` implementation MUST pass `run_store_contract`.
//! Backend integration tests look like:
//!
//! ```ignore
//! #[tokio::test]
//! async fn jsonl_passes_store_contract() {
//!     let tmp = tempfile::tempdir().unwrap();
//!     let root = tmp.path().to_path_buf();
//!     cogito_test_fixtures::store_contract::run_store_contract(move || {
//!         let root = root.clone();
//!         async move { Arc::new(JsonlStore::new(root)) as Arc<dyn ConversationStore> }
//!     })
//!     .await;
//! }
//! ```
//!
//! Sub-tests use disjoint `SessionId`s so the same store instance can be
//! reused across the full suite without interference.

#![allow(clippy::expect_used)]
// Justification: this module is test infrastructure. Sub-tests propagate
// errors via `?`; the top-level driver `.expect()`s once per sub-test so
// contract violations surface as panics with a clear label. Test fixtures
// are the canonical place to deviate from the workspace's deny-`expect`
// posture, since contract failure here MUST stop the harness.

use std::future::Future;
use std::sync::Arc;

use chrono::Utc;
use cogito_protocol::{
    ContentBlock, ConversationEvent, ConversationStore, EventId, EventPayload, SCHEMA_VERSION,
    SessionId, SessionMeta, TurnId,
};
use futures::StreamExt;

/// Boxed dynamic error returned by individual sub-tests; the driver
/// converts these into panics with a sub-test label.
type SubtestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Build a `SessionStarted` event for `session_id` at the given `seq`.
#[must_use]
pub fn session_started_event(session_id: SessionId, seq: u64) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: None,
        seq,
        ts: Utc::now(),
        payload: EventPayload::SessionStarted {
            meta: SessionMeta {
                cogito_version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
        },
    }
}

/// Build a `TurnStarted` event carrying one text user input.
#[must_use]
pub fn turn_started_event(
    session_id: SessionId,
    turn_id: TurnId,
    seq: u64,
    text: &str,
) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload: EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: text.into() }],
        },
    }
}

/// Run the full contract suite against the store produced by `make_store`.
///
/// `make_store` is called once at the start; the returned
/// `Arc<dyn ConversationStore>` is reused across every sub-test, so the
/// backend MUST tolerate state from earlier sub-tests (sub-tests use
/// disjoint `SessionId`s to avoid interference).
///
/// # Panics
///
/// Panics with the sub-test name when a contract assertion fails or
/// when a sub-test propagates an error via `?`.
///
/// # Note on the `F: Fn() -> Fut, Fut: Future<...>` bound
///
/// The plan specifies `F: AsyncFn() -> Arc<dyn ConversationStore>`.
/// `AsyncFn` is unstable as a user-facing trait bound on Rust 1.85, so
/// we use the explicit `Fn() -> Fut` shape, which is fully stable and
/// equally ergonomic at the call site.
pub async fn run_store_contract<F, Fut>(make_store: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = Arc<dyn ConversationStore>>,
{
    let store = make_store().await;
    test_append_then_latest_seq(&*store)
        .await
        .expect("test_append_then_latest_seq");
    test_append_then_replay_full(&*store)
        .await
        .expect("test_append_then_replay_full");
    test_append_then_replay_from_offset(&*store)
        .await
        .expect("test_append_then_replay_from_offset");
    test_replay_empty_session_returns_empty_stream(&*store)
        .await
        .expect("test_replay_empty_session_returns_empty_stream");
    test_latest_seq_empty_session_returns_none(&*store)
        .await
        .expect("test_latest_seq_empty_session_returns_none");
    test_multiple_sessions_isolated(&*store)
        .await
        .expect("test_multiple_sessions_isolated");
    test_close_then_reappend(&*store)
        .await
        .expect("test_close_then_reappend");
    test_concurrent_append_two_sessions(&*store)
        .await
        .expect("test_concurrent_append_two_sessions");
}

async fn test_append_then_latest_seq(store: &dyn ConversationStore) -> SubtestResult {
    let sid = SessionId::new();
    store.append(&session_started_event(sid, 0)).await?;
    store
        .append(&turn_started_event(sid, TurnId::new(), 1, "hi"))
        .await?;
    let last = store.latest_seq(sid).await?;
    assert_eq!(last, Some(1), "latest_seq after two appends");
    Ok(())
}

async fn test_append_then_replay_full(store: &dyn ConversationStore) -> SubtestResult {
    const N: u64 = 5;
    let sid = SessionId::new();
    for seq in 0..N {
        let event = if seq == 0 {
            session_started_event(sid, seq)
        } else {
            turn_started_event(sid, TurnId::new(), seq, "x")
        };
        store.append(&event).await?;
    }
    let stream = store.replay(sid, 0);
    let collected: Vec<_> = stream.collect().await;
    // from_seq = 0 means events with seq > 0; one fewer than appended.
    assert_eq!(
        collected.len(),
        usize::try_from(N - 1)?,
        "replay(from=0) should return events where seq > 0",
    );
    for r in collected {
        // Surface any backend error so the contract fails loudly.
        let _event = r?;
    }
    Ok(())
}

async fn test_append_then_replay_from_offset(store: &dyn ConversationStore) -> SubtestResult {
    let sid = SessionId::new();
    for seq in 0..5_u64 {
        store
            .append(&turn_started_event(sid, TurnId::new(), seq, "x"))
            .await?;
    }
    let stream = store.replay(sid, 2);
    let collected: Vec<_> = stream.collect().await;
    assert_eq!(
        collected.len(),
        2,
        "replay(from=2) should return events where seq > 2 (i.e. 3, 4)",
    );
    Ok(())
}

async fn test_replay_empty_session_returns_empty_stream(
    store: &dyn ConversationStore,
) -> SubtestResult {
    let sid = SessionId::new();
    let stream = store.replay(sid, 0);
    let collected: Vec<_> = stream.collect().await;
    assert!(
        collected.is_empty(),
        "replay of unknown session should be empty",
    );
    Ok(())
}

async fn test_latest_seq_empty_session_returns_none(
    store: &dyn ConversationStore,
) -> SubtestResult {
    let sid = SessionId::new();
    let last = store.latest_seq(sid).await?;
    assert_eq!(last, None);
    Ok(())
}

async fn test_multiple_sessions_isolated(store: &dyn ConversationStore) -> SubtestResult {
    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    store.append(&session_started_event(sid_a, 0)).await?;
    store
        .append(&turn_started_event(sid_a, TurnId::new(), 1, "a1"))
        .await?;
    store.append(&session_started_event(sid_b, 0)).await?;
    let a_last = store.latest_seq(sid_a).await?;
    let b_last = store.latest_seq(sid_b).await?;
    assert_eq!(a_last, Some(1));
    assert_eq!(b_last, Some(0));
    Ok(())
}

async fn test_close_then_reappend(store: &dyn ConversationStore) -> SubtestResult {
    let sid = SessionId::new();
    store.append(&session_started_event(sid, 0)).await?;
    store.close(sid).await?;
    store
        .append(&turn_started_event(sid, TurnId::new(), 1, "after-close"))
        .await?;
    let last = store.latest_seq(sid).await?;
    assert_eq!(last, Some(1));
    Ok(())
}

async fn test_concurrent_append_two_sessions(store: &dyn ConversationStore) -> SubtestResult {
    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    let n: u64 = 50;
    let store_a = store;
    let store_b = store;
    let task_a = async {
        for seq in 0..n {
            store_a
                .append(&turn_started_event(sid_a, TurnId::new(), seq, "a"))
                .await?;
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };
    let task_b = async {
        for seq in 0..n {
            store_b
                .append(&turn_started_event(sid_b, TurnId::new(), seq, "b"))
                .await?;
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };
    let (res_a, res_b) = tokio::join!(task_a, task_b);
    res_a?;
    res_b?;
    assert_eq!(store.latest_seq(sid_a).await?, Some(n - 1));
    assert_eq!(store.latest_seq(sid_b).await?, Some(n - 1));
    Ok(())
}

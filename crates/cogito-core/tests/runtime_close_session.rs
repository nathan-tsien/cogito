//! ADR-0034 — Runtime session-registry lifecycle: `get_session` /
//! `close_session`. The `sessions` registry is insert-only today, so a
//! session that was opened can never be reopened within the same `Runtime`
//! (it hits `SessionAlreadyOpen`). These tests pin the deregistration verb,
//! its idempotency, the live-handle lookup, and the store-resource release
//! on actor exit (Option A).

#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_store::JsonlStore;
use cogito_test_fixtures::FaultInjectingStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};

const DEADLINE: Duration = Duration::from_secs(5);

fn build_runtime(store: Arc<dyn ConversationStore>) -> Arc<Runtime> {
    let mock = Arc::new(MockModelGateway::new());
    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );
    Runtime::builder()
        .store(store)
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()
        .expect("runtime builds")
}

/// The headline regression: open a session, close it, then resume it within
/// the SAME `Runtime`. Today the second open hits `SessionAlreadyOpen`.
#[tokio::test]
async fn close_then_reopen_resume_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(store);

    let id = SessionId::new();
    // Open fresh — writes SessionStarted (seq 0) and registers the handle.
    let _h = runtime
        .open_session(id, OpenMode::New)
        .await
        .expect("open New");

    let outcome = runtime.close_session(id, DEADLINE).await.expect("close ok");
    assert!(
        outcome.is_some(),
        "closing a live session returns its ShutdownOutcome"
    );

    // The slot is free again: the same id can be resumed.
    let reopened = runtime.open_session(id, OpenMode::Resume).await;
    assert!(
        reopened.is_ok(),
        "resume after close should succeed, got: {:?}",
        reopened
            .as_ref()
            .map(|_| "<SessionHandle>")
            .map_err(std::string::ToString::to_string),
    );
}

/// `close_session` on an id that was never opened is a no-op, not an error.
#[tokio::test]
async fn close_session_unknown_id_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(store);

    let outcome = runtime
        .close_session(SessionId::new(), DEADLINE)
        .await
        .expect("close ok");
    assert!(
        outcome.is_none(),
        "closing a never-opened id returns Ok(None)"
    );
}

/// Closing twice: the first drives shutdown, the second is a no-op.
#[tokio::test]
async fn close_session_twice_returns_none_second_time() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(store);

    let id = SessionId::new();
    let _h = runtime
        .open_session(id, OpenMode::New)
        .await
        .expect("open New");

    let first = runtime
        .close_session(id, DEADLINE)
        .await
        .expect("first close ok");
    assert!(first.is_some(), "first close drives shutdown");

    let second = runtime
        .close_session(id, DEADLINE)
        .await
        .expect("second close ok");
    assert!(second.is_none(), "second close is a no-op (idempotent)");
}

/// `get_session` returns the live handle while registered and `None` after
/// it is closed (and for an id that was never opened).
#[tokio::test]
async fn get_session_returns_live_handle_then_none_after_close() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(store);

    let id = SessionId::new();
    assert!(
        runtime.get_session(id).is_none(),
        "an id that was never opened has no live handle"
    );

    let _h = runtime
        .open_session(id, OpenMode::New)
        .await
        .expect("open New");
    assert!(
        runtime.get_session(id).is_some(),
        "a live session returns its handle"
    );

    runtime.close_session(id, DEADLINE).await.expect("close ok");
    assert!(
        runtime.get_session(id).is_none(),
        "a closed session is no longer registered"
    );
}

/// Option A: the actor releases per-session store resources on exit, so a
/// `close_session` that frees the registry slot also frees the store's
/// file handle / connection slot. `drain_shutdown` does not flush/close the
/// store today — this pins the new behavior.
#[tokio::test]
async fn close_session_releases_store_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let inner = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let spy: Arc<FaultInjectingStore<JsonlStore>> = Arc::new(FaultInjectingStore::new(inner));
    let store: Arc<dyn ConversationStore> = spy.clone();
    let runtime = build_runtime(store);

    let id = SessionId::new();
    let _h = runtime
        .open_session(id, OpenMode::New)
        .await
        .expect("open New");
    assert_eq!(spy.close_count(), 0, "no store close before shutdown");

    runtime.close_session(id, DEADLINE).await.expect("close ok");
    assert!(
        spy.close_count() >= 1,
        "actor must release store resources on exit (ADR-0034 Option A); close_count={}",
        spy.close_count(),
    );
}

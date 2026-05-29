//! P4.3 — `Runtime::open_session` must dispatch by `OpenMode`.

#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

use std::sync::Arc;

use cogito_core::runtime::{OpenMode, Runtime, RuntimeError};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore as _;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_store::JsonlStore;
use cogito_test_fixtures::canonical_sample_session;
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn build_runtime_with_store(store: Arc<JsonlStore>) -> Arc<Runtime> {
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

#[tokio::test]
async fn open_session_resume_missing_returns_resume_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime_with_store(store);

    let id = SessionId::new();
    let result = runtime.open_session(id, OpenMode::Resume).await;

    assert!(
        matches!(result, Err(RuntimeError::ResumeFailed { id: e_id, .. }) if e_id == id),
        "expected ResumeFailed with id={id:?}, got: {:?}",
        result
            .as_ref()
            .map(|_| "<SessionHandle>")
            .map_err(std::string::ToString::to_string),
    );
}

#[tokio::test]
async fn open_session_new_with_existing_log_returns_session_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    // Pre-populate the store with the canonical fixture (SessionStarted at seq=0).
    let events = canonical_sample_session();
    let fixture_id = events[0].session_id;
    store.append(&events[0]).await.unwrap();

    let runtime = build_runtime_with_store(store);
    let result = runtime.open_session(fixture_id, OpenMode::New).await;

    assert!(
        matches!(result, Err(RuntimeError::SessionAlreadyExists { id: e_id }) if e_id == fixture_id),
        "expected SessionAlreadyExists with id={fixture_id:?}, got: {:?}",
        result
            .as_ref()
            .map(|_| "<SessionHandle>")
            .map_err(std::string::ToString::to_string),
    );
}

#[tokio::test]
async fn open_session_attach_with_empty_log_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime_with_store(store);

    let id = SessionId::new();
    let result = runtime.open_session(id, OpenMode::Attach).await;

    assert!(
        result.is_ok(),
        "Attach with empty store should succeed: {:?}",
        result
            .as_ref()
            .map(|_| "<SessionHandle>")
            .map_err(std::string::ToString::to_string),
    );
}

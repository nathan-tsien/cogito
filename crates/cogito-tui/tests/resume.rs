//! Resume test: read canonical JSONL -> translate -> drive into
//! `ChatModel` + `ToolTreeModel` -> assert expected scrollback.
//!
//! Two tests:
//! - `replay_canonical_fixture_reconstructs_chat`: exercises the full
//!   `load_initial_state` path by writing canonical JSONL bytes to a
//!   tempdir and calling the async resume entry point.
//! - `translate_events_handles_canonical_shape_synchronously`: a cheaper
//!   smoke-check without a tokio runtime.
//!
//! Both verify the spec §4.6 invariant: state must be regeneratable from
//! the JSONL event log (no cross-turn state in structs).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_tui::render_model::{ChatLine, ChatModel, ToolTreeModel};
use cogito_tui::resume::{InitialState, load_initial_state, translate_events};

/// Write the canonical fixture bytes to a tempdir as
/// `{session_id}.jsonl`, then call `load_initial_state` and assert the
/// resumed state contains at least one `AssistantText` chat line.
#[tokio::test(flavor = "current_thread")]
async fn replay_canonical_fixture_reconstructs_chat() {
    // Build canonical events and serialize to JSONL bytes.
    let events = cogito_test_fixtures::fixtures::canonical_sample_session();
    let jsonl_bytes = cogito_test_fixtures::fixtures::canonical_sample_jsonl();

    // Extract the session_id from the first event so we can name the file
    // correctly. JsonlStore expects `<root>/<session_id>.jsonl`.
    let session_id = events[0].session_id;

    // Write the JSONL bytes to a tempdir under the expected filename.
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join(format!("{session_id}.jsonl"));
    std::fs::write(&file_path, &jsonl_bytes).unwrap();

    // Open a JsonlStore rooted at the tempdir.
    let store: Arc<dyn cogito_protocol::ConversationStore> =
        Arc::new(cogito_store::JsonlStore::new(dir.path()));

    // Call the resume entry point. `is_new_session = false` triggers
    // the replay path.
    let state = load_initial_state(&store, &session_id, false)
        .await
        .unwrap();

    let stream_events = match state {
        InitialState::Replayed { stream_events } => stream_events,
        InitialState::Fresh => panic!("expected Replayed, got Fresh"),
    };
    assert!(
        !stream_events.is_empty(),
        "canonical fixture should produce at least 1 stream event"
    );

    // Drive events into both models (same code path as the live loop).
    let mut chat = ChatModel::new();
    let mut tools = ToolTreeModel::new();
    for ev in &stream_events {
        chat.on_event(ev);
        tools.on_event(ev);
    }

    // The canonical fixture includes AssistantMessageAppended with
    // "Reading /tmp/x now.", which translate_events maps to TextDelta,
    // which ChatModel stores as AssistantText.
    assert!(
        chat.lines
            .iter()
            .any(|l| matches!(l, ChatLine::AssistantText(_))),
        "expected at least one AssistantText line; got: {:?}",
        chat.lines
    );
}

/// Cheaper synchronous sanity-check: translate the canonical events
/// directly without a tokio runtime or JSONL I/O.
#[test]
fn translate_events_handles_canonical_shape_synchronously() {
    let events = cogito_test_fixtures::fixtures::canonical_sample_session();
    let translated = translate_events(&events);
    assert!(
        !translated.is_empty(),
        "canonical fixture must produce a non-empty stream-event list"
    );
}

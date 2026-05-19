//! `store_writer` stub. The Sprint 1 `StepRecorder` owns `Arc<dyn
//! ConversationStore>` and writes directly, so no separate store-writer
//! subtask is needed in v0.1. A dedicated subtask (with batching and
//! back-pressure) is tracked for Sprint 4+ in the ROADMAP.
//!
//! This module is intentionally empty; it is retained in the tree so
//! the `runtime` mod declaration does not need to be removed.

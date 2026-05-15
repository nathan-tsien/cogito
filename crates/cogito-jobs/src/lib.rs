//! cogito-jobs
//!
//! Async job manager. Submitted jobs run as `tokio::task`s, with state
//! persisted to SQLite for resume after crashes.

//! `cogito-store` — Session-layer `ConversationStore` backends.
//!
//! The crate is named after its **role** (the Session store), not a
//! backend. Concrete backends live as feature-gated modules so adding
//! a new one (Postgres in v0.4, `SQLite` later) is a feature flag plus a
//! module, not a new workspace crate. See ADR-0024.
//!
//! v0.1 ships a single backend, [`jsonl`], enabled by the default
//! `jsonl` feature. [`JsonlStore`] is re-exported at the crate root for
//! convenience; `cogito_store::jsonl::JsonlStore` reaches the same type.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

#[cfg(feature = "jsonl")]
pub mod jsonl;

#[cfg(feature = "jsonl")]
pub use jsonl::JsonlStore;

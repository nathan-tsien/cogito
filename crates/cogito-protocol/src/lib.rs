//! cogito-protocol
//!
//! Protocol layer: events, contracts, and types shared across the workspace.
//!
//! This crate is dependency-free with respect to other cogito crates.
//! Anything that defines the *contract* between components belongs here.
//!
//! Module map (1:1 with the Brain/Hands/Session boundaries in ADR-0004):
//! - [`content`]: `ContentBlock` — wire-format unit shared between model, tools, persisted events
//! - [`tool`]: `ToolProvider` trait, `ToolDescriptor`, `InvokeOutcome`, `ExecutionClass`
//! - [`stream`]: `StreamEvent` enum (real-time fanout to subscribers)
//! - [`job`]: `JobManager` trait, `JobId`, `JobStatus`, `JobCompletionEvent`
//! - [`session`]: `SessionMeta` — per-session pass-through metadata
//! - [`turn`]: `TurnOutcome`, `TurnFailureReason`
//! - [`error`]: shared error kinds and helpers
//! - [`ids`]: strongly-typed ULID newtypes (`EventId`, `SessionId`, `TurnId`)
//!
//! All v0.1 contract modules ship as part of Sprint 0 (Tasks 7-10 of
//! the Sprint 0 closure plan).

pub mod content;
pub mod error;
pub mod ids;
pub mod job;
pub mod session;
pub mod stream;
pub mod tool;
pub mod turn;

pub use content::ContentBlock;
pub use ids::{EventId, SessionId, TurnId};
pub use session::SessionMeta;

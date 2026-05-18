//! cogito-protocol
//!
//! Protocol layer: events, contracts, and types shared across the workspace.
//!
//! This crate is dependency-free with respect to other cogito crates.
//! Anything that defines the *contract* between components belongs here.
//!
//! Module map (1:1 with the Brain/Hands/Session boundaries in ADR-0004):
//! - [`tool`]: `ToolProvider` trait, `ToolDescriptor`, `InvokeOutcome`, `ExecutionClass`
//! - [`stream`]: `StreamEvent` enum (real-time fanout to subscribers)
//! - [`job`]: `JobManager` trait, `JobId`, `JobStatus`, `JobCompletionEvent`
//! - [`turn`]: `TurnOutcome`, `TurnFailureReason`
//! - [`error`]: shared error kinds and helpers
//!
//! Modules `tool`, `stream`, `job`, `turn` land in Tasks 7-10 of the
//! Sprint 0 closure plan; their declarations are commented out until
//! the corresponding implementation tasks ship.

pub mod error;
pub mod job;
// pub mod stream;
pub mod tool;
// pub mod turn;

//! Runtime layer: hosts per-session loops (one tokio task per session, see
//! [`session_loop::run_session`]), owns the tokio `Handle`, and injects
//! Hands/Boundary/Session into the Brain.
//!
//! The per-session loop implements the actor model (ADR-0006). We keep
//! `session_loop` in the public surface to describe what the code does;
//! "actor" remains the conceptual term inside docs where the invariants
//! (private state, message-driven, single mutator, cooperative
//! termination) need to be cited.
//!
//! See:
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
//!   §3 (task topology), §4 (lifecycle), §7 (channels)
//! - ADR-0004 (layer rules)
//! - ADR-0006 (Runtime + H01 execution model — actor invariants)

pub mod builder;
pub mod handle;
pub mod session_loop;
pub mod store_writer;
pub mod types;

pub use builder::{Runtime, RuntimeBuilder, RuntimeError};
pub use handle::{SessionError, SessionHandle};
pub use types::{NewMessage, OpenMode, SessionCommand, SessionId, ShutdownOutcome};

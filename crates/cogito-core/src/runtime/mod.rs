//! Runtime layer: hosts `SessionActor` tasks, owns the tokio `Handle`,
//! injects Hands/Boundary/Session into the Brain.
//!
//! See:
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
//!   §3 (task topology), §4 (lifecycle), §7 (channels)
//! - ADR-0004 (layer rules)

pub mod actor;
pub mod builder;
pub mod handle;
pub mod store_writer;
pub mod types;

pub use builder::{Runtime, RuntimeBuilder, RuntimeError};
pub use handle::{SessionError, SessionHandle};
pub use types::{NewMessage, OpenMode, SessionCommand, SessionId, ShutdownOutcome};

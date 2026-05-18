//! Shared error types used across protocol contracts.
//!
//! Per ADR-0005 §4: contracts return structured errors via `thiserror`.
//! Concrete crates may wrap these into their own error enums.

use thiserror::Error;

/// Errors that cross the protocol boundary. Concrete impls may add
/// backend-specific variants by wrapping `ProtocolError` in their own
/// `thiserror` enum.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Caller passed arguments that violate a documented invariant
    /// (e.g., schema mismatch, missing required field).
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    /// A backend resource (store, gateway, sandbox) is unavailable.
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
}

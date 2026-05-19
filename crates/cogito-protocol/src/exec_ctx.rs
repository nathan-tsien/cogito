//! `ExecCtx` — per-invocation context handed to every tool and hook.
//!
//! Brain constructs an `ExecCtx` once per turn (or per dispatch) and hands
//! a clone to each tool / hook call. v0.1 fields are minimal; v0.2 adds
//! `storage: Arc<dyn StorageSystem>` and v0.4 adds `tenant`.
//!
//! See:
//! - `docs/components/H08-tool-dispatcher.md` for the consumer side
//! - ADR-0006 §"Sprint 2 protocol-layer additions" for why
//!   `tokio_util::sync::CancellationToken` is allowed at the protocol layer

use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::ids::{SessionId, TurnId};

/// Per-invocation execution context. Tools and hooks receive this by value
/// and decide whether to honor `deadline` / `cancel`.
#[derive(Debug, Clone)]
pub struct ExecCtx {
    /// Identifies the current session for correlation in logs and metrics.
    pub session_id: SessionId,
    /// Identifies the current turn within the session.
    pub turn_id: TurnId,
    /// Absolute wall-clock deadline. Tools may check `Instant::now() > deadline`
    /// or use `tokio::time::timeout_at`. `None` means "no deadline".
    pub deadline: Option<Instant>,
    /// Cooperative cancellation token. Tools and adapters should listen via
    /// `select!` on `cancel.cancelled()` to abort in-flight work.
    pub cancel: CancellationToken,
}

impl ExecCtx {
    /// Convenience constructor for an open-ended context with a fresh
    /// cancel token.
    #[must_use]
    pub fn open_ended(session_id: SessionId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            turn_id,
            deadline: None,
            cancel: CancellationToken::new(),
        }
    }
}

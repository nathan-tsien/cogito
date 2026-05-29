//! `ExecCtx` — per-invocation context handed to every tool and hook.
//!
//! Brain constructs an `ExecCtx` once per turn (or per dispatch) and hands
//! a clone to each tool / hook call. v0.2 adds `brain_spawner` +
//! `subagent_depth` + `call_id` (ADR-0011); storage moved to v0.5 and
//! `tenant` lands in v0.4.
//!
//! See:
//! - `docs/components/H08-tool-dispatcher.md` for the consumer side
//! - ADR-0006 §"Sprint 2 protocol-layer additions" for why
//!   `tokio_util::sync::CancellationToken` is allowed at the protocol layer

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::ids::{SessionId, TurnId};
use crate::subagent::BrainSpawner;

/// Per-invocation execution context. Tools and hooks receive this by value
/// and decide whether to honor `deadline` / `cancel`.
///
/// `Debug` is hand-written because `brain_spawner` holds a trait object that
/// is not `Debug`; the impl prints only whether a spawner is present.
#[derive(Clone)]
pub struct ExecCtx {
    /// Identifies the current session for correlation in logs and metrics.
    pub session_id: SessionId,
    /// Identifies the current turn within the session.
    pub turn_id: TurnId,
    /// The tool-call id for the current dispatch, set by H08 before
    /// `ToolProvider::invoke`. `None` outside a tool dispatch.
    pub call_id: Option<String>,
    /// Absolute wall-clock deadline. Tools may check `Instant::now() > deadline`
    /// or use `tokio::time::timeout_at`. `None` means "no deadline".
    pub deadline: Option<Instant>,
    /// Cooperative cancellation token. Tools and adapters should listen via
    /// `select!` on `cancel.cancelled()` to abort in-flight work.
    pub cancel: CancellationToken,
    /// Subagent nesting depth of the current session (0 = top-level).
    /// `delegate` opens a child at `subagent_depth + 1`.
    pub subagent_depth: u32,
    /// Recursive Brain spawner (ADR-0011). `Some` when the Runtime wired a
    /// `BrainSpawner`; `None` otherwise (the `delegate` tool then errors).
    pub brain_spawner: Option<Arc<dyn BrainSpawner>>,
}

impl std::fmt::Debug for ExecCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecCtx")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("call_id", &self.call_id)
            .field("deadline", &self.deadline)
            .field("cancel", &self.cancel)
            .field("subagent_depth", &self.subagent_depth)
            .field("brain_spawner", &self.brain_spawner.is_some())
            // Maintenance: list every ExecCtx field above when one is added.
            .finish()
    }
}

impl ExecCtx {
    /// Convenience constructor for an open-ended context with a fresh
    /// cancel token.
    #[must_use]
    pub fn open_ended(session_id: SessionId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            turn_id,
            call_id: None,
            deadline: None,
            cancel: CancellationToken::new(),
            subagent_depth: 0,
            brain_spawner: None,
        }
    }
}

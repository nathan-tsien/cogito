//! Subagent execution seam (ADR-0011, v0.2 S2 minimal).
//!
//! `BrainSpawner` is the layer-rule seam (ADR-0004): Hands cannot import
//! Runtime, so the ability to spawn a child Brain is a Protocol trait that
//! `cogito-core::runtime` implements and injects into every tool via
//! [`crate::ExecCtx::brain_spawner`]. v0.2 ships a single `run_to_completion`
//! that the caller awaits to completion inline (no background job / JobId);
//! v0.3 grows the spawn/wait/cancel lifecycle additively.

use crate::ids::SessionId;

/// Request to run a child agent to completion. `#[non_exhaustive]` so v0.3
/// can add fields (e.g. `handed_tools`) without breaking call sites.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DelegateRequest {
    /// Strategy name to resolve into the child's `HarnessStrategy` (role).
    pub role: String,
    /// The child's first user message — the only parent→child channel.
    pub input: String,
    /// Parent session id, recorded child-side for linkage.
    pub parent_session_id: SessionId,
    /// The `delegate` tool-call id in the parent turn, recorded child-side.
    pub parent_call_id: String,
    /// Parent's subagent depth; the child opens at `parent_depth + 1`.
    pub parent_depth: u32,
}

impl DelegateRequest {
    /// Build a request. Convenience for call sites and tests.
    #[must_use]
    pub fn new(
        role: impl Into<String>,
        input: impl Into<String>,
        parent_session_id: SessionId,
        parent_call_id: impl Into<String>,
        parent_depth: u32,
    ) -> Self {
        Self {
            role: role.into(),
            input: input.into(),
            parent_session_id,
            parent_call_id: parent_call_id.into(),
            parent_depth,
        }
    }
}

/// Failure modes of [`BrainSpawner::run_to_completion`]. The `delegate`
/// tool maps every variant to a `ToolResult::Error` (Inviolable #5).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpawnError {
    /// `role` did not resolve to a known strategy.
    #[error("unknown subagent role `{role}`")]
    UnknownRole {
        /// The unresolved role name.
        role: String,
    },
    /// Opening the child session failed (store/runtime error).
    #[error("failed to open subagent session: {reason}")]
    OpenFailed {
        /// Human-readable cause.
        reason: String,
    },
    /// The child turn ended in a terminal failure.
    #[error("subagent failed: {reason}")]
    ChildFailed {
        /// Human-readable rendering of the child `TurnFailureReason`.
        reason: String,
    },
}

/// The layer-rule seam that lets a tool spawn a child Brain. Implemented by
/// `cogito-core::runtime::Runtime`; injected as `Arc<dyn BrainSpawner>` via
/// `ExecCtx`. Brain and tools see only this trait.
#[async_trait::async_trait]
pub trait BrainSpawner: Send + Sync {
    /// Run a child agent to completion inline — the caller awaits; no
    /// background job or JobId is involved — and return its final assistant
    /// text. The child is an independent top-level session; only the returned
    /// string crosses back to the caller.
    async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct MockSpawner;

    #[async_trait::async_trait]
    impl BrainSpawner for MockSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Ok(format!("ran {} with {}", req.role, req.input))
        }
    }

    #[tokio::test]
    async fn object_safe_and_invocable() {
        let spawner: Arc<dyn BrainSpawner> = Arc::new(MockSpawner);
        let req = DelegateRequest::new("reviewer", "check this", SessionId::new(), "c1", 0);
        let out = spawner.run_to_completion(req).await.expect("mock ok");
        assert_eq!(out, "ran reviewer with check this");
    }
}

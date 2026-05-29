//! `DelegateToolProvider` - the `delegate(role, input) -> output` tool
//! (ADR-0011, v0.2 S2 minimal). Reads `ExecCtx.brain_spawner` and runs a
//! child agent to completion. No `Runtime` reference is held; the spawner
//! arrives per-call via `ExecCtx`.

use cogito_protocol::ExecCtx;
use cogito_protocol::subagent::{DelegateRequest, SpawnError};
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};

/// Tool name exposed to the model.
pub const DELEGATE_TOOL_NAME: &str = "delegate";

/// Default maximum subagent (`delegate`) nesting depth when unconfigured.
pub const DEFAULT_MAX_SUBAGENT_DEPTH: u32 = 3;

/// The `delegate` tool. Construct with [`DelegateToolProvider::new`].
pub struct DelegateToolProvider {
    /// Maximum subagent nesting depth (inclusive guard). Default 3.
    max_depth: u32,
}

impl DelegateToolProvider {
    /// Build with an explicit max depth.
    #[must_use]
    pub fn new(max_depth: u32) -> Self {
        Self { max_depth }
    }
}

impl Default for DelegateToolProvider {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
        }
    }
}

#[derive(serde::Deserialize)]
struct DelegateArgs {
    role: String,
    input: String,
}

fn error(kind: ToolErrorKind, message: String) -> InvokeOutcome {
    InvokeOutcome::Sync(ToolResult::Error {
        kind,
        message,
        retryable: false,
    })
}

#[async_trait::async_trait]
impl ToolProvider for DelegateToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: DELEGATE_TOOL_NAME.to_string(),
            description: "Delegate a self-contained subtask to a child agent \
                identified by `role` (a strategy name). The child starts with \
                a fresh context and sees NONE of this conversation, so pack \
                every file path, fact, and decision it needs into `input`. \
                Returns the child's final message as text."
                .to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "description": "Name of a configured strategy/role for the child agent to run as (resolved by the runtime's strategy registry, e.g. a reviewer or summarizer role)." },
                    "input": { "type": "string", "description": "Self-contained task for the child." }
                },
                "required": ["role", "input"],
                "additionalProperties": false
            }),
            // AlwaysSync: invoke blocks inline until the child completes; the
            // child-drive backstop + the turn deadline are the time guards.
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        if name != DELEGATE_TOOL_NAME {
            return error(
                ToolErrorKind::InvocationFailed,
                format!("unknown tool `{name}`"),
            );
        }
        let parsed: DelegateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return error(
                    ToolErrorKind::InvalidArgs,
                    format!("invalid delegate args: {e}"),
                );
            }
        };
        if ctx.subagent_depth >= self.max_depth {
            return error(
                ToolErrorKind::InvocationFailed,
                format!(
                    "subagent depth limit reached (depth {} >= max {})",
                    ctx.subagent_depth, self.max_depth
                ),
            );
        }
        let Some(spawner) = ctx.brain_spawner.clone() else {
            return error(
                ToolErrorKind::InvocationFailed,
                "subagent delegation is not available (no BrainSpawner wired)".to_string(),
            );
        };
        let call_id = ctx.call_id.clone().unwrap_or_else(|| {
            tracing::warn!(
                "delegate invoked with no call_id in ExecCtx; child parent_call_id will be empty"
            );
            String::new()
        });
        let req = DelegateRequest::new(
            parsed.role,
            parsed.input,
            ctx.session_id,
            call_id,
            ctx.subagent_depth,
        );
        match spawner.run_to_completion(req).await {
            Ok(text) => InvokeOutcome::Sync(ToolResult::text(text)),
            Err(e) => map_spawn_error(&e),
        }
    }
}

fn map_spawn_error(e: &SpawnError) -> InvokeOutcome {
    match e {
        // A child that ran out of time is transient: surface the dedicated
        // Timeout kind and allow the parent strategy to retry.
        SpawnError::Timeout { .. } => InvokeOutcome::Sync(ToolResult::Error {
            kind: ToolErrorKind::Timeout,
            message: e.to_string(),
            retryable: true,
        }),
        // UnknownRole / OpenFailed / ChildFailed are deterministic - retrying
        // with the same args won't help.
        _ => error(ToolErrorKind::InvocationFailed, e.to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::subagent::BrainSpawner;
    use std::sync::Arc;

    struct OkSpawner;
    #[async_trait::async_trait]
    impl BrainSpawner for OkSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Ok(format!("done:{}", req.role))
        }
    }
    struct UnknownRoleSpawner;
    #[async_trait::async_trait]
    impl BrainSpawner for UnknownRoleSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Err(SpawnError::UnknownRole { role: req.role })
        }
    }
    struct TimeoutSpawner;
    #[async_trait::async_trait]
    impl BrainSpawner for TimeoutSpawner {
        async fn run_to_completion(&self, _req: DelegateRequest) -> Result<String, SpawnError> {
            Err(SpawnError::Timeout { seconds: 300 })
        }
    }

    fn ctx_with(depth: u32, spawner: Option<Arc<dyn BrainSpawner>>) -> ExecCtx {
        let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
        c.subagent_depth = depth;
        c.brain_spawner = spawner;
        c.call_id = Some("c1".into());
        c
    }

    fn args(role: &str, input: &str) -> serde_json::Value {
        serde_json::json!({ "role": role, "input": input })
    }

    #[tokio::test]
    async fn happy_path_returns_child_text() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke(
                "delegate",
                args("reviewer", "x"),
                ctx_with(0, Some(Arc::new(OkSpawner))),
            )
            .await;
        match out {
            InvokeOutcome::Sync(ToolResult::Output(v)) => {
                assert_eq!(v, vec![serde_json::Value::String("done:reviewer".into())]);
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn depth_guard_blocks_at_max() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke(
                "delegate",
                args("r", "x"),
                ctx_with(3, Some(Arc::new(OkSpawner))),
            )
            .await;
        assert!(matches!(
            out,
            InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn missing_spawner_errors() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke("delegate", args("r", "x"), ctx_with(0, None))
            .await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { .. })));
    }

    #[tokio::test]
    async fn bad_args_are_invalid_args() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke(
                "delegate",
                serde_json::json!({ "role": "r" }),
                ctx_with(0, Some(Arc::new(OkSpawner))),
            )
            .await;
        assert!(matches!(
            out,
            InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn unknown_role_maps_to_error() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke(
                "delegate",
                args("nope", "x"),
                ctx_with(0, Some(Arc::new(UnknownRoleSpawner))),
            )
            .await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { .. })));
    }

    #[tokio::test]
    async fn unknown_name_errors() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke("not-delegate", args("r", "x"), ctx_with(0, None))
            .await;
        assert!(matches!(
            out,
            InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn timeout_maps_to_retryable_timeout() {
        let p = DelegateToolProvider::new(3);
        let out = p
            .invoke(
                "delegate",
                args("r", "x"),
                ctx_with(0, Some(Arc::new(TimeoutSpawner))),
            )
            .await;
        assert!(matches!(
            out,
            InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::Timeout,
                retryable: true,
                ..
            })
        ));
    }
}

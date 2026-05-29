//! `McpToolProvider` aggregates handles from all successfully-started
//! MCP servers and presents their tools as a single `ToolProvider`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use serde_json::Value;
use tracing::warn;

use crate::client::{CallError, McpServerHandle, call_tool};
use crate::naming::qualify;
use crate::result_mapping::to_cogito_result;

/// Routing entry: a qualified tool name maps to `(server_handle,
/// raw_tool_name)`. The raw name is needed because rmcp's `tools/call`
/// uses the server-internal name, not the qualified one.
struct Route {
    handle: Arc<McpServerHandle>,
    raw_name: String,
}

/// `ToolProvider` aggregating zero or more MCP server handles.
///
/// Constructed by [`crate::factory::build_mcp_provider`]; `routes`
/// and `descriptors` are derived from handshake outputs with the
/// `mcp__server__tool` qualifier applied and within-provider dedup.
pub struct McpToolProvider {
    routes: HashMap<String, Route>,
    descriptors: Vec<ToolDescriptor>,
}

impl McpToolProvider {
    /// Build from per-server (handle, raw-tools) pairs. Performs the
    /// qualify + dedup-with-warn step.
    pub(crate) fn from_handshake_outputs(
        outputs: Vec<(Arc<McpServerHandle>, Vec<rmcp::model::Tool>)>,
    ) -> Self {
        let mut routes: HashMap<String, Route> = HashMap::new();
        let mut descriptors: Vec<ToolDescriptor> = Vec::new();

        for (handle, tools) in outputs {
            for tool in tools {
                let qualified = qualify(&handle.server_name, &tool.name);
                if routes.contains_key(&qualified) {
                    warn!(
                        mcp.server = %handle.server_name,
                        mcp.tool = %tool.name,
                        qualified = %qualified,
                        "duplicate qualified tool name; skipping"
                    );
                    continue;
                }
                let descriptor = ToolDescriptor {
                    name: qualified.clone(),
                    description: tool.description.clone().unwrap_or_default().into_owned(),
                    schema: serde_json::to_value(&tool.input_schema).unwrap_or(Value::Null),
                    execution_class: ExecutionClass::AlwaysSync,
                    outputs_model_visible_multimodal: false,
                };
                routes.insert(
                    qualified,
                    Route {
                        handle: Arc::clone(&handle),
                        raw_name: tool.name.clone().into_owned(),
                    },
                );
                descriptors.push(descriptor);
            }
        }

        Self {
            routes,
            descriptors,
        }
    }
}

#[async_trait]
impl ToolProvider for McpToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn invoke(&self, name: &str, args: Value, ctx: ExecCtx) -> InvokeOutcome {
        let Some(route) = self.routes.get(name) else {
            return InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("unknown MCP tool: {name}"),
                retryable: false,
            });
        };

        match call_tool(&route.handle, &route.raw_name, args, &ctx).await {
            Ok(call_result) => InvokeOutcome::Sync(to_cogito_result(call_result)),
            Err(CallError::Cancelled) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::Cancelled,
                message: format!("MCP tool `{name}` cancelled"),
                retryable: false,
            }),
            Err(CallError::Timeout(d)) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::Timeout,
                message: format!("MCP tool `{name}` timed out after {}s", d.as_secs_f64()),
                retryable: true,
            }),
            Err(CallError::Other(e)) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("MCP tool `{name}` failed: {e}"),
                retryable: false,
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::{SessionId, TurnId};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn unknown_tool_returns_invalid_args() {
        let provider = McpToolProvider {
            routes: HashMap::new(),
            descriptors: vec![],
        };
        let ctx = ExecCtx {
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            call_id: None,
            deadline: None,
            cancel: CancellationToken::new(),
            subagent_depth: 0,
            brain_spawner: None,
        };
        let outcome = provider.invoke("mcp__nope__nope", Value::Null, ctx).await;
        let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = outcome else {
            panic!("expected Sync(Error)");
        };
        assert!(matches!(kind, ToolErrorKind::InvalidArgs));
    }
}

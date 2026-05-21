//! Per-server `rmcp` client handle. Owns a running rmcp service and
//! the per-server policy (timeouts, tool filter applied at handshake).
//!
//! The runtime path (provider) consumes [`McpServerHandle`],
//! [`call_tool`], and [`CallError`]. The handshake path
//! ([`handshake_and_list`], [`HandshakeOutcome`], [`filter_tools`],
//! [`client_info`], and the `DEFAULT_STARTUP_TIMEOUT` constant) is
//! consumed by [`crate::factory::build_mcp_provider`].

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::service::{self, RoleClient, RunningService};
use tokio::time;

use crate::config::McpServerConfig;
use crate::error::McpStartupFailure;
use crate::handler::MinimalClientHandler;
use crate::transport::{BuiltTransport, build_transport};

/// Default startup timeout when [`McpServerConfig::startup_timeout_sec`]
/// is omitted.
pub(crate) const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Default tool-call timeout when [`McpServerConfig::tool_timeout_sec`]
/// is omitted.
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// A live MCP server, post-handshake.
pub(crate) struct McpServerHandle {
    pub(crate) server_name: String,
    pub(crate) service: Arc<RunningService<RoleClient, MinimalClientHandler>>,
    pub(crate) tool_timeout: Duration,
}

/// What the handshake produced.
pub(crate) struct HandshakeOutcome {
    pub(crate) handle: McpServerHandle,
    /// Server-internal tools (post enabled/disabled filter), one per
    /// entry the provider should register.
    pub(crate) tools: Vec<rmcp::model::Tool>,
}

fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("cogito", env!("CARGO_PKG_VERSION")),
    )
}

/// Build transport, spawn rmcp service, call `tools/list`, all wrapped
/// in [`McpServerConfig::startup_timeout_sec`].
///
/// On any failure, returns an [`McpStartupFailure`] (NOT a normal
/// `Result::Err`) — the caller (factory) collects these and proceeds.
pub(crate) async fn handshake_and_list(
    cfg: &McpServerConfig,
) -> Result<HandshakeOutcome, McpStartupFailure> {
    let startup_timeout = cfg
        .startup_timeout_sec
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(DEFAULT_STARTUP_TIMEOUT);

    let tool_timeout = cfg
        .tool_timeout_sec
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(DEFAULT_TOOL_TIMEOUT);

    let transport = build_transport(&cfg.name, &cfg.transport)?;
    let handler = MinimalClientHandler::new(cfg.name.clone(), client_info());

    // `serve_client` is generic over the transport, so the two enum
    // arms must be matched out to monomorphize separately.
    let service_result = match transport {
        BuiltTransport::ChildProcess(t) => {
            time::timeout(startup_timeout, service::serve_client(handler, t)).await
        }
        BuiltTransport::StreamableHttp(t) => {
            time::timeout(startup_timeout, service::serve_client(handler, t)).await
        }
    };

    let service = match service_result {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err(McpStartupFailure::HandshakeFailed {
                name: cfg.name.clone(),
                error: format!("{e}"),
            });
        }
        Err(_) => {
            return Err(McpStartupFailure::StartupTimeout {
                name: cfg.name.clone(),
                timeout_sec: startup_timeout.as_secs_f64(),
            });
        }
    };

    // `tools/list` is part of startup; wrap in the same total timeout
    // for simplicity (Instant-based remaining-time math is left for a
    // future tightening).
    let list_result = time::timeout(startup_timeout, service.list_tools(None))
        .await
        .map_err(|_| McpStartupFailure::StartupTimeout {
            name: cfg.name.clone(),
            timeout_sec: startup_timeout.as_secs_f64(),
        })?
        .map_err(|e| McpStartupFailure::HandshakeFailed {
            name: cfg.name.clone(),
            error: format!("tools/list: {e}"),
        })?;

    let raw_tools = list_result.tools;
    let tools = filter_tools(
        raw_tools,
        cfg.enabled_tools.as_deref(),
        cfg.disabled_tools.as_deref(),
    );

    Ok(HandshakeOutcome {
        handle: McpServerHandle {
            server_name: cfg.name.clone(),
            service: Arc::new(service),
            tool_timeout,
        },
        tools,
    })
}

/// Apply enabled/disabled filters. `enabled` first (when set), then
/// `disabled` removes from the result.
fn filter_tools(
    tools: Vec<rmcp::model::Tool>,
    enabled: Option<&[String]>,
    disabled: Option<&[String]>,
) -> Vec<rmcp::model::Tool> {
    let mut out = if let Some(allow) = enabled {
        let set: std::collections::HashSet<&str> = allow.iter().map(String::as_str).collect();
        tools
            .into_iter()
            .filter(|t| set.contains(t.name.as_ref()))
            .collect()
    } else {
        tools
    };
    if let Some(deny) = disabled {
        let set: std::collections::HashSet<&str> = deny.iter().map(String::as_str).collect();
        out.retain(|t| !set.contains(t.name.as_ref()));
    }
    out
}

/// Invoke a tool on this server's running service. The effective
/// timeout is `min(handle.tool_timeout, ctx.deadline-remaining)`; the
/// call races against `ctx.cancel` so a cancelled turn drops the
/// in-flight rmcp future (which closes the request natively).
pub(crate) async fn call_tool(
    handle: &McpServerHandle,
    raw_tool_name: &str,
    args: serde_json::Value,
    ctx: &cogito_protocol::ExecCtx,
) -> Result<rmcp::model::CallToolResult, CallError> {
    let now = std::time::Instant::now();
    let remaining = ctx
        .deadline
        .and_then(|d| d.checked_duration_since(now))
        .unwrap_or(handle.tool_timeout);
    let effective = remaining.min(handle.tool_timeout);

    let args_obj = match args {
        serde_json::Value::Object(map) => Some(map),
        serde_json::Value::Null => None,
        // rmcp expects a JSON object for arguments; anything else is
        // a schema violation upstream of us.
        other => {
            return Err(CallError::Other(format!(
                "expected object arguments, got {}",
                value_kind(&other)
            )));
        }
    };

    let mut params = CallToolRequestParams::new(raw_tool_name.to_string());
    params.arguments = args_obj;

    tokio::select! {
        () = ctx.cancel.cancelled() => Err(CallError::Cancelled),
        result = time::timeout(effective, handle.service.call_tool(params)) => {
            match result {
                Ok(Ok(call_result)) => Ok(call_result),
                Ok(Err(e)) => Err(CallError::Other(format!("{e}"))),
                Err(_) => Err(CallError::Timeout(effective)),
            }
        }
    }
}

/// Errors from [`call_tool`]. Surfaced to the provider; provider maps
/// to `ToolResult::Error` variants per ADR-0018 §5.
#[derive(Debug)]
pub(crate) enum CallError {
    /// `ctx.cancel` fired before the call completed.
    Cancelled,
    /// The effective timeout fired before the call completed.
    Timeout(Duration),
    /// Any other failure (transport, server-side error, malformed
    /// arguments).
    Other(String),
}

fn value_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

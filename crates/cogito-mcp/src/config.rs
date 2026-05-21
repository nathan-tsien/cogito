//! Configuration value types for MCP server entries.
//!
//! See ADR-0018 §2 for transport scope and §3 for failure-mode
//! implications, and `docs/configuration/overview.md` §"MCP servers"
//! for the human-facing reference.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One MCP server entry in the `[[mcp_servers]]` config array.
///
/// `transport` is a tagged enum (`transport = "stdio" | "streamable_http"`)
/// dispatched at server build time. `name` must be globally unique
/// within the array — duplicates land as
/// [`crate::error::McpStartupFailure::DuplicateName`] (ADR-0018 §3).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpServerConfig {
    /// Server identifier; appears in `mcp__<name>__<tool>` qualified
    /// names and in the startup banner.
    pub name: String,

    /// Transport-specific fields (stdio command/args or HTTP url/auth).
    #[serde(flatten)]
    pub transport: McpTransportConfig,

    /// Startup timeout in seconds (handshake + initial `tools/list`).
    /// Defaults to 10 seconds at the call site when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_sec: Option<f64>,

    /// Per-call timeout in seconds for `tools/call`. Defaults to
    /// 60 seconds at the call site when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_sec: Option<f64>,

    /// Allowlist of raw (server-internal) tool names. When set, only
    /// these tools are registered with cogito. Names are matched
    /// against the server-side raw name, NOT the qualified
    /// `mcp__<server>__<tool>` form — users write configs against
    /// what the server reports, not after sanitization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,

    /// Denylist of raw (server-internal) tool names. Applied after
    /// `enabled_tools`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<Vec<String>>,
}

/// Transport-specific configuration.
///
/// Marked `#[non_exhaustive]` so future transports (e.g. WebSocket if
/// MCP ever standardizes one) can land additively without breaking
/// downstream `match` arms.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum McpTransportConfig {
    /// stdio child process. See MCP spec
    /// <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#stdio>.
    Stdio {
        /// Command to execute; resolved via `PATH` if not absolute.
        command: String,
        /// Arguments passed verbatim to the child. cogito does **not**
        /// expand `~`, `$VAR`, or normalize relative paths — see
        /// `docs/configuration/overview.md` for the rationale.
        #[serde(default)]
        args: Vec<String>,
        /// Explicit environment variables to inject (in addition to
        /// the cogito process environment).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    /// streamable-HTTP endpoint. See MCP spec
    /// <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#streamable-http>.
    StreamableHttp {
        /// HTTP(S) endpoint URL.
        url: String,
        /// Name of the env var holding the bearer token. The literal
        /// token must NOT appear in the config (ADR-0018 §2). When
        /// omitted, no `Authorization` header is sent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        /// Static HTTP headers to attach to every request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_headers: Option<HashMap<String, String>>,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn stdio_variant_round_trips() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "filesystem"
            transport = "stdio"
            command = "uvx"
            args = ["mcp-server-filesystem", "/tmp"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.name, "filesystem");
        match cfg.transport {
            McpTransportConfig::Stdio {
                command,
                args,
                env: _,
            } => {
                assert_eq!(command, "uvx");
                assert_eq!(args, vec!["mcp-server-filesystem", "/tmp"]);
            }
            McpTransportConfig::StreamableHttp { .. } => panic!("wrong variant"),
        }
    }

    #[test]
    fn streamable_http_variant_round_trips() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "company_api"
            transport = "streamable_http"
            url = "https://mcp.example.com/v1"
            bearer_token_env_var = "COMPANY_MCP_TOKEN"
            "#,
        )
        .unwrap();
        match cfg.transport {
            McpTransportConfig::StreamableHttp {
                url,
                bearer_token_env_var,
                ..
            } => {
                assert_eq!(url, "https://mcp.example.com/v1");
                assert_eq!(bearer_token_env_var.as_deref(), Some("COMPANY_MCP_TOKEN"));
            }
            McpTransportConfig::Stdio { .. } => panic!("wrong variant"),
        }
    }

    #[test]
    fn literal_bearer_token_field_is_rejected() {
        let err = toml::from_str::<McpServerConfig>(
            r#"
            name = "leaky"
            transport = "streamable_http"
            url = "https://x.example.com"
            bearer_token = "this-should-not-be-here"
            "#,
        )
        .expect_err("must reject literal bearer_token field");
        let msg = err.to_string();
        assert!(
            msg.contains("bearer_token") || msg.contains("unknown"),
            "error should mention unknown bearer_token field: {msg}"
        );
    }

    #[test]
    fn missing_transport_field_errors() {
        let err = toml::from_str::<McpServerConfig>(
            r#"
            name = "incomplete"
            command = "x"
            "#,
        )
        .expect_err("must reject entry without transport tag");
        let msg = err.to_string();
        assert!(
            msg.contains("transport") || msg.contains("missing"),
            "error should mention missing transport: {msg}"
        );
    }

    #[test]
    fn enabled_disabled_tools_are_optional() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "minimal"
            transport = "stdio"
            command = "echo"
            "#,
        )
        .unwrap();
        assert!(cfg.enabled_tools.is_none());
        assert!(cfg.disabled_tools.is_none());
        assert!(cfg.startup_timeout_sec.is_none());
        assert!(cfg.tool_timeout_sec.is_none());
    }

    #[test]
    fn timeouts_round_trip_as_floats() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "with_timeouts"
            transport = "stdio"
            command = "x"
            startup_timeout_sec = 15.5
            tool_timeout_sec = 30
            "#,
        )
        .unwrap();
        assert!((cfg.startup_timeout_sec.unwrap() - 15.5).abs() < 1e-9);
        assert!((cfg.tool_timeout_sec.unwrap() - 30.0).abs() < 1e-9);
    }
}

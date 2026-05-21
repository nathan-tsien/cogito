//! `build_mcp_provider` — the soft-skip surface for MCP server startup.
//!
//! Per ADR-0018 §3, this function **never** returns `Result::Err`.
//! It returns [`McpProviderBuildResult`] carrying a (possibly absent)
//! provider and the list of per-server failures. A Surface
//! (`cogito-cli`, future `cogito-tui`, consumer Server) joins these
//! with any parse-time failures from `cogito-config` and surfaces
//! the full list via a startup banner.

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::tool::ToolProvider;
use tokio::task::JoinSet;

use crate::client::{HandshakeOutcome, McpServerHandle, handshake_and_list};
use crate::config::McpServerConfig;
use crate::error::McpStartupFailure;
use crate::provider::McpToolProvider;

/// Outcome of [`build_mcp_provider`].
///
/// `provider == None` when no server came up successfully (every
/// entry failed, or the input list was empty after duplicate
/// pruning). `failures` contains every per-server failure, in input
/// order (duplicates first, then handshake failures in `JoinSet`
/// completion order).
pub struct McpProviderBuildResult {
    /// Composite provider, or `None` if nothing came up.
    pub provider: Option<Arc<dyn ToolProvider>>,
    /// All per-server failures encountered.
    pub failures: Vec<McpStartupFailure>,
}

/// Bring up every configured MCP server concurrently. Servers that
/// fail are recorded; servers that succeed contribute their tools to
/// the returned provider. Runtime **never** sees a `Result::Err`
/// from this function — by design.
pub async fn build_mcp_provider(cfgs: &[McpServerConfig]) -> McpProviderBuildResult {
    if cfgs.is_empty() {
        return McpProviderBuildResult {
            provider: None,
            failures: Vec::new(),
        };
    }

    let mut failures: Vec<McpStartupFailure> = Vec::new();

    // Deduplicate by name up-front; later entries become DuplicateName failures.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut survivors: Vec<&McpServerConfig> = Vec::with_capacity(cfgs.len());
    for (idx, cfg) in cfgs.iter().enumerate() {
        if seen.insert(&cfg.name) {
            survivors.push(cfg);
        } else {
            failures.push(McpStartupFailure::DuplicateName {
                name: cfg.name.clone(),
                index: idx,
            });
        }
    }

    let mut joinset: JoinSet<Result<HandshakeOutcome, McpStartupFailure>> = JoinSet::new();
    for cfg in survivors {
        let cfg = cfg.clone();
        joinset.spawn(async move { handshake_and_list(&cfg).await });
    }

    let mut outputs: Vec<(Arc<McpServerHandle>, Vec<rmcp::model::Tool>)> = Vec::new();

    while let Some(joined) = joinset.join_next().await {
        match joined {
            Ok(Ok(outcome)) => {
                outputs.push((Arc::new(outcome.handle), outcome.tools));
            }
            Ok(Err(failure)) => {
                failures.push(failure);
            }
            Err(join_err) => {
                // Task panicked. Should not happen — handshake_and_list
                // is panic-safe. Surface as a generic transport error
                // so it shows up in the banner instead of being lost.
                failures.push(McpStartupFailure::TransportError {
                    name: "<unknown>".into(),
                    error: format!("task join error: {join_err}"),
                });
            }
        }
    }

    let provider: Option<Arc<dyn ToolProvider>> = if outputs.is_empty() {
        None
    } else {
        Some(Arc::new(McpToolProvider::from_handshake_outputs(outputs)))
    };

    McpProviderBuildResult { provider, failures }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::McpTransportConfig;

    #[tokio::test]
    async fn empty_input_returns_none_provider_and_no_failures() {
        let result = build_mcp_provider(&[]).await;
        assert!(result.provider.is_none());
        assert!(result.failures.is_empty());
    }

    #[tokio::test]
    async fn duplicate_names_record_failures_for_later_entries() {
        let cfgs = vec![
            McpServerConfig {
                name: "x".into(),
                transport: McpTransportConfig::Stdio {
                    command: "/nonexistent/binary".into(),
                    args: vec![],
                    env: None,
                },
                startup_timeout_sec: Some(0.1),
                tool_timeout_sec: None,
                enabled_tools: None,
                disabled_tools: None,
            },
            McpServerConfig {
                name: "x".into(), // duplicate
                transport: McpTransportConfig::Stdio {
                    command: "/nonexistent/binary".into(),
                    args: vec![],
                    env: None,
                },
                startup_timeout_sec: Some(0.1),
                tool_timeout_sec: None,
                enabled_tools: None,
                disabled_tools: None,
            },
        ];
        let result = build_mcp_provider(&cfgs).await;
        // First entry tries to start and fails (binary missing) — TransportError.
        // Second entry hits dedup — DuplicateName.
        let dup_count = result
            .failures
            .iter()
            .filter(|f| matches!(f, McpStartupFailure::DuplicateName { .. }))
            .count();
        assert_eq!(dup_count, 1);
    }

    #[tokio::test]
    async fn all_servers_fail_yields_none_provider_with_failures() {
        let cfgs = vec![McpServerConfig {
            name: "broken".into(),
            transport: McpTransportConfig::Stdio {
                command: "/this/path/does/not/exist".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: Some(0.1),
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        }];
        let result = build_mcp_provider(&cfgs).await;
        // Critical invariant: even when EVERYTHING fails, build returns
        // a value (no Result::Err) — this is the compiler-enforced
        // soft-skip from ADR-0018 §3.
        assert!(result.provider.is_none());
        assert!(!result.failures.is_empty());
    }
}

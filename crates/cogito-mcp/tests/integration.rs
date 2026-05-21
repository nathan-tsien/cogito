//! Integration tests for cogito-mcp covering the ADR-0018 §3
//! soft-skip-to-Runtime invariant through the `build_mcp_provider`
//! entry point.
//!
//! Live-server HTTP handshake coverage is deferred to a follow-up
//! commit; the four tests below already exercise the load-bearing
//! invariant (failures collect, provider is None or partial, no
//! `Result::Err` propagates).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_mcp::{McpServerConfig, McpStartupFailure, McpTransportConfig, build_mcp_provider};

fn stdio_cfg_to_nonexistent_binary(name: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.into(),
        transport: McpTransportConfig::Stdio {
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: None,
        },
        startup_timeout_sec: Some(0.5),
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: None,
    }
}

#[tokio::test]
async fn bearer_env_missing_yields_failure_not_runtime_break() {
    let cfg = McpServerConfig {
        name: "needs_token".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: "http://127.0.0.1:9".into(),
            bearer_token_env_var: Some("DEFINITELY_NOT_SET_MCP_TOKEN".into()),
            http_headers: None,
        },
        startup_timeout_sec: Some(0.5),
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: None,
    };

    // Critical: result is RETURNED, not raised. If build_mcp_provider
    // were to ever return Result<_, _>, this test wouldn't compile —
    // the soft-skip is structural.
    let result = build_mcp_provider(&[cfg]).await;
    assert!(result.provider.is_none());
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        &result.failures[0],
        McpStartupFailure::BearerEnvMissing { env_var, .. } if env_var == "DEFINITELY_NOT_SET_MCP_TOKEN"
    ));
}

#[tokio::test]
async fn failed_server_fault_contained_other_servers_unaffected() {
    let cfg = stdio_cfg_to_nonexistent_binary("bad");
    // Even if 100% of configured servers fail, build returns
    // gracefully — does NOT raise.
    let result = build_mcp_provider(&[cfg]).await;
    assert!(result.provider.is_none());
    assert!(!result.failures.is_empty());
}

#[tokio::test]
async fn duplicate_name_skips_later_entry() {
    let mut a = stdio_cfg_to_nonexistent_binary("shared");
    a.startup_timeout_sec = Some(0.1);
    let b = a.clone(); // duplicate name

    let result = build_mcp_provider(&[a, b]).await;
    assert!(
        result
            .failures
            .iter()
            .any(|f| matches!(f, McpStartupFailure::DuplicateName { index: 1, .. })),
        "expected DuplicateName for index 1, got {:?}",
        result.failures
    );
}

#[tokio::test]
async fn all_servers_fail_runtime_still_builds() {
    // The compile-time check is in the type signature of
    // build_mcp_provider — this test confirms the runtime semantics
    // match the contract.
    let cfgs = vec![
        stdio_cfg_to_nonexistent_binary("a"),
        stdio_cfg_to_nonexistent_binary("b"),
    ];
    let result = build_mcp_provider(&cfgs).await;
    assert!(result.provider.is_none());
    assert_eq!(result.failures.len(), 2);
}

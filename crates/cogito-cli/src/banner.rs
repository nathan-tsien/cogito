//! Startup banner — prints per-server MCP status to stderr after
//! Runtime construction completes. See ADR-0018 §3.5.3 for the
//! contract: every Surface MUST emit this so silent skips are
//! visible to users.

use std::collections::HashSet;
use std::io::Write;

use cogito_mcp::{McpServerConfig, McpStartupFailure};

/// Render the banner to a writer. Caller passes `&mut io::stderr()`.
///
/// Format (one line per server, plus an optional all-fail note):
/// ```text
/// [mcp] ✓ filesystem ready (4 tools)
/// [mcp] ✗ broken_server skipped: env var `COMPANY_MCP_TOKEN` is unset
/// [mcp] ✗ mcp_servers[3] skipped: unknown transport "websocket"
/// [mcp] note: 0 of N configured servers came up; running with builtin tools only
/// ```
//
// `dead_code` is silenced because Task 12 lands this function ahead of
// the Task 13 caller in `chat.rs`. Once `chat::run` invokes it the
// attribute can be removed.
#[allow(dead_code)]
pub fn render_banner<W: Write>(
    out: &mut W,
    configs: &[McpServerConfig],
    failures: &[McpStartupFailure],
    successful_tool_counts: &[(String, usize)],
) -> std::io::Result<()> {
    let configured_count = configs.len();
    let success_count = successful_tool_counts.len();

    // Collect names that failed (mapped from McpStartupFailure::server_name).
    let failed_names: HashSet<&str> = failures
        .iter()
        .filter_map(McpStartupFailure::server_name)
        .collect();

    // Successful servers (in original config order).
    for cfg in configs {
        if let Some((_, n_tools)) = successful_tool_counts
            .iter()
            .find(|(name, _)| name == &cfg.name)
        {
            writeln!(out, "[mcp] ✓ {} ready ({} tools)", cfg.name, n_tools)?;
        } else if failed_names.contains(cfg.name.as_str()) {
            // Find its specific failure for the reason.
            let reason = failures
                .iter()
                .find(|f| f.server_name() == Some(cfg.name.as_str()))
                .map_or_else(|| "unknown error".to_string(), ToString::to_string);
            writeln!(out, "[mcp] ✗ {} skipped: {}", cfg.name, reason)?;
        }
    }

    // ConfigParse failures have no server name; render by index.
    for failure in failures {
        if let McpStartupFailure::ConfigParse { index, error } = failure {
            writeln!(out, "[mcp] ✗ mcp_servers[{index}] skipped: {error}")?;
        }
    }

    // All-fail summary, only if any servers were configured.
    if configured_count > 0 && success_count == 0 {
        writeln!(
            out,
            "[mcp] note: 0 of {configured_count} configured servers came up; running with builtin tools only"
        )?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_mcp::McpTransportConfig;

    fn cfg(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            transport: McpTransportConfig::Stdio {
                command: "x".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        }
    }

    fn render(
        cfgs: &[McpServerConfig],
        fails: &[McpStartupFailure],
        ok: &[(String, usize)],
    ) -> String {
        let mut buf = Vec::new();
        render_banner(&mut buf, cfgs, fails, ok).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn all_success_prints_one_line_per_server() {
        let configs = vec![cfg("a"), cfg("b")];
        let ok = vec![("a".into(), 3), ("b".into(), 7)];
        let out = render(&configs, &[], &ok);
        assert!(out.contains("[mcp] ✓ a ready (3 tools)"));
        assert!(out.contains("[mcp] ✓ b ready (7 tools)"));
        assert!(!out.contains("note:"));
    }

    #[test]
    fn missing_env_var_failure_renders_with_reason() {
        let configs = vec![cfg("broken")];
        let failures = vec![McpStartupFailure::BearerEnvMissing {
            name: "broken".into(),
            env_var: "TOKEN".into(),
        }];
        let out = render(&configs, &failures, &[]);
        assert!(out.contains("[mcp] ✗ broken skipped"));
        assert!(out.contains("env var `TOKEN`"));
    }

    #[test]
    fn parse_failure_renders_with_index() {
        let failures = vec![McpStartupFailure::ConfigParse {
            index: 2,
            error: "unknown transport \"ws\"".into(),
        }];
        let out = render(&[], &failures, &[]);
        assert!(out.contains("[mcp] ✗ mcp_servers[2] skipped"));
        assert!(out.contains("unknown transport"));
    }

    #[test]
    fn all_fail_appends_summary_note() {
        let configs = vec![cfg("a"), cfg("b")];
        let failures = vec![
            McpStartupFailure::TransportError {
                name: "a".into(),
                error: "no binary".into(),
            },
            McpStartupFailure::HandshakeFailed {
                name: "b".into(),
                error: "timeout".into(),
            },
        ];
        let out = render(&configs, &failures, &[]);
        assert!(
            out.contains(
                "[mcp] note: 0 of 2 configured servers came up; running with builtin tools only"
            ),
            "missing summary line: {out}"
        );
    }

    #[test]
    fn no_servers_configured_emits_nothing() {
        let out = render(&[], &[], &[]);
        assert_eq!(out, "");
    }
}

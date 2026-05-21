//! H05 Tool Surface Builder — pure, deterministic filter + sort.
//!
//! See `docs/components/H05-tool-surface.md` §"v0.1 scope" and
//! ADR-0018 §7 for the MCP observability contract.

use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::tool::{ToolDescriptor, ToolProvider};

/// Prefix marking an MCP-sourced qualified tool name. Hardcoded here
/// (not imported from cogito-mcp) to preserve the Brain-only-imports-
/// Protocol rule in ADR-0004; cogito-mcp owns the production string.
const MCP_TOOL_PREFIX: &str = "mcp__";

/// Build the per-turn tool surface from the active strategy and the
/// injected provider's full catalog. Sort: `strategy.tool_order` first
/// (in given order), then remaining tools alphabetically.
pub fn surface(strategy: &HarnessStrategy, provider: &dyn ToolProvider) -> Vec<ToolDescriptor> {
    let mut allowed: Vec<ToolDescriptor> = provider
        .list()
        .into_iter()
        .filter(|d| match &strategy.allowed_tools {
            ToolFilter::All => true,
            ToolFilter::Allow(names) => names.iter().any(|n| n == &d.name),
        })
        .collect();

    if let Some(order) = &strategy.tool_order {
        // Sort: explicitly-ordered names first (in given order), then
        // remaining tools alphabetically.
        allowed.sort_by_key(|d| {
            order
                .iter()
                .position(|n| n == &d.name)
                .unwrap_or(usize::MAX)
        });
        // Stable-sort the "remaining" (usize::MAX) bucket by name.
        let split = allowed
            .iter()
            .position(|d| !order.contains(&d.name))
            .unwrap_or(allowed.len());
        allowed[split..].sort_by(|a, b| a.name.cmp(&b.name));
    } else {
        allowed.sort_by(|a, b| a.name.cmp(&b.name));
    }

    emit_surface_telemetry(&allowed);
    allowed
}

/// Emit per-surface tool-count + byte-size telemetry. Pure
/// observability — see ADR-0018 §7.
fn emit_surface_telemetry(tools: &[ToolDescriptor]) {
    let mcp_count = tools
        .iter()
        .filter(|d| d.name.starts_with(MCP_TOOL_PREFIX))
        .count();
    let mcp_desc_bytes: usize = tools
        .iter()
        .filter(|d| d.name.starts_with(MCP_TOOL_PREFIX))
        .map(|d| d.description.len())
        .sum();
    let builtin_count = tools.len() - mcp_count;

    tracing::info!(
        target: "h05.tool_surface",
        {
            mcp.tool_count = mcp_count,
            mcp.tool_desc_total_bytes = mcp_desc_bytes,
            builtin.tool_count = builtin_count,
        },
        "tool surface built"
    );
}

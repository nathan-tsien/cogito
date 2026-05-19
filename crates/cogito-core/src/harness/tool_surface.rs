//! H05 Tool Surface Builder — pure, deterministic filter + sort.
//!
//! See `docs/components/H05-tool-surface.md` §"v0.1 scope".

use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::tool::{ToolDescriptor, ToolProvider};

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
    allowed
}

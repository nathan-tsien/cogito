//! H05 Tool Surface Builder — pure, deterministic filter + sort.
//!
//! See `docs/components/H05-tool-surface.md` §"v0.1 scope" and
//! ADR-0018 §7 for the MCP observability contract.

use cogito_protocol::context::ToolFilterOverrideMode;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::ids::TurnId;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::tool::{ToolDescriptor, ToolProvider};

/// Prefix marking an MCP-sourced qualified tool name. Hardcoded here
/// (not imported from cogito-mcp) to preserve the Brain-only-imports-
/// Protocol rule in ADR-0004; cogito-mcp owns the production string.
const MCP_TOOL_PREFIX: &str = "mcp__";

/// Build the per-turn tool surface from the active strategy, the injected
/// provider's full catalog, and any `ToolFilterOverridden` event written by
/// H11 for `current_turn`.
///
/// Override modes (ADR-0008):
/// - `Inherit`          — use `strategy.allowed_tools` unchanged (default).
/// - `Intersect(names)` — intersect the strategy filter result with `names`.
/// - `Replace(names)`   — ignore `strategy.allowed_tools`; use provider's full
///   catalog filtered to `names` only.
///
/// Sort: `strategy.tool_order` first (in given order), then remaining tools
/// alphabetically.
pub fn surface(
    strategy: &HarnessStrategy,
    provider: &dyn ToolProvider,
    events: &[ConversationEvent],
    current_turn: TurnId,
) -> Vec<ToolDescriptor> {
    // Find the latest ToolFilterOverridden event for this turn, if any.
    let mode = find_override_mode(events, current_turn);

    let mut allowed: Vec<ToolDescriptor> = {
        // ToolFilterOverrideMode is #[non_exhaustive]. Unknown future variants
        // are treated as Inherit (strategy filter applied unchanged).
        match &mode {
            // Replace: ignore strategy filter; start from the full provider catalog,
            // then keep only the explicitly listed names.
            Some(ToolFilterOverrideMode::Replace { tools }) => provider
                .list()
                .into_iter()
                .filter(|d| tools.iter().any(|n| n == &d.name))
                .collect(),

            // Intersect: apply strategy filter first, then keep only the listed names.
            Some(ToolFilterOverrideMode::Intersect { tools }) => {
                let intersect_names = tools;
                provider
                    .list()
                    .into_iter()
                    .filter(|d| match &strategy.allowed_tools {
                        ToolFilter::All => true,
                        ToolFilter::Allow(names) => names.iter().any(|n| n == &d.name),
                    })
                    .filter(|d| intersect_names.iter().any(|n| n == &d.name))
                    .collect()
            }

            // Inherit, None, or unknown future variant: apply strategy.allowed_tools.
            None | Some(ToolFilterOverrideMode::Inherit | _) => provider
                .list()
                .into_iter()
                .filter(|d| match &strategy.allowed_tools {
                    ToolFilter::All => true,
                    ToolFilter::Allow(names) => names.iter().any(|n| n == &d.name),
                })
                .collect(),
        }
    };

    apply_sort(&mut allowed, strategy);
    emit_surface_telemetry(&allowed);
    allowed
}

/// Find the `ToolFilterOverrideMode` from the latest `ToolFilterOverridden`
/// event for `current_turn`. Returns `None` when no such event exists.
fn find_override_mode(
    events: &[ConversationEvent],
    current_turn: TurnId,
) -> Option<ToolFilterOverrideMode> {
    events.iter().rev().find_map(|ev| {
        if let EventPayload::ToolFilterOverridden { turn_id, mode, .. } = &ev.payload {
            if *turn_id == current_turn {
                return Some(mode.clone());
            }
        }
        None
    })
}

/// Sort `tools` in place: `strategy.tool_order` positions first (in given
/// order), then the remaining tools alphabetically.
fn apply_sort(tools: &mut [ToolDescriptor], strategy: &HarnessStrategy) {
    if let Some(order) = &strategy.tool_order {
        tools.sort_by_key(|d| {
            order
                .iter()
                .position(|n| n == &d.name)
                .unwrap_or(usize::MAX)
        });
        // Stable-sort the "remaining" (usize::MAX) bucket by name.
        let split = tools
            .iter()
            .position(|d| !order.contains(&d.name))
            .unwrap_or(tools.len());
        tools[split..].sort_by(|a, b| a.name.cmp(&b.name));
    } else {
        tools.sort_by(|a, b| a.name.cmp(&b.name));
    }
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

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::match_wildcard_for_single_variants
)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::context::ToolFilterOverrideMode;
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
    use cogito_protocol::tool::{
        ExecutionClass, InvokeOutcome, ToolDescriptor, ToolProvider, ToolResult,
    };

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn make_descriptor(name: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.into(),
            description: format!("desc for {name}"),
            schema: serde_json::json!({}),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    /// A `ToolProvider` backed by a fixed list of descriptors.
    struct FixedProvider(Vec<ToolDescriptor>);

    #[async_trait::async_trait]
    impl ToolProvider for FixedProvider {
        fn list(&self) -> Vec<ToolDescriptor> {
            self.0.clone()
        }

        async fn invoke(
            &self,
            _name: &str,
            _args: serde_json::Value,
            _ctx: cogito_protocol::ExecCtx,
        ) -> InvokeOutcome {
            InvokeOutcome::Sync(ToolResult::text("unused"))
        }
    }

    fn make_override_event(turn_id: TurnId, mode: ToolFilterOverrideMode) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: Some(turn_id),
            seq: 0,
            ts: Utc::now(),
            payload: EventPayload::ToolFilterOverridden {
                turn_id,
                mode,
                contributors: vec![],
                produced_by: "test".into(),
            },
        }
    }

    // ---------------------------------------------------------------------------
    // Task 30 tests
    // ---------------------------------------------------------------------------

    #[test]
    fn h05_inherits_when_no_override() {
        let turn_id = TurnId::new();
        let mut strategy = HarnessStrategy::default_with_model("test");
        strategy.allowed_tools = ToolFilter::Allow(vec!["tool_a".into(), "tool_b".into()]);

        let provider = FixedProvider(vec![
            make_descriptor("tool_a"),
            make_descriptor("tool_b"),
            make_descriptor("tool_c"),
        ]);

        // No events for this turn at all.
        let result = surface(&strategy, &provider, &[], turn_id);

        let names: Vec<&str> = result.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["tool_a", "tool_b"],
            "strategy filter applied; no override"
        );
    }

    #[test]
    fn h05_intersects_with_override() {
        let turn_id = TurnId::new();
        let mut strategy = HarnessStrategy::default_with_model("test");
        // Strategy allows all three tools.
        strategy.allowed_tools = ToolFilter::All;

        let provider = FixedProvider(vec![
            make_descriptor("tool_a"),
            make_descriptor("tool_b"),
            make_descriptor("tool_c"),
        ]);

        // Override intersects to only tool_a and tool_b.
        let events = vec![make_override_event(
            turn_id,
            ToolFilterOverrideMode::Intersect {
                tools: vec!["tool_a".into(), "tool_b".into()],
            },
        )];

        let result = surface(&strategy, &provider, &events, turn_id);
        let names: Vec<&str> = result.iter().map(|d| d.name.as_str()).collect();

        // tool_c is excluded by the Intersect override even though strategy allows all.
        assert_eq!(names, ["tool_a", "tool_b"]);
    }

    #[test]
    fn h05_replaces_with_override() {
        let turn_id = TurnId::new();
        let mut strategy = HarnessStrategy::default_with_model("test");
        // Strategy blocks tool_c; Replace should override this.
        strategy.allowed_tools = ToolFilter::Allow(vec!["tool_a".into(), "tool_b".into()]);

        let provider = FixedProvider(vec![
            make_descriptor("tool_a"),
            make_descriptor("tool_b"),
            make_descriptor("tool_c"),
        ]);

        // Override replaces with only tool_c (ignored by strategy filter normally).
        let events = vec![make_override_event(
            turn_id,
            ToolFilterOverrideMode::Replace {
                tools: vec!["tool_c".into()],
            },
        )];

        let result = surface(&strategy, &provider, &events, turn_id);
        let names: Vec<&str> = result.iter().map(|d| d.name.as_str()).collect();

        // tool_c must appear even though strategy would normally exclude it.
        assert_eq!(names, ["tool_c"]);
    }
}

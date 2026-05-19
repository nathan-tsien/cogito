//! Tests for `HarnessStrategy` factory + `ToolFilter` wire shape.

use cogito_protocol::{HarnessStrategy, ToolFilter};

#[test]
fn default_factory_yields_safe_defaults() {
    let s = HarnessStrategy::default_with_model("claude-opus-4-7");
    assert_eq!(s.name, "default");
    assert!(matches!(s.allowed_tools, ToolFilter::All));
    assert_eq!(s.max_turns, 16);
    assert_eq!(s.model_params.model, "claude-opus-4-7");
    assert_eq!(s.model_params.max_tokens, 4096);
}

#[test]
fn tool_filter_wire_shape() -> serde_json::Result<()> {
    let all = ToolFilter::All;
    let json = serde_json::to_value(&all)?;
    assert_eq!(json, serde_json::json!("all"));
    let allow = ToolFilter::Allow(vec!["read_file".into(), "grep".into()]);
    let json = serde_json::to_value(&allow)?;
    assert_eq!(json, serde_json::json!({ "allow": ["read_file", "grep"] }));
    Ok(())
}

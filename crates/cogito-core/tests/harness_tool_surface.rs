//! Integration tests for H05 Tool Surface Builder.

use std::sync::Arc;

use cogito_core::harness::tool_surface::surface;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::tool::ToolProvider;
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn provider() -> Arc<dyn ToolProvider> {
    Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    )
}

#[test]
fn allow_all_returns_full_catalog() {
    let s = HarnessStrategy::default_with_model("test");
    let p = provider();
    let out = surface(&s, p.as_ref());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "read_file");
}

#[test]
fn allow_list_filters() {
    let mut s = HarnessStrategy::default_with_model("test");
    s.allowed_tools = ToolFilter::Allow(vec!["grep".into()]); // not in catalog
    let p = provider();
    let out = surface(&s, p.as_ref());
    assert!(out.is_empty());
}

#[test]
fn tool_order_pulls_named_to_front() {
    let mut s = HarnessStrategy::default_with_model("test");
    s.tool_order = Some(vec!["read_file".into()]);
    let p = provider();
    let out = surface(&s, p.as_ref());
    assert_eq!(out[0].name, "read_file");
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::sync::Arc;

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};

struct NamedNoop;
impl HookHandler for NamedNoop {
    fn name(&self) -> &'static str {
        "named-noop"
    }
}

struct AlwaysReject;
impl HookHandler for AlwaysReject {
    fn name(&self) -> &'static str {
        "always-reject"
    }
    fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
        HookDecision::Reject {
            hook_name: "always-reject".into(),
            reason: "no".into(),
        }
    }
}

struct StaticProvider(Vec<Arc<dyn HookHandler>>);
impl HookProvider for StaticProvider {
    fn list(&self) -> Vec<Arc<dyn HookHandler>> {
        self.0.clone()
    }
}

#[test]
fn default_methods_return_allow() {
    let h = NamedNoop;
    assert_eq!(h.name(), "named-noop");
    assert!(matches!(
        h.pre_prompt(&ModelInput::default()),
        HookDecision::Allow
    ));
    assert!(matches!(
        h.pre_dispatch("call-id", "tool", &serde_json::Value::Null),
        HookDecision::Allow
    ));
    h.post_model();
    h.post_turn();
    h.on_error("err");
}

#[test]
fn override_pre_prompt_takes_effect() {
    let h = AlwaysReject;
    match h.pre_prompt(&ModelInput::default()) {
        HookDecision::Reject { hook_name, reason } => {
            assert_eq!(hook_name, "always-reject");
            assert_eq!(reason, "no");
        }
        _ => panic!("expected Reject"),
    }
}

#[test]
fn provider_lists_handlers() {
    let provider = StaticProvider(vec![Arc::new(NamedNoop) as Arc<dyn HookHandler>]);
    let listed = provider.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name(), "named-noop");
}

#[test]
fn lifecycle_point_variants_round_trip() {
    let points = [
        HookLifecyclePoint::PrePrompt,
        HookLifecyclePoint::PreDispatch,
        HookLifecyclePoint::PostModel,
        HookLifecyclePoint::PostTurn,
        HookLifecyclePoint::OnError,
    ];
    for p in points {
        let s = serde_json::to_string(&p).unwrap();
        let back: HookLifecyclePoint = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}

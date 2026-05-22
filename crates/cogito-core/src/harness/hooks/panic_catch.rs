//! Panic-catch wrappers for `HookHandler` lifecycle methods.
//!
//! H09 invariant: a panicking hook must not crash Brain. We wrap each
//! call in `std::panic::catch_unwind(AssertUnwindSafe(...))` and
//! convert the payload into `HookDecision::Reject { ... }`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler};

/// Wraps `HookHandler::pre_prompt` with panic catch.
pub fn wrap_pre_prompt(handler: &dyn HookHandler, input: &ModelInput) -> HookDecision {
    catch_unwind(AssertUnwindSafe(|| handler.pre_prompt(input)))
        .unwrap_or_else(|payload| panic_to_reject(handler.name(), &payload))
}

/// Wraps `HookHandler::pre_dispatch` with panic catch.
pub fn wrap_pre_dispatch(
    handler: &dyn HookHandler,
    call_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
) -> HookDecision {
    catch_unwind(AssertUnwindSafe(|| {
        handler.pre_dispatch(call_id, tool_name, args)
    }))
    .unwrap_or_else(|payload| panic_to_reject(handler.name(), &payload))
}

/// Wraps `HookHandler::post_model` with panic catch.
///
/// Observation hooks return unit; a panic here is logged but does NOT
/// reject (the turn has already produced model output).
pub fn wrap_post_model(handler: &dyn HookHandler) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.post_model()));
}

/// Wraps `HookHandler::post_turn` with panic catch.
pub fn wrap_post_turn(handler: &dyn HookHandler) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.post_turn()));
}

/// Wraps `HookHandler::on_error` with panic catch.
pub fn wrap_on_error(handler: &dyn HookHandler, reason: &str) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.on_error(reason)));
}

fn panic_to_reject(hook_name: &str, payload: &Box<dyn std::any::Any + Send>) -> HookDecision {
    let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_owned()
    };
    HookDecision::Reject {
        hook_name: hook_name.to_owned(),
        reason: format!("hook '{hook_name}' panicked: {msg}"),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unnecessary_literal_bound
)]
mod tests {
    use super::*;

    struct Allowy;
    impl HookHandler for Allowy {
        fn name(&self) -> &str {
            "allowy"
        }
    }

    struct Panicky;
    impl HookHandler for Panicky {
        fn name(&self) -> &str {
            "panicky"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            panic!("boom")
        }
    }

    #[test]
    fn allow_happy_path() {
        let h = Allowy;
        assert!(matches!(
            wrap_pre_prompt(&h, &ModelInput::default()),
            HookDecision::Allow
        ));
    }

    #[test]
    fn panic_becomes_reject_with_hook_name_and_message() {
        let h = Panicky;
        match wrap_pre_prompt(&h, &ModelInput::default()) {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "panicky");
                assert!(reason.contains("panicky"), "{reason}");
                assert!(reason.contains("boom"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }
}

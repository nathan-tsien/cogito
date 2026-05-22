//! `SensitiveContentHook` — rejects tool calls whose args contain
//! well-known secret-shaped strings.
//!
//! Patterns are intentionally conservative — high signal, low false
//! positives. Extend via configuration in a later sprint.

#![allow(clippy::expect_used)] // gated to LazyLock regex constants below

use std::sync::LazyLock;

use cogito_protocol::hook::{HookDecision, HookHandler};
use regex::Regex;

static AWS_ACCESS_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("regex compiles"));
static GITHUB_PAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ghp_[A-Za-z0-9]{36}").expect("regex compiles"));
static OPENAI_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-[A-Za-z0-9]{20,}").expect("regex compiles"));

const HOOK_NAME: &str = "sensitive-content";

/// Hook that rejects tool invocations whose JSON args contain secret-
/// shaped strings.
#[derive(Debug, Default)]
pub struct SensitiveContentHook;

impl SensitiveContentHook {
    /// Creates a new `SensitiveContentHook`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn scan(value: &serde_json::Value) -> Option<&'static str> {
        match value {
            serde_json::Value::String(s) => {
                if AWS_ACCESS_KEY.is_match(s) {
                    return Some("aws-access-key");
                }
                if GITHUB_PAT.is_match(s) {
                    return Some("github-pat");
                }
                if OPENAI_KEY.is_match(s) {
                    return Some("openai-key");
                }
                None
            }
            serde_json::Value::Array(arr) => arr.iter().find_map(Self::scan),
            serde_json::Value::Object(map) => map.values().find_map(Self::scan),
            _ => None,
        }
    }
}

impl HookHandler for SensitiveContentHook {
    fn name(&self) -> &str {
        HOOK_NAME
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        _tool_name: &str,
        args: &serde_json::Value,
    ) -> HookDecision {
        match Self::scan(args) {
            Some(pattern) => HookDecision::Reject {
                hook_name: HOOK_NAME.into(),
                reason: format!("matched sensitive pattern '{pattern}' in tool args"),
            },
            None => HookDecision::Allow,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use cogito_protocol::hook::{HookDecision, HookHandler};
    use serde_json::json;

    use super::*;

    #[test]
    fn clean_args_allow() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch("c1", "any", &json!({"q": "hello world"}));
        assert!(matches!(dec, HookDecision::Allow));
    }

    #[test]
    fn aws_key_rejects_with_pattern_name() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch("c1", "any", &json!({"creds": "AKIAIOSFODNN7EXAMPLE"}));
        match dec {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "sensitive-content");
                assert!(reason.contains("aws-access-key"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn github_pat_rejects() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"token": "ghp_abcdefghijklmnopqrstuvwxyz0123456789"}),
        );
        match dec {
            HookDecision::Reject { reason, .. } => {
                assert!(reason.contains("github-pat"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn openai_key_rejects() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"k": "sk-abcdefghijklmnopqrstuvwxyz0123456789"}),
        );
        match dec {
            HookDecision::Reject { reason, .. } => {
                assert!(reason.contains("openai-key"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn nested_json_is_scanned() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"outer": {"nested": "AKIAIOSFODNN7EXAMPLE"}}),
        );
        assert!(matches!(dec, HookDecision::Reject { .. }));
    }
}

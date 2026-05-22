//! `SensitiveContentHook` — implementation lands in Task 9.

#![allow(dead_code, clippy::unnecessary_literal_bound)]

use cogito_protocol::hook::HookHandler;

/// Placeholder; real impl in Task 9.
pub struct SensitiveContentHook;

impl HookHandler for SensitiveContentHook {
    fn name(&self) -> &str {
        "sensitive-content"
    }
}

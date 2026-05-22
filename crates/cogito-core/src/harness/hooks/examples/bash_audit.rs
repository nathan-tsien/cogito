//! `BashAuditHook` — implementation lands in Task 10.

#![allow(dead_code, clippy::unnecessary_literal_bound)]

use cogito_protocol::hook::HookHandler;

/// Placeholder; real impl in Task 10.
pub struct BashAuditHook;

impl HookHandler for BashAuditHook {
    fn name(&self) -> &str {
        "bash-audit"
    }
}

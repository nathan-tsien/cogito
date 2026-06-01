//! Per-session provider overrides.
//!
//! A `SessionSpec` carries optional overrides applied to one session at
//! open time (`Runtime::open_session_with`) or mid-session
//! (`SessionHandle::update_session`). A `None` field falls back to the
//! Runtime's build-time default provider. See ADR-0028.

use std::sync::Arc;

use cogito_protocol::skill::SkillProvider;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolProvider;

/// Optional per-session provider overrides.
///
/// `Default` is all-`None`, which makes `open_session_with(id, mode,
/// SessionSpec::default())` behave exactly like the legacy
/// `open_session(id, mode)`.
#[derive(Default, Clone)]
pub struct SessionSpec {
    /// Per-session tool provider. `None` → Runtime default.
    pub tools: Option<Arc<dyn ToolProvider>>,
    /// Per-session skill provider. `None` → Runtime default.
    pub skills: Option<Arc<dyn SkillProvider>>,
    /// Per-session strategy. `None` → Runtime default.
    pub strategy: Option<HarnessStrategy>,
    /// Tenant identity stamped into `SessionMeta` at open. Ignored by
    /// `update_session` (identity is fixed at open time).
    pub tenant_id: Option<String>,
    /// User identity stamped into `SessionMeta` at open. Ignored by
    /// `update_session`.
    pub user_id: Option<String>,
}

impl std::fmt::Debug for SessionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionSpec")
            .field("tools", &self.tools.is_some())
            .field("skills", &self.skills.is_some())
            .field("strategy", &self.strategy.as_ref().map(|s| &s.name))
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .finish()
    }
}

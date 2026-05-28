//! Read-only registry of named `HarnessStrategy` bundles.
//!
//! The trait is the protocol-layer seam between the wiring layer
//! (which discovers strategies — from disk in v0.1, from a database
//! or object store in v0.4 `SaaS`) and any consumer that resolves a
//! strategy by name. Brain does NOT depend on this trait in v0.1;
//! the wiring layer (`cogito-cli`) resolves the strategy up-front
//! and hands `RuntimeBuilder` the final `HarnessStrategy` value.
//!
//! See `docs/adr/0026-strategy-registry.md`.

use thiserror::Error;

use crate::strategy::HarnessStrategy;

/// Read-only registry. v0.1 ships an FS-backed impl in `cogito-strategy`;
/// v0.4 `SaaS` adds a DB-backed impl behind the same trait.
pub trait StrategyRegistry: Send + Sync + 'static {
    /// Returns the named strategy. The returned value has `system_prompt`
    /// fully materialized (any `file:` references already resolved).
    ///
    /// # Errors
    ///
    /// Returns `StrategyError::Unknown` if `name` is not registered.
    /// Returns `StrategyError::Validation` for impl-specific
    /// post-load checks.
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError>;

    /// Returns the names of all strategies currently registered.
    /// MUST be sorted ascending and deduplicated.
    fn list(&self) -> Vec<String>;
}

/// Errors surfaced by `StrategyRegistry`. `LoadError` (in `cogito-strategy`)
/// is a strictly-richer cousin used at registry-build time.
#[derive(Error, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StrategyError {
    /// `name` is not registered. `available` is `registry.list()` at
    /// the time of the failed lookup; used by CLI surfaces for
    /// "did you mean" output.
    #[error("strategy `{0}` not found; available: {1:?}")]
    Unknown(String, Vec<String>),

    /// Strategy references a provider that is not in `cogito.toml`.
    /// Detected by the wiring layer when both inputs first meet.
    #[error("strategy `{name}` references missing provider `{provider}`")]
    UnknownProvider {
        /// Name of the offending strategy.
        name: String,
        /// Provider id the strategy referenced.
        provider: String,
    },

    /// Catch-all for impl-specific validation failures.
    #[error("strategy `{name}` validation failed: {reason}")]
    Validation {
        /// Name of the offending strategy.
        name: String,
        /// Human-readable explanation of why validation failed.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::HarnessStrategy;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Smallest possible test impl — proves the trait is dyn-compatible
    /// and shareable as `Arc<dyn StrategyRegistry>`.
    struct StubRegistry(HashMap<String, HarnessStrategy>);

    impl StrategyRegistry for StubRegistry {
        fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
            self.0
                .get(name)
                .cloned()
                .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
        }
        fn list(&self) -> Vec<String> {
            let mut v: Vec<String> = self.0.keys().cloned().collect();
            v.sort();
            v
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let mut m = HashMap::new();
        m.insert("foo".into(), HarnessStrategy::default_with_model("test"));
        let reg: Arc<dyn StrategyRegistry> = Arc::new(StubRegistry(m));
        assert_eq!(reg.list(), vec!["foo"]);
        assert!(reg.get("foo").is_ok());
        assert!(matches!(
            reg.get("missing"),
            Err(StrategyError::Unknown(_, _))
        ));
    }
}

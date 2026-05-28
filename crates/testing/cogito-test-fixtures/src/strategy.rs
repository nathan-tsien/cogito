//! In-memory `StrategyRegistry` + a contract test suite consumed by
//! every concrete impl (FS-backed, future DB-backed).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Justification: this module is test infrastructure. The contract
// helper deliberately panics on contract violation to surface failures
// in the consumer's test runner, and the in-memory `MapStrategyRegistry`
// only ever sees pre-built `HarnessStrategy` values.

use std::collections::BTreeMap;

use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};

/// Simple in-memory registry for tests that don't want to touch disk.
#[derive(Debug, Clone, Default)]
pub struct MapStrategyRegistry {
    inner: BTreeMap<String, HarnessStrategy>,
}

impl MapStrategyRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a strategy by name. Last write wins.
    pub fn insert(&mut self, name: impl Into<String>, strategy: HarnessStrategy) {
        self.inner.insert(name.into(), strategy);
    }

    /// Builder-style helper.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, strategy: HarnessStrategy) -> Self {
        self.insert(name, strategy);
        self
    }
}

impl StrategyRegistry for MapStrategyRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        self.inner
            .get(name)
            .cloned()
            .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
    }

    fn list(&self) -> Vec<String> {
        self.inner.keys().cloned().collect()
    }
}

/// Run the canonical contract suite against any `StrategyRegistry` impl.
///
/// `make_registry` returns a freshly-built registry containing exactly
/// these strategies: `"foo"` (`system_prompt` = "FOO"), `"bar"`
/// (`system_prompt` = "BAR"). The contract is impl-agnostic.
///
/// # Panics
///
/// Panics if the registry under test violates any documented invariant
/// of `StrategyRegistry`.
pub fn strategy_registry_contract<R: StrategyRegistry>(make_registry: impl Fn() -> R) {
    // list() is sorted.
    let reg = make_registry();
    assert_eq!(reg.list(), vec!["bar", "foo"], "list() must be sorted");

    // list() is deduplicated (trivially true for our two-entry case;
    // re-running the contract on a registry with duplicate-name entries
    // is meaningful — kept simple here).
    let reg = make_registry();
    let names = reg.list();
    let mut sorted_dedup = names.clone();
    sorted_dedup.dedup();
    assert_eq!(names, sorted_dedup, "list() must be deduplicated");

    // get() of any name in list() succeeds and is deterministic.
    let reg = make_registry();
    for name in reg.list() {
        let first = reg.get(&name).unwrap();
        let second = reg.get(&name).unwrap();
        assert_eq!(first.name, second.name, "get is deterministic for {name}");
        assert_eq!(
            first.system_prompt, second.system_prompt,
            "get is deterministic for {name}"
        );
    }

    // get() of any name NOT in list() returns Unknown.
    let reg = make_registry();
    let err = reg.get("definitely-not-registered").unwrap_err();
    assert!(
        matches!(err, StrategyError::Unknown(_, _)),
        "expected Unknown, got {err:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use cogito_protocol::strategy::HarnessStrategy;

    fn build_canonical() -> MapStrategyRegistry {
        let mut foo = HarnessStrategy::default_with_model("test");
        foo.name = "foo".into();
        foo.system_prompt = "FOO".into();
        let mut bar = HarnessStrategy::default_with_model("test");
        bar.name = "bar".into();
        bar.system_prompt = "BAR".into();
        MapStrategyRegistry::new().with("foo", foo).with("bar", bar)
    }

    #[test]
    fn map_registry_passes_contract() {
        strategy_registry_contract(build_canonical);
    }
}

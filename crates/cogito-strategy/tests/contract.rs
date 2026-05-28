//! Contract-suite integration test: runs the canonical
//! `strategy_registry_contract` (from `cogito-test-fixtures`) against
//! the FS-backed `FsStrategyRegistry` to prove the impl satisfies the
//! same invariants every future `StrategyRegistry` impl will need.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use cogito_strategy::{FsStrategyRegistry, Scope, ScopeRoot};
use cogito_test_fixtures::strategy::strategy_registry_contract;
use tempfile::TempDir;

fn canonical_fs_registry() -> FsStrategyRegistry {
    // Match the contract's expectation: registry holds exactly "foo" and "bar".
    // Box::leak gives the TempDir a 'static lifetime so the directory survives
    // the closure; this is a deliberate test-only leak.
    let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
    let foo_path = tmp.path().join("foo.md");
    let bar_path = tmp.path().join("bar.md");
    fs::write(&foo_path, "---\nname: foo\n---\nFOO\n").unwrap();
    fs::write(&bar_path, "---\nname: bar\n---\nBAR\n").unwrap();
    FsStrategyRegistry::from_roots(&[ScopeRoot::new(Scope::Repo, tmp.path().to_path_buf())])
        .unwrap()
}

#[test]
fn fs_registry_passes_contract() {
    strategy_registry_contract(canonical_fs_registry);
}

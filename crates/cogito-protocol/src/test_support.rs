//! Shared test infrastructure exposed via the `test-support` feature.
//!
//! Modules under this gate are compiled only when downstream consumers
//! opt in with `cogito-protocol = { workspace = true, features =
//! ["test-support"] }` in their dev-dependencies. They are intentionally
//! excluded from production builds: the harness traits, contract suites,
//! and fixtures here exist solely to let each trait in `cogito-protocol`
//! be exercised against every implementation in the workspace, without
//! either polluting the public API or duplicating tests across crates.
//!
//! Add a new submodule per contract — e.g. [`contract_job_manager`] for
//! [`crate::job::JobManager`] — and have each implementation crate's
//! integration tests drive `run_contract_suite::<MyHarness>()`.

// Contract suites assert via `panic!` / `expect` / `unwrap`; documenting
// every "# Panics" section on every assertion-wrapped helper would just
// be noise, so silence the pedantic lint at the module root.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

pub mod contract_command_executor;
pub mod contract_job_manager;
pub mod contract_workspace;

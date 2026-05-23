//! cogito-test-fixtures — shared test fixtures. Not published.

pub mod chaos_scenarios;
pub mod context;
pub mod fault_store;
pub mod fixtures;
pub mod mock_job_manager;
pub mod store_contract;

pub use chaos_scenarios::{ChaosScenario, all as all_chaos_scenarios};
pub use fault_store::{FaultInjectingStore, FaultTrigger};
pub use fixtures::{
    canonical_sample_jsonl, canonical_sample_session, canonical_skill_jsonl,
    canonical_skill_session, sse_fixture,
};
pub use mock_job_manager::MockJobManager;

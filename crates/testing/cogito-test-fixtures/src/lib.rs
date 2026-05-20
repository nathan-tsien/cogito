//! cogito-test-fixtures — shared test fixtures. Not published.

pub mod fault_store;
pub mod fixtures;
pub mod store_contract;

pub use fault_store::{FaultInjectingStore, FaultTrigger};
pub use fixtures::{canonical_sample_jsonl, canonical_sample_session, sse_fixture};

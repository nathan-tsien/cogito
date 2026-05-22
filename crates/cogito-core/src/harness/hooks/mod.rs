//! H09 Hook Pipeline — composite invoker over `Vec<Arc<dyn HookHandler>>`.
//!
//! The lifecycle trait lives in `cogito-protocol::hook`. This module
//! provides the runtime invocation surface: panic catch, metrics, and
//! the `CompositeHookPipeline` that the FSM transitions call.
//!
//! See `docs/components/H09-hook-pipeline.md`.

pub mod composite;
pub mod examples;
pub mod panic_catch;

pub use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};
pub use composite::CompositeHookPipeline;

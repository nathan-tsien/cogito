//! Context-management trait implementations and assembly factory.
//!
//! Brain (`cogito-core::harness`) sees only `cogito-protocol` traits and
//! interacts with this crate's outputs via `Arc<dyn ...>`. This crate is
//! a Hand-like layer in the ADR-0004 boundary: trait impls and composition.
//!
//! v0.1 ships:
//! - `compactor::none::NoneCompactor`
//! - `compactor::truncate::TruncateCompactor`
//! - `projector::standard::StandardProjector`
//! - `injector::none::NoneInjector`
//! - `overrider::none::NoneOverrider`
//!
//! Sprint 7 adds `injector::skills`; v0.2 adds `compactor::summarize`,
//! `projector::tool_elision`, `overrider::read_only`, etc.

pub mod compactor;
pub mod injector;
pub mod overrider;
pub mod pipeline;
pub mod projector;

pub use cogito_protocol::ContextPipeline;
pub use pipeline::{PipelineBuildError, build_pipeline, build_pipeline_v2};

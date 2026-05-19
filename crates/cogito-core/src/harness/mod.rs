//! Harness: the 10 components that drive one iteration of the agent loop.
//!
//! See `docs/components/H0X-*.md` for per-component design notes.
//! See `ARCHITECTURE.md` for the component dependency rules.

pub mod dispatcher;
pub mod hooks;
pub mod prompt;
pub mod resume;
pub mod step_recorder;
pub mod strategy;
pub mod stream_demux;
pub mod tool_resolver;
pub mod tool_surface;
pub mod turn_driver;

pub use step_recorder::StepRecorder;

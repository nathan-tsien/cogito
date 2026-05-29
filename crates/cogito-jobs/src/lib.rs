//! Local async job manager. Jobs run as `tokio::task`s inside the
//! Runtime process; their lifecycle is mirrored into the conversation
//! event log (`JobSubmitted` / `JobCompletedRecorded`). A Runtime
//! restart loses every running job; the resume coordinator synthesizes
//! `JobOutcome::Failed { message: "lost across process restart" }` for
//! any open job at crash time so Brain unwinds cleanly. True
//! cross-process job survival is a v0.4 SaaS-ready concern.

mod local;
pub use local::LocalJobManager;

mod run_tests;
pub use run_tests::RunTestsTool;

mod bash;
pub use bash::{BashConfig, BashTool};

#[cfg(any(test, feature = "test-tools"))]
mod sleep_tool;
#[cfg(any(test, feature = "test-tools"))]
pub use sleep_tool::SleepTool;

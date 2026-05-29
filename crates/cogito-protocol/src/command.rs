//! `CommandExecutor` — the seam for running a subprocess in a
//! policy-selected environment. Whether execution is isolated is decided
//! by the concrete implementation injected at runtime, NOT by the tool
//! that calls it. v0.4 ADR-0012 (sandbox lifecycle) / ADR-0013
//! (credential isolation) extend this seam with isolating / remote impls.
//!
//! This is a runtime-only trait: it is never serialized into the event
//! log and is not part of the cross-language wire contract, so adding it
//! does not touch `SCHEMA_VERSION`.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::ExecCtx;

/// One command to execute. `env` policy and the root directory are
/// implementation/construction-time concerns (see `SandboxConfig`), so
/// they are deliberately absent here to keep the per-call surface minimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Shell command line. `DirectExecutor` runs it via `sh -c <command>`.
    pub command: String,
    /// Working directory. Relative paths resolve against the executor's
    /// configured root; `None` means "use the root".
    pub cwd: Option<PathBuf>,
    /// Hard wall-clock timeout for this execution.
    pub timeout: Duration,
    /// Per-stream byte budget for stdout/stderr (head + tail kept, middle
    /// elided when exceeded).
    pub max_output_bytes: usize,
}

/// Captured result of a finished (or timed-out) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Captured stdout (possibly truncated; see `truncated`).
    pub stdout: String,
    /// Captured stderr (possibly truncated; see `truncated`).
    pub stderr: String,
    /// Process exit code; `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// `true` when the command was killed because `timeout` elapsed.
    pub timed_out: bool,
    /// `true` when stdout or stderr was truncated to `max_output_bytes`.
    pub truncated: bool,
}

/// Failure modes that prevent producing a `CommandOutcome`.
///
/// A non-zero exit code is NOT an error — it is a normal `CommandOutcome`
/// with `exit_code = Some(n)`. A timeout is also not an error — it is a
/// `CommandOutcome` with `timed_out = true` and whatever output was
/// captured before the kill. Only spawn failure and cooperative
/// cancellation surface here.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandError {
    /// The process could not be spawned (binary missing, permission, …).
    #[error("failed to spawn command: {0}")]
    Spawn(String),
    /// `ExecCtx::cancel` fired; the child was killed and execution aborted.
    #[error("command cancelled")]
    Cancelled,
}

/// Abstraction over "run a subprocess". Implementations live in
/// `cogito-sandbox` (v0.1: `DirectExecutor`) and are injected into tools
/// (e.g. `bash`) at construction time. Brain / H08 never see this trait.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute `spec`, honoring `ctx.cancel`. Returns the captured outcome,
    /// or a `CommandError` for spawn failure / cancellation.
    async fn run(&self, spec: CommandSpec, ctx: ExecCtx) -> Result<CommandOutcome, CommandError>;
}

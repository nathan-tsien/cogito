//! Tool contract: descriptor, invocation outcome, result, error kinds.
//!
//! See:
//! - `docs/components/H07-tool-resolver.md` (descriptor and validation)
//! - `docs/components/H08-tool-dispatcher.md` (invocation flow)
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` §6
//!   (sync vs async judgment via `ExecutionClass`)

use serde::{Deserialize, Serialize};

// TODO(Task 8): replace with `crate::job::JobId` once that module lands.
/// Placeholder for the `JobId` type; Task 8 of the Sprint 0 plan replaces
/// this with a real Ulid-backed identifier in the `job` module.
///
/// `JobIdStub::default()` returns the all-zero id. The real `JobId`
/// will return a freshly-generated Ulid from its `Default` impl, so
/// any code relying on `Default` semantics needs updating in Task 8.
#[deprecated(
    since = "0.1.0",
    note = "Sprint 0 Task 8 placeholder; use cogito_protocol::job::JobId once Task 8 lands. Do not depend on this in downstream code."
)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobIdStub(pub u64);

/// A tool exposed by a `ToolProvider`. `ToolDescriptor` is the metadata the
/// LLM (and H05 Tool Surface Builder) sees; the actual call goes through
/// `ToolProvider::invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Unique name; the LLM uses this in tool calls.
    pub name: String,
    /// One-line description shown to the model.
    pub description: String,
    /// JSON Schema (Draft 2020-12) for the arguments.
    pub schema: serde_json::Value,
    /// Whether invocations are sync, async, or per-call adaptive.
    pub execution_class: ExecutionClass,
    /// If `true`, this tool may emit Image/Video/Audio `ContentBlock`s in its
    /// result. H05 may filter the tool out when the selected model has no
    /// native multimodal capability.
    pub outputs_model_visible_multimodal: bool,
}

/// Statically-declared execution class for a tool. H08 uses this to validate
/// the `InvokeOutcome` variant returned by `invoke()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    /// Always returns `InvokeOutcome::Sync`. Typical: `read_file`, `now`,
    /// `parse_json`.
    AlwaysSync,
    /// Always returns `InvokeOutcome::Async(JobId)`. Typical: `run_tests`,
    /// `build_release`.
    AlwaysAsync,
    /// Decides per call based on arguments. Typical: `transcribe_audio`
    /// (short clip -> Sync; long clip -> Async).
    Adaptive,
}

/// Outcome of a single `ToolProvider::invoke` call.
#[allow(deprecated)] // JobIdStub usage; remove with Task 8
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InvokeOutcome {
    /// Result is available immediately.
    Sync(ToolResult),
    /// Result is deferred; consult `JobManager` for completion. The Brain
    /// will pause the turn until a matching `JobCompletionEvent` arrives.
    ///
    /// TODO(Task 8): replace `JobIdStub` with `crate::job::JobId`.
    Async(JobIdStub),
}

/// Result body returned by a tool. `Vec<ContentBlock>` arrives in v0.2 when
/// the multimodal upgrade lands; v0.1 uses plain text via the convenience
/// constructor [`ToolResult::text`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResult {
    /// Successful output. v0.1 represents content as a list of opaque
    /// JSON values; v0.2 replaces this with `Vec<ContentBlock>`.
    Output(Vec<serde_json::Value>),
    /// Structured error. H08 records this as the tool's result, then feeds
    /// it back to the model so the model can decide how to proceed.
    Error {
        /// Classification of the failure reason.
        kind: ToolErrorKind,
        /// Human-readable error message for the model and operator.
        message: String,
        /// Whether it is safe for the caller to retry the invocation.
        retryable: bool,
    },
}

impl ToolResult {
    /// Convenience constructor for the common "single text block" case.
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        ToolResult::Output(vec![serde_json::Value::String(s.into())])
    }
}

/// Classification of why a tool call failed. The model only ever sees
/// `ToolResult::Error`; this kind helps H09 hooks and H10 strategy decide
/// whether to retry or surface to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    /// H07 schema validation rejected the args.
    InvalidArgs,
    /// Tool returned a business-logic error.
    InvocationFailed,
    /// The tool implementation panicked; caught at H08 Layer 3
    /// (see spec §9).
    ToolPanicked,
    /// Cancellation token fired during the invocation.
    Cancelled,
    /// Tool-internal timeout (distinct from turn-level timeout, which
    /// produces `TurnFailureReason::TurnTimedOut`).
    Timeout,
    /// Resuming a paused turn: `JobManager` lost track of the `JobId`.
    JobStateLost,
    /// Async job completed but reported an internal failure.
    AsyncFailed,
}

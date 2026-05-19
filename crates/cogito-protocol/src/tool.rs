//! Tool contract: descriptor, invocation outcome, result, error kinds.
//!
//! See:
//! - `docs/components/H07-tool-resolver.md` (descriptor and validation)
//! - `docs/components/H08-tool-dispatcher.md` (invocation flow)
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` §6
//!   (sync vs async judgment via `ExecutionClass`)

use serde::{Deserialize, Serialize};

use crate::job::JobId;

/// A tool exposed by a `ToolProvider`. `ToolDescriptor` is the metadata the
/// LLM (and H05 Tool Surface Builder) sees; the actual call goes through
/// `ToolProvider::invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
///
/// Marked `#[non_exhaustive]` because this type is part of the cross-language
/// wire contract (per ADR-0007) and reaches external readers via
/// `EventPayload`. Reserving the variant set lets future versions add
/// classes (e.g. an `Either` variant for tools whose stream-or-async choice
/// depends on the call site) without breaking downstream `match` arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
///
/// Marked `#[non_exhaustive]` because this type is part of the cross-language
/// wire contract (per ADR-0007) and is reachable from `EventPayload` via
/// `ToolResult`. Reserving the variant set lets future versions add new
/// dispatch shapes (e.g. streaming outputs) without breaking downstream
/// `match` arms.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub enum InvokeOutcome {
    /// Result is available immediately.
    Sync(ToolResult),
    /// Result is deferred; consult `JobManager` for completion. The Brain
    /// will pause the turn until a matching `JobCompletionEvent` arrives.
    Async(JobId),
}

/// Result body returned by a tool. `Vec<ContentBlock>` arrives in v0.2 when
/// the multimodal upgrade lands; v0.1 uses plain text via the convenience
/// constructor [`ToolResult::text`].
///
/// Note: `PartialEq` is derived but not `Eq` because `serde_json::Value`
/// (used in the `Output` variant) does not implement `Eq`.
///
/// Marked `#[non_exhaustive]` because this type is part of the cross-language
/// wire contract (per ADR-0007) and reaches external readers via
/// `EventPayload::ToolResultRecorded`. The v0.2 multimodal upgrade swaps
/// `Output(Vec<serde_json::Value>)` for `Output(Vec<ContentBlock>)`; new
/// classifications (e.g. an explicit `Streaming` variant) may follow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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

/// Brain-facing contract for any component that exposes callable tools.
///
/// Implementations live in `cogito-tools` (builtin), `cogito-mcp` (v0.2+),
/// and `cogito-subagent` (v0.3+). Brain holds an `Arc<dyn ToolProvider>`
/// injected at Runtime construction; it never imports concrete crates directly
/// (ADR-0004 layer rule).
#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    /// Return metadata for every tool this provider exposes. H05 (Tool Surface
    /// Builder) calls this once per turn to populate the model's tool schema.
    fn list(&self) -> Vec<ToolDescriptor>;

    /// Invoke a single tool by name. `args` is the raw JSON the model emitted
    /// for this call; the implementation validates and executes it.
    ///
    /// Implementations MUST NOT panic — all failures must be returned as
    /// `InvokeOutcome::Sync(ToolResult::Error { ... })`.
    async fn invoke(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: crate::ExecCtx,
    ) -> InvokeOutcome;
}

/// Classification of why a tool call failed. The model only ever sees
/// `ToolResult::Error`; this kind helps H09 hooks and H10 strategy decide
/// whether to retry or surface to the user.
///
/// Marked `#[non_exhaustive]` so v0.2+ multimedia / MCP / subagent tools
/// can introduce new failure shapes (e.g. `StorageUnavailable`,
/// `McpServerDisconnected`) without breaking downstream `match` arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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

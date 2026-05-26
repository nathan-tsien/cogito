//! Brain-facing `ToolProvider` implementation that holds a fixed set of
//! builtin tools constructed at process startup.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_jobs::{LocalJobManager, RunTestsTool};
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{
    InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};

/// One builtin tool exposed via `BuiltinToolProvider`. Concrete tools live
/// in `crate::builtins::*`.
#[async_trait]
pub trait BuiltinTool: Send + Sync {
    /// Stable metadata. Constructed lazily and cached by the provider.
    fn descriptor(&self) -> ToolDescriptor;

    /// Execute the tool. Implementations must NEVER panic — turn unrecoverable
    /// failures into `ToolResult::Error { kind: InvocationFailed, ... }`.
    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult;
}

/// A `ToolProvider` that wraps a fixed set of builtin tools.
///
/// Construct via the `builder()` -> `with_tool()` -> `build()` pattern so
/// the descriptor cache is computed once.
///
/// When async tools (e.g. `RunTestsTool`) are registered, the surface
/// also calls `with_jobs(arc)` on the builder, handing the provider a
/// concrete `Arc<LocalJobManager>` clone. The provider stashes it so
/// individual tool implementations can call `LocalJobManager::submit`
/// (which is intentionally not on the `JobManager` trait — only async
/// tools submit). The CLI threads the SAME `Arc` into the
/// `RuntimeBuilder` via `job_manager`, so the tool's `submit` and the
/// Brain's `on_complete` agree on which manager holds the `JobId`s.
pub struct BuiltinToolProvider {
    tools: HashMap<String, Arc<dyn BuiltinTool>>,
    descriptors: Vec<ToolDescriptor>,
    /// Async tool wrapping `cargo nextest run`. Constructed in `build()`
    /// when [`BuiltinToolProviderBuilder::with_jobs`] supplied a
    /// `LocalJobManager`. When `None`, `run_tests` is not exposed and
    /// any `invoke("run_tests", ..)` falls through to the sync table
    /// (which will report "unknown tool").
    run_tests: Option<Arc<RunTestsTool>>,
}

impl BuiltinToolProvider {
    /// Begin a builder.
    #[must_use]
    pub fn builder() -> BuiltinToolProviderBuilder {
        BuiltinToolProviderBuilder::default()
    }
}

/// Builder for `BuiltinToolProvider`. Order of `with_tool` calls determines
/// the descriptor cache order.
#[derive(Default)]
pub struct BuiltinToolProviderBuilder {
    tools: Vec<Arc<dyn BuiltinTool>>,
    job_mgr: Option<Arc<LocalJobManager>>,
}

impl BuiltinToolProviderBuilder {
    /// Register one builtin tool.
    #[must_use]
    pub fn with_tool(mut self, tool: Arc<dyn BuiltinTool>) -> Self {
        debug_assert!(
            !tool.descriptor().name.starts_with("mcp__"),
            "builtin tool names must not start with `mcp__` (ADR-0018 §4)"
        );
        self.tools.push(tool);
        self
    }

    /// Hand the provider an `Arc<LocalJobManager>` clone for async tools
    /// to submit against. Required only when at least one registered
    /// tool kicks off async work; sync tools ignore the handle.
    ///
    /// The argument is the concrete `Arc<LocalJobManager>` rather than
    /// `Arc<dyn JobManager>` because async tools need `submit`, which
    /// is intentionally not on the trait. The same `Arc` MUST be
    /// threaded into `RuntimeBuilder::job_manager` or async tool calls
    /// will hang (the tool's `JobId` would not be visible to the Brain
    /// registering `on_complete`).
    #[must_use]
    pub fn with_jobs(mut self, job_mgr: Arc<LocalJobManager>) -> Self {
        self.job_mgr = Some(job_mgr);
        self
    }

    /// Finalize the provider, building the descriptor cache.
    ///
    /// When `with_jobs` was set, a [`RunTestsTool`] is registered
    /// implicitly; its descriptor is appended after the sync tools
    /// supplied via `with_tool`. The provider then dispatches
    /// `run_tests` invocations directly to the async tool's
    /// `ToolProvider::invoke`, returning [`InvokeOutcome::Async`].
    #[must_use]
    pub fn build(self) -> BuiltinToolProvider {
        let mut tools = HashMap::with_capacity(self.tools.len());
        let mut descriptors = Vec::with_capacity(self.tools.len() + 1);
        for t in self.tools {
            let d = t.descriptor();
            descriptors.push(d.clone());
            tools.insert(d.name.clone(), t);
        }
        let run_tests = self.job_mgr.map(|mgr| Arc::new(RunTestsTool::new(mgr)));
        if let Some(rt) = run_tests.as_ref() {
            descriptors.extend(rt.list());
        }
        BuiltinToolProvider {
            tools,
            descriptors,
            run_tests,
        }
    }
}

#[async_trait]
impl ToolProvider for BuiltinToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        // Async-tool branch: when the consumer wired a `LocalJobManager`
        // via `with_jobs`, dispatch `run_tests` to the embedded
        // `RunTestsTool` so it can return `InvokeOutcome::Async(JobId)`.
        // Sync `BuiltinTool`s cannot model async outcomes, so this
        // dispatch is out-of-band rather than going through `self.tools`.
        if let Some(rt) = &self.run_tests {
            if rt.list().iter().any(|d| d.name == name) {
                return rt.invoke(name, args, ctx).await;
            }
        }
        match self.tools.get(name) {
            Some(t) => InvokeOutcome::Sync(t.invoke(args, ctx).await),
            None => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("unknown tool: {name}"),
                retryable: false,
            }),
        }
    }
}

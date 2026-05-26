# ADR-0025: Hands sub-layer boundary — JobManager / ToolProvider / internal primitive

## Status

Accepted (2026-05-26).

## Context

ADR-0004 establishes the top-level Brain / Hands / Session / Boundary layers and forbids the Brain (`cogito-core::harness`) from importing concrete Hands crates. It does **not** specify how the Hands layer is decomposed internally.

Sprint 8 (PR #23) shipped `LocalJobManager` (in `cogito-jobs`) together with two async tools — `RunTestsTool` (production) and `SleepTool` (test fixture) — that submit work to it. Because `LocalJobManager::submit` is a generic method (it takes `F: Future<Output = JobOutcome>`), it cannot live on the dyn-compatible `JobManager` trait. As a result, the tools held a concrete `Arc<LocalJobManager>` and `cogito-tools` was forced to depend on `cogito-jobs` to thread that handle through `BuiltinToolProvider::with_jobs`. The provider then carried a `run_tests: Option<Arc<RunTestsTool>>` field and a special-case branch in `invoke`. A second async tool would have required the same pattern, with the dispatch table forking inside what is supposed to be a generic sync framework.

The classification axis was wrong. "Sync vs async" is not a Hands-layer boundary — `ToolProvider::invoke` is already async, and `InvokeOutcome::{Sync, Async}` is the only place the distinction matters (the H08 dispatcher). The actual sub-layers inside Hands are:

1. **JobManager implementations** — concurrency primitives for async work. `LocalJobManager` is the v0.1 instance; `SqliteJobManager` is the v0.4 instance. Tools never sit here.
2. **ToolProvider implementations** — concrete providers that expose tools to the Brain. May be sync-only (`cogito-tools::BuiltinToolProvider`), async (`cogito-jobs::RunTestsTool`), or a mix. May depend on a `JobManager` *trait object*.
3. **Internal primitives** — subprocess sandbox (`cogito-sandbox`), storage backends (`cogito-storage-local`). Consumed by ToolProvider crates; never expose `ToolProvider` themselves.
4. **Composition** — happens at the Surface layer (`cogito-cli`, consumer services) via `CompositeToolProvider`. The Surface is the only place that knows the full tool inventory.

## Decision

**1. No Hands-internal crate may depend on another Hands-internal crate, with one exception.** The exception is: any ToolProvider crate may depend on `cogito-protocol` for the `JobManager` / `LocalJobSubmitter` traits *only*. Tools take `Arc<dyn LocalJobSubmitter>`, never a concrete `Arc<LocalJobManager>`. Internal primitives (`cogito-sandbox`, `cogito-storage-local`) are consumed via their own public APIs.

**2. Submission is bounded by trait `LocalJobSubmitter`** in `cogito-protocol::job`:

```rust
#[async_trait]
pub trait LocalJobSubmitter: JobManager {
    /// Submit a boxed future as an async job. Resolves to the new `JobId`
    /// once the lifecycle entry is recorded; the future is driven on the
    /// ambient Tokio runtime.
    ///
    /// `BoxFuture<'static>` rather than a generic `F: Future` so this
    /// method is dyn-compatible. The single `Box::pin` per submission is
    /// negligible against actual tool execution cost.
    async fn submit_boxed(
        self: Arc<Self>,
        fut: BoxFuture<'static, JobOutcome>,
    ) -> JobId;
}
```

`LocalJobSubmitter` extends `JobManager`. The receiver is `self: Arc<Self>` (an object-safe receiver kind) so the trait object form `Arc<dyn LocalJobSubmitter>` works, and the implementation gets a strong reference it can hand to a spawned task without needing a `Weak<Self>` back-ref or `unsafe`. Callers invoke as `self.job_mgr.clone().submit_boxed(...).await`.

v0.4 distributed backends do NOT implement this trait — they will introduce a parallel `RemoteJobSubmitter { submit(spec: JobSpec) -> Result<JobId, JobError> }` whose payload is serializable. Tools that want to be deployable both locally and remotely take `Arc<dyn JobSubmitter>` (a third super-trait, to be introduced if and when needed); v0.1 tools may be `LocalJobSubmitter`-only.

**3. `BuiltinToolProvider` is sync-only.** Its `BuiltinTool` trait returns `ToolResult` (not `InvokeOutcome`), so it structurally cannot express async work. Async tools implement `ToolProvider` directly (as `RunTestsTool` already does). `BuiltinToolProvider::with_jobs` and the embedded `run_tests` special-case are deleted.

**4. Composition happens at the Surface.** `cogito-cli` builds the tool inventory by passing `[Arc<BuiltinToolProvider>, Arc<RunTestsTool>, Arc<McpToolProvider>, …]` to `CompositeToolProvider::new(.., NamingPolicy::Strict)`. No Hands-internal crate special-cases another Hands-internal crate's tools.

**5. File placement of tools follows dependency, not sync/async.** A tool lives in whichever Hands crate owns the dependencies it requires:
- `ReadFile` → `cogito-tools` (sync, std-only).
- `RunTestsTool` → `cogito-jobs` (needs `LocalJobSubmitter` + `tokio::process`).
- `SleepTool` → `cogito-jobs` (needs `LocalJobSubmitter`; kept behind `test-tools` feature).
- Future `RunInSandbox` → `cogito-tools-execution` (new crate, depends on `cogito-sandbox` + `cogito-protocol`).

A tool MAY change crate when its dependencies change; this is not a layering violation.

## Consequences

**Positive:**
- `cogito-tools` reverts to pure sync builtins + composition utility; no transitive `tokio::process` pull-in for consumers who only want sync tools.
- Adding a new async tool is `Arc<NewTool>` in CLI's composite — no edits to `cogito-tools` or `cogito-jobs`.
- `LocalJobSubmitter` is a clean, dyn-compatible API: tests can mock it; future backends can wrap it. The v0.4 remote-submitter shape is preserved as a separate trait, no migration debt.
- Crate-level acyclicity: `cogito-tools ⊥ cogito-jobs`, both depend only on `cogito-protocol`.

**Negative:**
- One extra `Box::pin` per async submission. Cost is two pointer writes; orders of magnitude under the cost of the tool's actual work. Acceptable.
- `cogito-jobs` retains a small amount of "this is a tool, not a JobManager" code (`RunTestsTool`, `SleepTool`). The CLAUDE.md workspace-table description ("JobManager impl") becomes slightly stale; the table is informational and the ADR is canonical.
- Surface code (CLI, consumer services) must explicitly register every async tool. There is no convenient "give me all builtins" facade. This is a feature: tool inventory becomes an explicit configuration decision.
- **Future trait extension for subprocess-cancel-orphan fix.** `LocalJobManager::cancel` today aborts the spawned task at its next `.await`, which orphans any OS subprocesses the task spawned (see `TODO(subprocess-cancel-orphan)` in `cogito-jobs/src/{local.rs, run_tests.rs}`). The proper fix — per-job `CancellationToken` signaled before the abort — needs the tool's spawned future to listen on that token. With `submit_boxed` taking just a `BoxFuture`, the natural shape is to widen the signature to a struct (`JobSpec { fut, cancel }`) so the manager can mint and own the token. This is an additive trait extension, not a migration: existing callers add one struct field; `submit_boxed` stays object-safe.

## Alternatives considered

1. **Move all tools into `cogito-tools` behind feature gates.** Rejected: forces every consumer to track per-tool features; `cogito-tools` becomes a kitchen sink; sandbox-driven tools would still need their own home.
2. **Split sync vs async into separate crates (`cogito-tools-sync`, `cogito-tools-async`).** Rejected: sync/async is an implementation detail of `ToolProvider::invoke`'s return value, not a layering axis. The split would multiply with every new dimension (sandboxed, MCP, subagent).
3. **Keep `cogito-tools → cogito-jobs` dependency, accept it as a layering pragma.** Rejected: forces every subsequent async tool to special-case inside `BuiltinToolProvider`; ADR-0004's spirit ("Brain only sees traits") is violated by the back door.

## References

- ADR-0004 (Brain / Hands / Session / Boundary).
- ADR-0007 (event log as cross-language contract — informs why `JobSubmitted` is additive).
- Sprint 8 design spec: `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`.
- PR #23 code review thread.

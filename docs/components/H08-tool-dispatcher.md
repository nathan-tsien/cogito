# H08 · Tool Dispatcher

> **Status**: v0.1 complete · Sprint 8 (sync + async paths). The async path was wired in Sprint 8: `InvokeOutcome::Async(job_id)` causes H08 to record `JobSubmitted { call_id, job_id, tool_name }`, register `on_complete`, and signal H01 to transition to `Paused`. The turn resumes on the corresponding `JobCompletionEvent`. v0.2 Sprint 11 adds one behavior: H08 now populates `ExecCtx.call_id` (the model's tool-call id) before each `invoke`, enabling the `delegate` subagent tool to record child-side parent-call linkage.

## Role in Harness

Invoke `ToolProvider` for each resolved tool call and route the FSM based
on the returned `InvokeOutcome`. The **only** component in Brain that calls
Hands.

## Interface (design level)

- `dispatch(call: ToolInvocation, provider: &dyn ToolProvider, ctx: ExecCtx) -> DispatchOutcome`
- `DispatchOutcome` is **harness-internal** (lives in
  `cogito-core::harness::dispatcher`), not in `cogito-protocol`. It is
  consumed only by `transitions::tool_dispatching` to decide the next
  `TurnState`.
- `DispatchOutcome::SyncResult(ToolResult)` — proceed to next call (or back to `PromptBuilt`)
- `DispatchOutcome::AsyncJob(JobId)` — turn transitions to `Paused`;
  resumes on the corresponding `JobCompletionEvent`. Wired in Sprint 8
  alongside `cogito-jobs::LocalJobManager` and the first `AlwaysAsync`
  tool (`RunTestsTool`).

## Dependencies

**Calls (out)**:
- `ToolProvider::invoke(name, args, ctx) -> InvokeOutcome` — the entire Hands surface (per ADR-0004; Brain never sees `Sandbox` etc.). Since Sprint 11, H08 sets `ctx.call_id = Some(<this call's id>)` before `invoke`, so tools can record parent-call linkage. The `delegate` subagent tool (`AlwaysSync`, in `cogito-core::runtime::subagent`) relies on this together with `ExecCtx.brain_spawner`. File tools (`read_file`, `write_file`, `list_dir`, `edit`, `grep`) read `ExecCtx.workspace` (ADR-0030 / ADR-0031) — the per-session working tree — and return a structured error when it is `None`. Paths are workspace-relative; absolute / escaping paths surface as `ToolResult::Error { kind: InvalidArgs }`. `bash` resolves its cwd against the same workspace root (ADR-0031 §5).
- H02 Step Recorder — for `ToolDispatched`, `ToolResultRecorded`, `JobSubmitted` events
- H09 Hook Pipeline — `pre_dispatch` hook point (may `Reject` the call)

**Called by**: H01 Turn Driver, at `ToolDispatching`.

## Critical invariants

1. **Catches tool-implementation panics.** Wraps the `invoke` call in `catch_unwind` (or equivalent), emitting `ToolResult::Error { kind: InternalPanic, message: <panic_payload> }`. A panicked tool fails *that call*, never the turn-driving task itself.
2. **Respects `ExecCtx.deadline` and `ExecCtx.cancel`.** If deadline expires or cancel fires during a sync invocation, returns `ToolResult::Error { kind: Timeout }` or `Cancelled`. (Co-operative cancellation requires the tool implementation to check the token; sandboxed subprocesses get SIGKILL on deadline.)
3. **Records dispatch and result as separate events.** `ToolDispatched { call_id, name }` is recorded *before* the `invoke` call; `ToolResultRecorded { call_id, result }` *after*. This makes H03's resume decision unambiguous.
4. **Sequential in v0.1.** Multiple tool calls in one turn are dispatched one-at-a-time, in the order the model emitted them. Parallel dispatch is a 0.x option gated by `strategy.parallel_dispatch: bool`.
5. **Async outcomes pause the turn.** `InvokeOutcome::Async(job_id)` causes H08 to record `JobSubmitted { call_id, job_id, tool_name }`, signal H01 to transition to `Paused`, and return. The turn doesn't re-enter `ToolDispatching` until a `JobCompletionEvent` arrives (the Runtime layer subscribes to `JobManager` and writes that event).
6. **`JobSubmitted` is recorded before `on_complete` is registered.** H08 writes `JobSubmitted` first, then calls `JobManager::on_complete(job_id, sink)`. A crash between the two leaves the event log in a recoverable state: H03 sees a paused turn whose `job_id` is unknown to the freshly-restarted in-memory `LocalJobManager`, and the resume coordinator synthesizes a `JobOutcome::Failed` so the turn drains instead of pausing forever. The reverse order (register first, record after) would lose the `call_id ↔ job_id` mapping if the actor crashed in between.

## Failure-to-result mapping

All failure modes surface as `ToolResult::Error` variants, never as panics
or propagated `Err`s reaching H01:

| Failure source | `ToolResult::Error.kind` |
|---|---|
| Tool impl panic | `InternalPanic` |
| Deadline exceeded | `Timeout` |
| Cancellation token fired | `Cancelled` |
| Async job failed (after resume) | `AsyncFailed` |
| Hook `pre_dispatch` returned `Reject` | `BlockedByHook` |
| Provider returned its own structured error | `ToolError { provider_kind, message }` |

The full enum is defined in `cogito-protocol::hands::ToolResult`.

## v0.1 scope

- Sequential dispatch only
- Sync, async, **and `Adaptive`** all work. Since Sprint 8 the dispatcher
  routes purely by the **actual `InvokeOutcome` returned per call**
  (`Sync` -> `SyncResult`, `Async` -> pause); it does not validate the
  outcome against the descriptor's `ExecutionClass`, which is now only a
  **surface advisory** (e.g. H05 filtering). An `Adaptive` tool that
  returns `Sync` or `Async` per call therefore works with **no dispatcher
  change**. `bash` (Sprint 10) is the first `Adaptive` tool. (An earlier
  draft of this doc said "Adaptive deferred"; that was pre-Sprint-8 and is
  no longer true.)
- `pre_dispatch` hook supported; modify-args is a 0.x option
- Async path: at most one outstanding async dispatch per turn (turn pauses immediately on `InvokeOutcome::Async`); multi-async-dispatch is a post-v0.1 option

`bash`'s sync vs background dual path lives **inside the tool**: it decides
per call whether to `executor.run(...).await` synchronously or to submit a
background job, and returns the matching `InvokeOutcome`. The injected
`CommandExecutor` is a tool-internal detail; H08 never sees it (see
ADR-0027 §"Two-layer model").

## Open design questions

- Parallel dispatch ordering: when two parallel async jobs both complete, which `JobCompleted` event arrives first? Need a deterministic merge / FIFO at the Runtime → event-log boundary. Initial v0.1: not relevant (sequential).
- "Speculative" dispatch (start tool A before model has finished emitting tool B)? Out of scope.

## Testing strategy

- **Unit**: each failure-to-result mapping, exercised with mocked `ToolProvider`.
- **Integration**: full dispatch loop with `BuiltinToolProvider` against a `read_file` tool; verify the event sequence on success, on panic, on timeout, on cancel.
- **Chaos**: crash injection between `ToolDispatched` and `ToolResultRecorded`; H03 must correctly resume with the call marked as needing re-dispatch. The Sprint 8 `paused_async_job` scenario additionally injects crashes between `JobSubmitted` and `TurnPaused`, and between `TurnPaused` and `JobCompletedRecorded`; the lost-job synthesis path produces a `JobOutcome::Failed` so the turn drains.

## References

- ARCHITECTURE.md §"Hands layer internal structure"
- ARCHITECTURE.md §"Tool execution classes"
- ADR-0004 §3 (Hands traits in protocol)
- AGENTS.md §"Inviolable design principles" #5
- `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`
  (Sprint 8 async path: `JobSubmitted` event, record-then-register
  ordering, lost-job synthesis, `paused_async_job` chaos scenario)

## Implementation note (v0.1)

H08 branches on **one** signal: the `InvokeOutcome` returned by
`ToolProvider::invoke` (`Sync(ToolResult)` -> `SyncResult`;
`Async(JobId)` -> record `JobSubmitted`, register the completion sink,
return `AsyncJob`). It does **not** consult `ToolDescriptor.execution_class`
before invoking, nor does it validate the returned outcome against the
declared class — `execution_class` is a surface advisory only. A future
`InvokeOutcome` variant the dispatcher does not understand surfaces as a
structured `ToolResult::Error { kind: InvocationFailed }`. Strategy
filtering (e.g., `allow_async_tools: false`) is H05's responsibility, not
H08's; H08 trusts the outcome it receives.

This is why `Adaptive` tools need no special handling: `bash` returns
`Sync` for foreground commands and `Async` for `background:true`, and the
dispatcher routes each call by what it actually got back.

Cancellation: each `invoke()` call runs inside
`tokio::select!(provider.invoke(...), ctx.cancel.cancelled())`. On
cancel, in-flight tool futures are *dropped on next yield*
(cooperative) — cogito does not `task.abort()` them, leaving cleanup
to the tool's RAII. Tools that want to honor cancel must `select!` on
`ctx.cancel` internally.

Panic isolation: each `invoke()` is wrapped in `catch_unwind` (Layer 3
of the three-layer panic isolation described in
`docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
§9). A panicking tool surfaces as `ToolResult::Error { kind:
ToolPanicked }`; the turn continues.

See spec §6 for the full sync/async judgment table and §9 for
cancellation + panic propagation.

# H08 · Tool Dispatcher

> **Status**: 🚧 Not implemented · Sprint 4 (sync path in Sprint 2 as part of minimal loop)

## Role in Harness

Invoke `ToolProvider` for each resolved tool call and route the FSM based
on the returned `InvokeOutcome`. The **only** component in Brain that calls
Hands.

## Interface (design level)

- `dispatch(call: ToolInvocation, provider: &dyn ToolProvider, ctx: ExecCtx) -> DispatchOutcome`
- `DispatchOutcome::SyncResult(ToolResult)` — proceed to next call (or back to `PromptBuilt`)
- `DispatchOutcome::AsyncJob(JobId)` — turn transitions to `Paused`; resumes on `JobCompleted` event

## Dependencies

**Calls (out)**:
- `ToolProvider::invoke(name, args, ctx) -> InvokeOutcome` — the entire Hands surface (per ADR-0004; Brain never sees `Sandbox` etc.)
- H02 Step Recorder — for `ToolDispatched`, `ToolResultRecorded`, `JobSubmitted` events
- H09 Hook Pipeline — `pre_dispatch` hook point (may `Reject` the call)

**Called by**: H01 Turn Driver, at `ToolDispatching`.

## Critical invariants

1. **Catches tool-implementation panics.** Wraps the `invoke` call in `catch_unwind` (or equivalent), emitting `ToolResult::Error { kind: InternalPanic, message: <panic_payload> }`. A panicked tool fails *that call*, never the turn-driving task itself.
2. **Respects `ExecCtx.deadline` and `ExecCtx.cancel`.** If deadline expires or cancel fires during a sync invocation, returns `ToolResult::Error { kind: Timeout }` or `Cancelled`. (Co-operative cancellation requires the tool implementation to check the token; sandboxed subprocesses get SIGKILL on deadline.)
3. **Records dispatch and result as separate events.** `ToolDispatched { call_id, name }` is recorded *before* the `invoke` call; `ToolResultRecorded { call_id, result }` *after*. This makes H03's resume decision unambiguous.
4. **Sequential in v0.1.** Multiple tool calls in one turn are dispatched one-at-a-time, in the order the model emitted them. Parallel dispatch is a 0.x option gated by `strategy.parallel_dispatch: bool`.
5. **Async outcomes pause the turn.** `InvokeOutcome::Async(job_id)` causes H08 to record `JobSubmitted { call_id, job_id }`, signal H01 to transition to `Paused`, and return. The turn doesn't re-enter `ToolDispatching` until a `JobCompleted` event arrives (the Runtime layer subscribes to `JobManager` and writes that event).

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
- Sync (class A) and async (class D) coverage; classes B/C/E/F deferred
- `pre_dispatch` hook supported; modify-args is a 0.x option

## Open design questions

- Parallel dispatch ordering: when two parallel async jobs both complete, which `JobCompleted` event arrives first? Need a deterministic merge / FIFO at the Runtime → event-log boundary. Initial v0.1: not relevant (sequential).
- "Speculative" dispatch (start tool A before model has finished emitting tool B)? Out of scope.

## Testing strategy

- **Unit**: each failure-to-result mapping, exercised with mocked `ToolProvider`.
- **Integration**: full dispatch loop with `BuiltinToolProvider` against a `read_file` tool; verify the event sequence on success, on panic, on timeout, on cancel.
- **Chaos**: crash injection between `ToolDispatched` and `ToolResultRecorded`; H03 must correctly resume with the call marked as needing re-dispatch.

## References

- ARCHITECTURE.md §"Hands layer internal structure"
- ARCHITECTURE.md §"Tool execution classes"
- ADR-0004 §3 (Hands traits in protocol)
- AGENTS.md §"Inviolable design principles" #5

## Implementation note (v0.1)

H08 branches on two signals:

1. `ToolDescriptor.execution_class`
   (`AlwaysSync` / `AlwaysAsync` / `Adaptive`) — checked before invoke
2. `InvokeOutcome` returned by `ToolProvider::invoke`
   (`Sync(ToolResult)` / `Async(JobId)`)

Contract violations (e.g., a tool descriptor declared `AlwaysSync`
returning `Async`) are `debug_assert!`s in dev builds and a structured
`ToolResult::Error { kind: InvocationFailed }` in release. Strategy
filtering (e.g., `allow_async_tools: false`) is H05's responsibility,
not H08's; H08 trusts the descriptor it receives.

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

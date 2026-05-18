# ADR-0006: Runtime + H01 Turn Driver execution model

## Status

Accepted (2026-05-18)

## Context

ADR-0003 established that the Turn Driver is an explicit state machine and
ADR-0004 established that Brain, Hands, Session, and Runtime are distinct
layers. Neither ADR answered the operational question: *how does the state
machine actually run on tokio?*

Specifically, three gaps remained:

1. **Concurrency model**: is a session a shared `Arc<Mutex<_>>` or an isolated
   task? At ≥1000 concurrent sessions per process (ADR-0005 §3 SLO target), the
   concurrency primitive is a load-bearing choice, not an implementation detail.
2. **tokio handle ownership**: cogito is an *embedded library* (ADR-0005 §1);
   it must not create its own `tokio::Runtime` if the consumer already has one.
3. **Cancellation protocol**: ctrl-C should stop a turn but not kill the session;
   SIGTERM should drain all sessions. Neither primitive maps directly to
   `task.abort()`.
4. **Event fanout**: persisting every event to disk (ADR-0002) and streaming
   every chunk to a UI subscriber are two different contracts with different
   latency/reliability tradeoffs. A single channel cannot satisfy both.
5. **Async job wake-up**: when a tool submits an async job and the Turn Driver
   pauses, something must resume the session when the job completes — without
   blocking the actor's mailbox loop.
6. **Sync vs async tool routing**: H08 must know, before calling `invoke()`,
   whether to expect an immediate `ToolResult` or a `JobId`. Leaving that
   decision to the LLM or to a post-hoc runtime check creates silent SLO
   regressions.

This ADR ratifies the six decisions that resolve these gaps. The detailed
rationale, Codex comparison tables, channel capacity derivations, pseudocode,
and sequence diagrams live in the design spec:
`docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`.

## Decision

### 1. Per-session actor task model

Each session runs in a dedicated long-lived tokio task (`SessionActor`). The
actor owns a mailbox (`mpsc<SessionCommand>`, capacity 64), a broadcast channel
(`broadcast<StreamEvent>`, capacity 256), a persist channel
(`mpsc<PersistCommand>`, capacity 256), and an in-flight turn handle. A
separate store-writer subtask drains the persist channel and performs all
`ConversationStore` I/O. When a turn starts, the actor spawns a short-lived
`TurnDriver` task; when the turn reaches a terminal state the task ends and is
joined.

**Rejected**: Codex-style `Arc<Session> + Mutex<ActiveTurn>` shared-state
concurrency. A poisoned mutex in one session propagates to all callers of that
session; a killed lock-holder blocks unrelated code paths. The actor model gives
per-session failure isolation at the tokio scheduler level, which is the
prerequisite for the ≥1000-concurrent-session SLO target (ADR-0005 §3).

See spec §3 for the full task topology diagram and invariant list.

### 2. Caller-injected tokio `Handle`

`RuntimeBuilder::handle(h: tokio::runtime::Handle)` accepts an externally
supplied handle. If the caller omits it, `build()` calls `Handle::current()` as
fallback, matching standard tokio library convention. All blocking I/O
(`spawn_blocking` for JSONL fsync) uses the injected handle, never
`tokio::fs`. A dedicated `current_thread` runtime is accepted at build time
for test compatibility; production use of `multi_thread` is documented
convention, not a compiler-enforced constraint.

**Rejected**: cogito-owned `Runtime::new()`. An embedded library that creates
its own runtime conflicts with the consumer's scheduler tuning, doubles thread
counts, and provides no extra panic isolation (isolation is task-level, not
runtime-level). Explicitly accepting a `Handle` is a one-line change to switch
between shared and dedicated runtimes without an API break (see spec §3
"tokio Handle 注入").

### 3. Two-level cancellation, cooperative

**Turn cancel** (`SessionHandle::cancel_turn()`): fires a per-turn
`CancellationToken` directly (no mailbox enqueue, so it bypasses FIFO and
takes effect immediately). For the `PausedOnJob` state where no `TurnDriver`
task is running, `cancel_turn()` additionally sends `SessionCommand::InternalCancel`
through the mailbox, which causes the actor to call `job_manager.cancel(job_id)`.
A new `CancellationToken` is created for each turn; the previous token is dropped
when the turn ends.

**Session shutdown** (`SessionHandle::shutdown(timeout: Duration)`): sends
`SessionCommand::Shutdown` through the mailbox. The actor drains in-flight work
cooperatively, falls back to token cancellation after the deadline, then joins
the store-writer subtask to flush all pending events before returning.

A process-level `shutdown_token: CancellationToken` is reserved in
`RuntimeBuilder` for v0.4 SaaS-ready (`runtime.shutdown_all()`); it is wired
but not exercised in v0.1.

**Rejected**: `task.abort()` as the primary cancellation primitive. Aborting a
future at an arbitrary `await` point can leave RAII guards (file locks, mutex
guards) in inconsistent state. Cooperative cancellation via `select!` on a
token gives every `await` site the chance to clean up before yielding (spec §4).

### 4. Dual event streams — persist channel and broadcast channel

Two independent outbound channels carry different views of the same events:

| Channel | Type | Capacity | Consumers | Batching |
|---|---|---|---|---|
| `persist_tx` | `mpsc<PersistCommand>` | 256 | Store-writer subtask (serial, one per session) | Text deltas batched ≤200 ms or ≤500 chars, then flush; all other events immediate |
| `events_out` | `broadcast<StreamEvent>` | 256 | Zero or more UI/API subscribers | Per-chunk, no batching; slow subscribers receive `Lagged(n)` and handle degradation themselves |

The `TurnDriver` writes to both channels for the same logical event when
appropriate (e.g., a `TextDelta` goes to `persist_tx` for batched durability
and to `events_out` per-chunk for low-latency streaming).

**Rejected**: a single unified channel shared by persistence and broadcast.
Persistence needs backpressure and durability guarantees; broadcast needs
low-latency fan-out with lossiness tolerance. Any single channel design
forces one contract to compromise. The capacity numbers (256 for both) match
Codex's `rollout/recorder.rs:244` for the persist side; Sprint 1 benchmarks
lock the final values (spec §7, ADR-0005 §3).

### 5. Mailbox-injected `JobCompleted` for async-job wake-up

When `ToolProvider::invoke()` returns `InvokeOutcome::Async(job_id)`, the
`TurnDriver` task persists a `TurnPaused` event and terminates. The actor
transitions to `in_flight = PausedOnJob { job_id }` and calls
`job_manager.on_complete(job_id, sink)` to register a completion callback. The
`sink` is an `mpsc::Sender<JobCompletionEvent>` shared across all pending jobs
for the session (capacity 32). When the job finishes, `JobManager` sends one
`JobCompletionEvent` on `sink`. The actor converts it to
`SessionCommand::JobCompleted` and routes it through the mailbox — preserving
FIFO ordering against any new `Input` commands that may have arrived — before
spawning a new `TurnDriver` task that resumes the FSM from `ToolDispatching`
with the completed result.

The `JobManager` trait shape that enables this:

```rust
pub trait JobManager: Send + Sync {
    async fn status(&self, job_id: JobId) -> Result<JobStatus>;
    async fn result(&self, job_id: JobId) -> Result<JobOutcome>;
    async fn cancel(&self, job_id: JobId) -> Result<()>;
    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<()>;
}
```

This matches the "fires between turns" pattern of Claude Code, and the
`waitForEvent` / Signal patterns of Inngest and Temporal respectively. The
`on_complete(job_id, sink)` shape is the in-process form of a distributed
broker callback; v0.4 swaps the implementation without changing Brain code
(spec §6 "SaaS scalability of this design").

**Rejected**: actor blocking on `job_manager.await_result(job_id)` while
`PausedOnJob`. Blocking the actor task means the mailbox is not polled,
which prevents `cancel_turn()`, new `Input` commands, and `Shutdown` from
being processed. The actor must always be available to read its mailbox.

### 6. `ExecutionClass` on `ToolDescriptor` — runtime-decided sync/async routing

`ToolDescriptor` gains an `execution_class: ExecutionClass` field:

```rust
pub enum ExecutionClass {
    /// invoke() must return InvokeOutcome::Sync.   e.g. read_file, now
    AlwaysSync,
    /// invoke() must return InvokeOutcome::Async.  e.g. run_tests, build_release
    AlwaysAsync,
    /// invoke() chooses per call based on arguments. e.g. transcribe_audio
    Adaptive,
}
```

H08 Tool Dispatcher reads `execution_class` before calling `invoke()` to
statically know which `InvokeOutcome` variant to expect. `Adaptive` tools
inspect their arguments inside `invoke()` and return either variant. Contract
violations (e.g., `AlwaysSync` returning `Async`) are `debug_assert!` in debug
builds and a `warn!` log in release builds, then handled by actual return value.

H05 Tool Surface Builder uses `HarnessStrategy::allow_async_tools: bool`
(default `true`) to filter `AlwaysAsync` and `Adaptive` tools from the prompt
when a role must not initiate long-running work.

**Rejected**: exposing `run_in_background: bool` as a tool-call argument visible
to the LLM. This pollutes the prompt schema with execution-model knowledge,
is not portable across providers (Anthropic and OpenAI tool specs differ), and
allows LLM misjudgments to cause silent SLO regressions (short task marked
background wastes a mailbox round-trip; long task marked sync blocks the model
stream for minutes). cogito is an embedded runtime supporting arbitrary callers
and models; the LLM must not need to understand the runtime's scheduling model
(spec §6 "为什么不选 (3)").

## Consequences

- **Easier**:
  - Layer violations are caught by the compiler: `cogito-core/Cargo.toml` lists
    only `cogito-protocol` as a non-dev dependency in the `harness/` module.
    Any attempt to `use cogito_tools::…` inside `harness/` is a build error.
  - Failure isolation is three-level: `catch_unwind` at the actor entry point
    stops a session panic from killing other sessions; `catch_unwind` around each
    turn stops a tool panic from killing the session; the process continues
    serving all other sessions regardless of what any one turn does. This
    concretely implements ADR-0005 §4 "Failure isolation."
  - The `JobManager::on_complete(job_id, sink)` trait shape is SaaS-ready by
    design: v0.4 can replace `cogito-jobs` (in-process `LocalJobManager`) with
    `cogito-jobs-distributed` (Redis Stream or Inngest/Temporal backend) without
    modifying any Brain code.
  - `ExecutionClass` lets H08 make routing decisions without calling `invoke()`,
    enabling pre-dispatch validation and strategy-level tool filtering.

- **Harder**:
  - The three-task-per-session topology (actor + store-writer subtask +
    TurnDriver task) requires careful shutdown sequencing. The drain-then-join
    protocol in `SessionCommand::Shutdown` handling is non-trivial to test.
  - Resuming after a crash while `PausedOnJob` requires H03 to query
    `job_manager.status(job_id)` on replay — a new entry in H03's decision
    table that was not part of ADR-0003.
  - `Adaptive` tools require runtime contract-violation detection rather than
    compile-time enforcement.

- **Given up**:
  - The simplicity of a single `async fn turn_driver(…)` that owns all I/O.
    We trade it for the resumability and failure-isolation properties the
    actor + FSM design provides.
  - Direct `task.abort()` as a fast-kill escape hatch. Cooperative cancellation
    means a misbehaving tool can delay shutdown up to the deadline before the
    actor falls back to abort.

## Follow-on work

- **Sprint 1** (Plan 2, Task group B): implement the `store_writer` subtask
  body and `cogito-store-jsonl` per-event fsync. Lock the P99 step-record write
  latency number per ADR-0005 §3.
- **Sprint 2**: implement the full `TurnDriver` FSM body, model gateway
  integration (H04/H06 model stream demux), and synchronous tool dispatch
  (H08 with `AlwaysSync` tools only).
- **Sprint 3**: implement H03 Resume Coordinator, including the new
  `job_manager.status(job_id)` query on the `PausedOnJob` replay path.
- **Sprint 4**: implement real `JobManager` in `cogito-jobs` (`LocalJobManager`
  with `spawn_blocking` job tasks) and the first `AlwaysAsync` tool.
- **v0.4**: `cogito-jobs-distributed` backend; `JobManager` trait shape is
  unchanged from this ADR. Distributed locking via store fsync + monotonic event
  IDs; `SessionForked` event for split-brain detection.

## References

- **Spec**: `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
  — detailed rationale, Codex comparison tables, channel capacity derivations,
  pseudocode, sequence diagrams for all decisions above
- **ADR-0001** (Rust workspace layout) — establishes the crate list; this ADR
  confirms `cogito-core/harness/` is Protocol-only and `cogito-jobs` is the
  v0.1 `JobManager` implementation crate
- **ADR-0002** (event-sourced conversation log) — establishes that state lives
  in the store; Decision 5 above ("resume_state is rebuilt from store, not held
  in actor memory") is the concrete enforcement of that rule
- **ADR-0003** (state-machine Turn Driver) — establishes the FSM states and
  "write event before transition" invariant; this ADR adds the `enum TurnState`
  Rust representation and the multi-task execution context
- **ADR-0004** (Brain / Hands / Session crate boundaries) — establishes the
  import rules; Decision 1 actor topology and Decision 6 `ExecutionClass` both
  preserve "Brain only sees Protocol traits"
- **ADR-0005** (production scope and quality gates) — establishes the ≥1000
  concurrent session SLO target (motivates Decision 1), the failure isolation
  gate (concretized by the three-level `catch_unwind` in Consequence), and the
  SaaS-ready pluggability requirement (met by Decision 5 `on_complete` shape)
- **ARCHITECTURE.md** §"Workspace layout" — crate-to-layer table updated to
  reflect `cogito-core/harness/` Protocol-only import constraint
- **ARCHITECTURE.md** §"Trait contracts in cogito-protocol" — `StreamEvent`,
  `ExecutionClass`, `InvokeOutcome`, `TurnOutcome`, `TurnFailureReason`, and
  the updated `JobManager::on_complete` shape are all defined in
  `cogito-protocol` per this ADR

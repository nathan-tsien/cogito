# Sprint 8 · Async Jobs (`cogito-jobs`) — design spec

**Status**: Proposed
**Sprint**: v0.1 / Sprint 8
**Budget**: 2 days
**Predecessors**: Sprint 3 (Resume Coordinator decision table + `MockJobManager` + `InFlight::PausedOnJob` scaffolding), Sprint 2 (H08 sync path, `JobManager` trait freeze in `cogito-protocol`)
**Successors**: v0.3 Subagent S1 `wait_agent` (depends on `JobManager`), v0.4 multi-replica resume (depends on cross-process job model — out of scope here, see §2)

---

## 1. Goals

Ship the concrete `JobManager` implementation and wire the async dispatch
path end-to-end, so that:

1. A model can call an `ExecutionClass::AlwaysAsync` tool, the turn pauses,
   the job runs as a background `tokio::task`, and on completion the turn
   resumes and reaches terminal state.
2. The conversation event log captures every load-bearing fact: who was
   submitted (`JobSubmitted`), when the turn paused (`TurnPaused`), how the
   job terminated (`JobCompletedRecorded`), and what `ToolResult` flowed
   back to Brain (`ToolResultRecorded`).
3. If the Runtime process crashes while a turn is paused, the next
   `Runtime::open_session(Resume)` synthesizes a structured failure for
   the lost job so Brain never gets stuck — no `JobManagerUnavailable`
   shutdown path.
4. The user can cancel a paused turn (the in-flight job is killed) and can
   queue exactly one user message while a turn is in flight (drained when
   the current turn finishes).
5. One real, externally-useful async tool ships: `run_tests`.

Sprint 8 is the last v0.1 feature sprint that touches the FSM. After this
the Brain-side surface is frozen until v0.2 Plugin work.

## 2. Non-goals

- **Cross-process job survival.** A Runtime crash terminates every running
  job. Resume only re-establishes Brain-side state from the event log;
  it does not re-execute or reconnect to the killed tokio task. Sprint 8
  scope is "process-bounded jobs with a durable conversation log";
  true cross-process jobs are a v0.4 SaaS-ready concern.
- **Multiple concurrent async dispatches per turn.** v0.1 enforces ≤1
  async dispatch per turn at H08. The protocol already supports the
  multi case (each `JobSubmitted` carries `call_id`); enforcement
  loosens in a later sprint.
- **Parallel sync dispatch.** Inherited from H08's existing v0.1 scope
  (sequential only).
- **Per-job resource budgets.** A 10-minute hard deadline is the only
  guard. Memory caps, CPU caps, and rate limits land in v0.4.
- **Streaming tool stdout.** `run_tests` buffers stdout/stderr and returns
  one `ToolResult::Output` at terminal time. Streaming would need a new
  `StreamEvent::ToolOutputDelta` variant; out of scope.
- **Sandboxing.** `run_tests` spawns a plain `tokio::process::Command`.
  `cogito-sandbox` redesign is a v0.4 ROADMAP item.
- **Separate `jobs.jsonl` per-session log.** Once `JobSubmitted` lives in
  the conversation event log, a sidecar log adds no information for the
  Sprint 8 scope. The ROADMAP bullet that called for one is being
  revised by this sprint (see §11).

## 3. Locked decisions (this spec)

| Topic | Decision | Source |
|---|---|---|
| Process survival | Process-bounded jobs; conversation log as sole persistence | Q1 brainstorm |
| Async tool shipped | `run_tests` (real) + `SleepTool` (test fixture) | Q2 |
| Event variant | Add `EventPayload::JobSubmitted { call_id, job_id, tool_name }` (additive, no `SCHEMA_VERSION` bump) | Q3 |
| Mid-pause queue | Single-slot, latest-wins, warn on overwrite | Q4 |
| Cancel-while-paused | `cancel_turn` calls `JobManager::cancel(current_job_id)` | Q4 |
| JobManager topology | Runtime-singleton `LocalJobManager`, injected into both `ToolProvider` and each session | Q5 |
| Sidecar `jobs.jsonl` | Dropped from sprint scope | Q5 |
| Cancel-token-disconnect (existing `TODO`) | Fix in this sprint (`Arc<parking_lot::Mutex<CancellationToken>>` shared between `SessionState` and `SessionShared`) | Section 1 |
| Resume entry shape | Reuse `TurnEntry::FromToolDispatching` for both `ResumeAfterJobCompletion` and live job-completion (no new `TurnEntry` variant) | Section 2 |
| Lost-job synthesis path | Post the synthetic `JobCompletionEvent` on the session's own `job_completion_rx` so it flows through the same FIFO Arm 3 as live completions | Section 2 |
| `run_tests` cwd | `std::env::current_dir()`; no `ExecCtx.cwd` field added | Q6 |
| `run_tests` output | Buffer + truncate (32 KiB head + 32 KiB tail) | Q6 |
| Default deadline if `ExecCtx.deadline = None` | 10 minutes hard cap | Q6 |

## 4. Architectural overview

```
┌─ RuntimeBuilder ───────────────────────────────────────────────────────┐
│   1. Construct Arc<LocalJobManager> singleton                          │
│   2. Pass clone into BuiltinToolProvider so RunTestsTool can submit    │
│   3. Pass clone (as Arc<dyn JobManager>) into every SessionState       │
└────────────────────────┬───────────────────────────────────────────────┘
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
┌─ SessionState ──────────┐   ┌─ BuiltinToolProvider ─────────────────┐
│  job_completion_rx      │   │  RunTestsTool {                       │
│  job_completion_tx      │   │    job_mgr: Arc<LocalJobManager>      │
│  pending_user_input:    │   │  }                                    │
│    Option<TurnTrigger>  │   │   invoke() returns InvokeOutcome::    │
│  in_flight: Option<     │   │     Async(JobId) after job_mgr.submit │
│    Active | PausedOnJob │   └───────────────────────────────────────┘
│      { turn_id, job_id, │
│        call_id }>       │
└─────────────────────────┘
```

Brain/Hands boundary stays intact: `harness/` continues to `use
cogito_protocol::*` only. `LocalJobManager` lives in `cogito-jobs` (Hands)
and is injected as `Arc<dyn JobManager>` via Runtime wiring.

## 5. Protocol changes (`cogito-protocol`)

### 5.1 New event variant

```rust
EventPayload::JobSubmitted {
    /// Identifier matching the originating ToolUseRecorded.call_id.
    call_id: String,
    /// Opaque job identifier produced by JobManager.
    job_id: JobId,
    /// Tool name, redundant with the call_id lookup but kept for
    /// log-readability and debugging.
    tool_name: String,
}
```

- Additive variant under ADR-0007; no `SCHEMA_VERSION` bump.
- Recorded by H08 immediately after `tool.invoke()` returns
  `InvokeOutcome::Async(job_id)`, **before** the `on_complete` sink is
  registered. Ordering guarantee: `ToolUseRecorded(call_id) <
  JobSubmitted(call_id) < TurnPaused(job_id) < JobCompletedRecorded(job_id) <
  ToolResultRecorded(call_id)`.
- JSON Schema artifact at `docs/schemas/conversation-event-v1.json`
  regenerated by `cargo run -p cogito-protocol --bin gen-schema` (or the
  existing CI gate auto-detects drift).
- Canonical fixture `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
  gains one `JobSubmitted` line so the contract test matrix exercises
  the new variant.

### 5.2 No trait changes

`JobManager` trait shape is unchanged from its Sprint 0 freeze. The
existing `on_complete(JobId, mpsc::Sender<JobCompletionEvent>) ->
Result<(), JobError>` shape is what enables the singleton-JobManager +
per-session-sink topology.

### 5.3 No `ExecCtx` changes

No `cwd` field added. `ExecCtx.deadline` and `ExecCtx.cancel` are the
only signals async tools observe.

## 6. `cogito-jobs` crate

### 6.1 `LocalJobManager`

```rust
pub struct LocalJobManager {
    jobs: Arc<Mutex<HashMap<JobId, JobLifecycle>>>,
}

struct JobLifecycle {
    status: JobStatus,
    outcome: Option<JobOutcome>,
    on_complete_sink: Option<mpsc::Sender<JobCompletionEvent>>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl LocalJobManager {
    pub fn new() -> Arc<Self> { ... }

    /// Submission API (not part of the JobManager trait per protocol
    /// design note). Returns a JobId; spawns the future as a tokio task
    /// that, on completion, writes the outcome into the lifecycle map
    /// and fires the registered sink (if any).
    pub fn submit<F>(&self, fut: F) -> JobId
    where
        F: Future<Output = JobOutcome> + Send + 'static;
}
```

`LocalJobManager` implements the same `on_complete` semantics as
`MockJobManager` (contract 1: fire immediately if already terminal;
contract 2: store sink, fire later) — extracted to a shared contract
test (see §10).

The `cancel(job_id)` implementation invokes `AbortHandle::abort()` on
the task and, when the task observes the abort and yields, the
submission wrapper records `JobOutcome::Cancelled` and fires the sink.
For tasks that don't yield (CPU-bound), abort doesn't take effect until
the next `.await`; for `run_tests` this is fine because the wait on
the child process is an `.await` point.

### 6.2 `RunTestsTool`

```rust
pub struct RunTestsTool {
    job_mgr: Arc<LocalJobManager>,
}

#[derive(Deserialize)]
struct RunTestsArgs {
    /// Optional package filter (`-p <pkg>` to nextest).
    package: Option<String>,
    /// Optional test name filter (positional arg to nextest).
    filter: Option<String>,
}
```

- `ToolDescriptor.execution_class = ExecutionClass::AlwaysAsync`.
- `invoke()` parses args, then `job_mgr.submit(run_cargo_nextest(args, ctx))` and returns `InvokeOutcome::Async(job_id)`.
- `run_cargo_nextest` runs `cargo nextest run [package args] [filter]` via `tokio::process::Command`:
  - `cwd = std::env::current_dir()`
  - stdout/stderr captured with `Stdio::piped()`
  - wrapped in `tokio::select!` against `ctx.cancel.cancelled()` and `tokio::time::sleep_until(deadline_or_10min)`
  - on either cancel or deadline: `child.kill().await`; outcome is `Cancelled` or `Failed { message: "timeout" }`
  - on natural exit: assemble `ToolResult::Output { value: json!({ "stdout": truncate(stdout), "stderr": truncate(stderr), "exit_code": status.code() }) }` wrapped in `JobOutcome::Success`
- Truncation: `truncate(bytes)` returns the original string if ≤ 64 KiB; otherwise `<first 32 KiB> + format!("\n... [{N} bytes elided] ...\n") + <last 32 KiB>`.

### 6.3 `SleepTool` (test fixture)

Lives behind `#[cfg(any(test, feature = "test-tools"))]` in `cogito-jobs`
(or in `cogito-test-fixtures` — final placement during implementation).

```rust
pub struct SleepTool { job_mgr: Arc<LocalJobManager> }

#[derive(Deserialize)]
struct SleepArgs { duration_ms: u64 }
```

- `ExecutionClass::AlwaysAsync`.
- `invoke()` submits a job whose future is `tokio::time::sleep(Duration::from_millis(duration_ms)).await; JobOutcome::Success { result: ToolResult::text("slept") }`.

Used by integration tests and the chaos test so they don't depend on a
real `cargo nextest` invocation.

## 7. H08 Tool Dispatcher async path (`cogito-core::harness::dispatcher`)

Current code in `dispatcher.rs` returns `async_not_supported(name)` for
both `AlwaysAsync` tools and `InvokeOutcome::Async`. Replace with the
real async path:

```rust
// Pseudocode — actual signature plumbs job_mgr + step recorder.
match outcome {
    InvokeOutcome::Sync(result) => DispatchOutcome::SyncResult(result),
    InvokeOutcome::Async(job_id) => {
        // 1. Record JobSubmitted event (write-before-transition).
        recorder.record_job_submitted(turn_id, call_id, job_id, name).await?;
        // 2. Register session sink for completion.
        job_mgr.on_complete(job_id, session_job_tx.clone()).await?;
        // 3. Hand back to TurnDriver which records TurnPaused and pauses.
        DispatchOutcome::AsyncJob(job_id)
    }
    _ => DispatchOutcome::SyncResult(async_not_supported(&name)),
}
```

Threading note: `dispatch()` signature gains `job_mgr: &dyn JobManager`,
`session_job_tx: &mpsc::Sender<JobCompletionEvent>`, `recorder: &Mutex<StepRecorder>`,
and `call_id: &str`. These come from `TurnDeps`, plumbed through
`transitions::tool_dispatching`.

Invariant enforcement: H08 enforces "≤1 async dispatch per turn" by
asserting `state.in_flight != PausedOnJob` before dispatching — the
sequential nature of v0.1 dispatch already prevents two async calls in
the same `ToolDispatching` pass; the assertion catches future
refactoring regressions.

## 8. `transitions::tool_dispatching` and session loop

### 8.1 Transition logic

```
on DispatchOutcome::AsyncJob(job_id):
    recorder.record_turn_paused(turn_id, job_id)
    return TurnOutcome::Paused { job_id }
```

### 8.2 `on_turn_complete` for `Paused`

Already handled correctly today (`record_turn_paused`) — but
`on_turn_complete` currently sets `state.in_flight = None`. Change:
when outcome is `TurnOutcome::Paused { job_id }`, set `state.in_flight =
PausedOnJob { turn_id, job_id, call_id }` instead.

`call_id` is not on `TurnOutcome::Paused` and we are *not* widening the
protocol to add it (keeps `TurnOutcome` minimal). Instead,
`on_turn_complete` calls `lookup_call_id(&state.recorder, job_id)` —
defined in §9.4 — which scans the just-written log for the matching
`JobSubmitted`. This is one extra log read per pause; negligible.

`InFlight::PausedOnJob` shape today is `{ turn_id, job_id }` (both
`#[allow(dead_code)]`). Sprint 8 widens it to `{ turn_id, job_id,
call_id }` and removes the `dead_code` allows.

### 8.3 `SessionCommand::JobCompleted` handling

Currently a stub. Implement:

```
handle_command(JobCompleted { job_id, outcome }):
    let Some(InFlight::PausedOnJob { turn_id, job_id: expected, call_id }) = state.in_flight else {
        tracing::error!("JobCompleted for non-paused session; dropping");
        return None;
    };
    if expected != job_id {
        tracing::error!("JobCompleted job_id mismatch; dropping");
        return None;
    }
    recorder.record_job_completed(turn_id, job_id, outcome.clone()).await;
    let tool_result = outcome_to_tool_result(outcome);
    spawn_turn_driver(state, turn_id, TurnEntry::FromToolDispatching {
        pending: vec![],
        completed: vec![(call_id, tool_result)],
    }, deps);
```

`outcome_to_tool_result` maps per §3 of the brainstorming — `Success` →
unwrap inner `ToolResult`; `Failed { message }` → `ToolResult::Error {
kind: AsyncFailed, message, retryable: false }`; `Cancelled` →
`ToolResult::Error { kind: Cancelled, message: "job cancelled",
retryable: false }`.

### 8.4 Mid-pause user input (single-slot queue)

Add `pending_user_input: Option<TurnTrigger>` to `SessionState`.

```
handle_command(Trigger(t)):
    if state.has_active_turn() || state.is_paused() {
        if state.pending_user_input.is_some() {
            tracing::warn!("overwriting queued user input");
        }
        state.pending_user_input = Some(t);
    } else {
        try_start_turn(state, t, deps);
    }

on_turn_complete(...):
    ... record terminal event ...
    if let Some(pending) = state.pending_user_input.take() {
        try_start_turn(state, pending, deps);
    }
```

The queued trigger is **not** written to the event log when it arrives;
it only becomes a `TurnStarted` event when it transitions into an
actual turn. Rationale: a queued-and-overwritten trigger never became a
turn from Brain's perspective, so there's nothing to record.

### 8.5 `cancel_turn` mid-pause

`SessionHandle::cancel_turn`:

```
cancel_turn():
    state.current_cancel_token.lock().cancel();  // existing behavior
    if let InFlight::PausedOnJob { job_id, .. } = state.in_flight {
        // Send a new SessionCommand::CancelJob { job_id } that the loop
        // forwards to JobManager.cancel(job_id) on the actor task
        // (JobManager.cancel is async; SessionHandle is sync-callable).
    }
```

Detail: introduce `SessionCommand::CancelJob { job_id }` so the
JobManager call happens on the session-loop task, not on the caller's
thread. The actor calls `job_mgr.cancel(job_id).await`. The subsequent
completion flows through Arm 3 as a normal `JobCompleted { outcome:
Cancelled }`.

### 8.6 Cancel-token-disconnect fix

Today: `SessionShared.current_cancel_token` is a clone of the token at
session-open time; `spawn_turn_driver` mints a fresh token per turn and
writes it into `SessionState.current_cancel_token`, but
`SessionHandle::cancel_turn` fires the original sibling — so cancel of
any turn past the first is silently a no-op.

Fix:

- Change `SessionState.current_cancel_token` and
  `SessionShared.current_cancel_token` to both hold
  `Arc<parking_lot::Mutex<CancellationToken>>` pointing at the same
  inner mutex.
- `spawn_turn_driver` replaces the inner `CancellationToken` via the
  shared mutex.
- `SessionHandle::cancel_turn` reads through the same Arc.

Regression test: two consecutive turns, `cancel_turn` mid-second-turn,
verify the second turn observes cancel (see §10).

## 9. H03 Resume Coordinator updates (`cogito-core::harness::resume`)

### 9.1 `ResumeAfterJobCompletion.call_id` derivation

Sprint 3 derived `call_id` by scanning backward from `TurnPaused` for
the latest unmatched `ToolUseRecorded`. With `JobSubmitted` now in the
log, the derivation becomes:

```
find latest JobSubmitted { call_id, job_id }
where job_id matches TurnPaused.job_id
return call_id
```

Strip the "find latest unmatched `ToolUseRecorded`" narrowing from
Sprint 3 spec §4.3 and from `resume.rs`. Update doc comments.

### 9.2 `ResumePoint::ResumePausedJob` payload

No shape change. Already `{ turn_id: TurnId, job_id: JobId }`. The
`call_id` is fetched from the new `JobSubmitted` event when
`apply_resume_point` needs to record the synthesized failure — but for
`ResumePausedJob` specifically, the failure flows through Arm 3 as a
normal completion, so `call_id` is re-derived in
`handle_command(JobCompleted)` via the same lookup.

### 9.3 `apply_resume_point` activations

Replace today's `ShutdownOutcome::JobManagerUnavailable` returns:

```rust
ResumePoint::ResumePausedJob { turn_id, job_id } => {
    let call_id = lookup_call_id_in_events(&initial_events, job_id)
        .ok_or_else(|| ShutdownOutcome::ResumeFailed(
            format!("no JobSubmitted for job {job_id}")
        ))?;
    state.in_flight = Some(InFlight::PausedOnJob { turn_id, job_id, call_id });
    match job_mgr.on_complete(job_id, state.job_completion_tx.clone()).await {
        Ok(()) => {}  // sink registered; will fire when job terminates
        Err(JobError::UnknownJob(_)) => {
            // Lost across process restart. Synthesize a Failed completion.
            let _ = state.job_completion_tx.send(JobCompletionEvent {
                job_id,
                outcome: JobOutcome::Failed {
                    message: "lost across process restart".into(),
                },
            }).await;
        }
        Err(e) => return Err(ShutdownOutcome::ResumeFailed(e.to_string())),
    }
    Ok(())
}

ResumePoint::ResumeAfterJobCompletion { turn_id, call_id, outcome, .. } => {
    let tool_result = outcome_to_tool_result(outcome);
    spawn_turn_driver(state, turn_id, TurnEntry::FromToolDispatching {
        pending: vec![],
        completed: vec![(call_id, tool_result)],
    }, deps);
    Ok(())
}
```

`ShutdownOutcome::JobManagerUnavailable` retains its variant slot (for
future backends that genuinely can't get a JobManager at startup) but
v0.1 stops returning it.

### 9.4 `lookup_call_id` helper

Pure function used by §8.2 (live pause) and §9.3 (resume):

```rust
fn lookup_call_id_in_events(events: &[ConversationEvent], job_id: JobId) -> Option<String> {
    events.iter().rev().find_map(|e| match &e.payload {
        EventPayload::JobSubmitted { call_id, job_id: jid, .. } if *jid == job_id => {
            Some(call_id.clone())
        }
        _ => None,
    })
}
```

Live-path counterpart reads from the `StepRecorder`'s in-memory tail
(the recorder retains the last-N events for projection use; if it
doesn't, fall back to a small `Option<(JobId, String)>` cache populated
by `record_job_submitted`). Implementation chooses the path that fits
the recorder's current internals — both are correct.

Failure to find a match is structural: it means we recorded a
`TurnPaused` without a preceding `JobSubmitted`, which Sprint 8's
H08 contract forbids. Resume returns `ShutdownOutcome::ResumeFailed`;
live path is a `tracing::error!` + leave `in_flight = None` (turn
ends without recording an extra event — the existing terminal record
in `on_turn_complete` already wrote `TurnPaused`, which a later
`open_session(Resume)` will treat as `ResumePausedJob` and recover from).

## 10. Testing strategy

### 10.1 Unit tests

- `cogito-jobs::LocalJobManager`:
  - `submit` registers `Running`, returns unique `JobId`
  - `on_complete` contract 1 (already-terminal → fire immediately)
  - `on_complete` contract 2 (still-running → store sink, fire on completion)
  - `cancel` aborts the task and fires `Cancelled`
  - `status` / `result` lifecycle queries
  - Sink dropped before completion → silently swallow `send` error
- `cogito-core::harness::dispatcher`:
  - `AlwaysAsync` tool → records `JobSubmitted`, returns `AsyncJob` (with mock JobManager)
  - Descriptor `AlwaysSync` + `InvokeOutcome::Async` → `ToolResult::Error { InvocationFailed }`
- `cogito-core::harness::resume`:
  - Decision-table row for `JobSubmitted + TurnPaused + no JobCompletedRecorded` → `ResumePausedJob { call_id from JobSubmitted }`
  - Decision-table row for `JobSubmitted + TurnPaused + JobCompletedRecorded` → `ResumeAfterJobCompletion { call_id from JobSubmitted, outcome }`

### 10.2 Contract test

Promote the JobManager contract tests currently inline in
`MockJobManager` to a shared `contract_tests::job_manager` module under
`crates/cogito-protocol/tests/` (mirroring the `ConversationStore`
contract-test pattern). Both `MockJobManager` and `LocalJobManager`
import and run the suite.

### 10.3 Integration tests (`crates/cogito-jobs/tests/`)

- `sleep_then_complete`: full async loop with `SleepTool(100ms)`.
  Verifies event sequence: `TurnStarted → ToolUseRecorded → JobSubmitted
  → TurnPaused → JobCompletedRecorded → ToolResultRecorded → AssistantMessageAppended → TurnCompleted`.
- `cancel_while_paused`: pause on `SleepTool(60s)`, call `cancel_turn`,
  verify `JobManager.cancel` fires, `JobCompletionEvent { outcome:
  Cancelled }` flows through, turn terminates with
  `ToolResult::Error { kind: Cancelled }`.
- `mid_pause_user_input`: pause, `send_user("a")`, complete the
  in-flight job, verify first turn ends and second turn starts with
  `"a"`. Then a variant: pause, `send_user("a")`, `send_user("b")` —
  verify warn log + `"b"` wins.
- `run_tests_happy_path`: spawn a fixture crate under
  `crates/cogito-jobs/tests/fixtures/echo_crate/` with one trivial
  test; have the session call `run_tests`; verify the truncated output
  JSON makes it back. No `#[ignore]` — relies on `cargo nextest` per
  CLAUDE.md prereq.

### 10.4 Chaos tests (`crates/cogito-core/tests/resume_chaos.rs`)

Activate the `paused_async_job` scenario Sprint 3 left dormant:

- Trajectory: `user → model emits sleep_tool → JobSubmitted → TurnPaused → [CRASH] → restart → ResumePausedJob → synthetic JobCompletedRecorded (Failed) → ToolResultRecorded (AsyncFailed) → model end_turn → TurnCompleted`.
- Uses `MockJobManager` (deterministic; `LocalJobManager` is covered by
  the integration tests).
- Crash boundaries to exercise:
  - between `JobSubmitted` and `TurnPaused`
  - between `TurnPaused` and `JobCompletedRecorded` (where the lost-job synthesis kicks in)
  - between `JobCompletedRecorded` and the next `ToolResultRecorded` (exercises `ResumeAfterJobCompletion`)
- All 4 oracles (prefix immutable, terminal equivalent, tool mapping
  equivalent, final text equivalent) pass.

### 10.5 Cancel-token-disconnect regression test

Unit test in `crates/cogito-core/src/runtime/session_loop.rs` `#[cfg(test)]`:

- Open session.
- Run turn 1 to completion (`UserText("hello")` against a mock gateway that returns end_turn immediately).
- Send `UserText("loop forever")` against a mock gateway whose
  `stream()` future awaits `ctx.cancel.cancelled()` (i.e., it pauses
  indefinitely until cancelled).
- Spawn a tokio task that, after 50ms, calls `SessionHandle::cancel_turn`.
- Assert: turn 2 terminates with `TurnFailed { reason: TurnTimedOut }`
  or `Cancelled`, within 500ms.

Without the fix, the cancel token fired by `SessionHandle` is a stale
sibling and the test hangs.

### 10.6 Schema-drift gate

The existing CI gate
(`docs/schemas/conversation-event-v1.json` drift check) auto-detects
the new `JobSubmitted` variant. Run `make ci` locally to refresh.

## 11. Out-of-band changes during this sprint

- **`crates/cogito-jobs/src/lib.rs` doc comment.** Today says "state
  persisted to SQLite for resume after crashes". Rewrite to:
  > Local async job manager. Jobs run as `tokio::task`s inside the
  > Runtime process; their lifecycle is mirrored into the conversation
  > event log (`JobSubmitted` / `JobCompletedRecorded`). A Runtime
  > restart loses every running job; the resume coordinator synthesizes
  > `JobOutcome::Failed { message: "lost across process restart" }` for
  > any open job at crash time so Brain unwinds cleanly. True
  > cross-process job survival is a v0.4 SaaS-ready concern.
- **`ROADMAP.md` Sprint 8 bullets.** Edit:
  - Drop "JSONL job log persistence" wording.
  - Change "cross-process job state persistence (mirrors event log
    structure)" to "process-bounded jobs; conversation log is sole
    persistence; lost-job synthesis on Runtime restart". Reference this
    spec.
- **`docs/components/H08-tool-dispatcher.md`.** Promote `Status` from
  "In progress · Sprint 2" to "v0.1 complete · Sprint 8" and replace
  the "returns stubbed error until Sprint 5" comments with the actual
  behavior. (Sprint 5 was renumbered to Sprint 8 in the 2026-05-22
  rebalance; the comment never got refreshed.)
- **`docs/components/H03-resume-coordinator.md`.** Note that
  `ResumePausedJob` and `ResumeAfterJobCompletion` are no longer
  "Sprint 4 deliverable" — they are wired in Sprint 8.
- **`docs/data-model/jsonl-v1.md`.** Additive entry for `JobSubmitted`.

## 12. Out of scope (this sprint) — explicitly tracked

- **Multi-async-dispatch per turn.** Enforcement loosens in a later
  sprint; `JobSubmitted.call_id` is already shaped for it.
- **`run_bash` general shell tool.** Considered and deferred — `run_bash`
  needs sandbox boundary thinking that belongs to v0.4.
- **Streaming tool output.** Would need
  `StreamEvent::ToolOutputDelta`; deferred.
- **`RestartCurrentTurn` full implementation** (still downgrades to
  `FreshTurn` per Sprint 3 narrowing). Recovering `user_input` from the
  event log is independent of async jobs; tracked for Sprint 10 hardening.
- **`ExecCtx.cwd` field.** Tracked for the sprint that ships a tool
  that genuinely needs per-call cwd (likely `run_bash`).

## 13. Acceptance checklist

Sprint 8 closes when:

- [ ] `EventPayload::JobSubmitted` lands in `cogito-protocol` (additive); schema artifact regenerated; canonical fixture updated.
- [ ] `LocalJobManager` implements `JobManager` and `submit`; passes shared `contract_tests::job_manager`.
- [ ] `RunTestsTool` ships in `cogito-jobs`, registered through `BuiltinToolProvider`; `cogito chat` can invoke it and receives a truncated `ToolResult::Output`.
- [ ] H08 async path records `JobSubmitted` → registers `on_complete` → returns `AsyncJob`; sync path unchanged.
- [ ] `transitions::tool_dispatching` records `TurnPaused` and returns `TurnOutcome::Paused`.
- [ ] `session_loop::handle_command(JobCompleted)` records `JobCompletedRecorded` and resumes via `TurnEntry::FromToolDispatching`.
- [ ] `session_loop` honors single-slot mid-pause user input with warn-on-overwrite.
- [ ] `SessionHandle::cancel_turn` mid-pause routes through `SessionCommand::CancelJob` → `JobManager::cancel`.
- [ ] Cancel-token-disconnect fixed; regression test passes.
- [ ] `apply_resume_point` activates `ResumePausedJob` (synthesizes lost-job failure) and `ResumeAfterJobCompletion`; no `JobManagerUnavailable` return remains.
- [ ] `resume.rs` derives `call_id` from `JobSubmitted` (Sprint 3 narrowing removed).
- [ ] `paused_async_job` chaos scenario activated; all 4 oracles green at all three boundaries.
- [ ] `make ci` green (fmt + clippy + layer-check + test + schema-drift).
- [ ] `docs/components/H03-resume-coordinator.md`, `H08-tool-dispatcher.md`, `docs/data-model/jsonl-v1.md`, ROADMAP, and `cogito-jobs/src/lib.rs` doc comment all updated per §11.
- [ ] CHANGELOG entry under v0.1.

## 14. Risks

- **Subprocess kill semantics on cancel.** `child.kill().await` sends
  `SIGKILL` on Unix. Tests on macOS / Linux. Windows behavior
  documented but not in v0.1 CI matrix.
- **Cancel-token-disconnect fix may surface latent test assumptions.**
  Any test that implicitly relied on the broken behavior (i.e., assumed
  `cancel_turn` past the first turn was a no-op) will break. Likely
  zero such tests today (the bug is documented as untested), but flag
  during code review.
- **Truncation algorithm correctness for non-UTF-8 byte streams.**
  `cargo nextest` output is UTF-8 in practice; we use `String`. If a
  test prints raw bytes via stdout, the `String::from_utf8_lossy` path
  applies (one `replacement_char` per invalid sequence). Documented in
  `RunTestsTool` doc comment.

# ADR-0011: Subagent execution model (v0.2 S2 minimal)

**Status**: Accepted (v0.2 Sprint 11 minimal scope ratified 2026-05-30;
the v0.3 full surface is **amendment-only** — see §"v0.3 amendment, not
built now")
**Date**: 2026-05-30
**Spec**: `docs/superpowers/specs/2026-05-30-sprint-11-subagent-minimal-design.md`
**Relates to**: ADR-0004 (layer rule — why `BrainSpawner` is a Protocol
trait), ADR-0007 (event log as cross-language contract — why parent↔child
linkage rides `SessionMeta`, not a new store method), ADR-0026 (strategy =
role)

## Context

cogito needs recursive Brain: an agent that can hand a bounded subtask to
a second agent and use its result. The 2026-05-22 roadmap rebalance
**split** the original subagent ADR into two shippable increments:

- **v0.2 Sprint 11 — S2 minimal**: a single `delegate(role, input) →
  output` tool, synchronous, no parent-child event tree, ~200 LoC, **no
  new crate** (lives in `cogito-core::runtime::subagent`).
- **v0.3 — S1 full**: the four-tool lifecycle (`spawn_agent` /
  `wait_agent` / `send_input` / `cancel_agent`), parent-child event tree,
  crash semantics, depth/role session metadata.

This ADR ratifies the v0.2 minimal decision and fixes the protocol seams
so v0.3 can extend them **additively**.

**Forces.**

1. **Layer rule (ADR-0004).** Hands cannot import Runtime. To let a
   tool spawn a child Brain, the spawn capability must be a
   `cogito-protocol` trait injected as a trait object. Brain (and the
   tool, which is Hands-adjacent) sees only the protocol type.
2. **Minimal surface.** The roadmap caps v0.2 at a sync `delegate`. We
   must not pull the v0.3 async lifecycle (`JobManager`-backed spawn /
   wait / cancel) into this sprint.
3. **No cross-turn struct state (Inviolable #3).** Anything a child
   needs after a crash must be rebuildable from an event log.
4. **`ConversationStore` stays single-session (Inviolable #7).** Any
   parent↔child reconstruction is a cross-session query, which must be
   served by reading the backend directly — never by a new trait method.

**Prior art (researched 2026-05-29/30).** Claude Code (Task/Agent tool),
OpenAI Agents SDK (`Agent.as_tool`), Codex CLI subagents, and LangGraph
subagents converge on the **agent-as-tool** model:

- the child starts with a **fresh context** — only the handed `input`
  crosses the parent→child boundary; the child uses its own system
  prompt (its role), not the parent's;
- the parent **model** sees only the child's **final message**;
  intermediate steps stay inside the child;
- the child's transcript is **persisted as a separate session/file**,
  not inlined into the parent's;
- the parent↔child **link on disk is the weak spot both incumbents are
  retrofitting** — Anthropic issue #32175 asks to write
  `parentSessionId` / `parentToolCallId` into the child's
  `session_meta`; Codex child threads carry no persisted parent pointer.

We design that linkage in from day one, child-side, on `SessionMeta`.

## Decision (v0.2 S2 minimal)

### 1. The `delegate` tool

A single tool `delegate(role, input) → output`, `ExecutionClass::AlwaysSync`,
implemented by `DelegateToolProvider` (`impl ToolProvider`) in
`cogito-core::runtime::subagent`. It returns
`InvokeOutcome::Sync(ToolResult::Output([Text(final_text)]))` on success
and `ToolResult::Error { .. }` on any failure. The tool description tells
the model that **`input` is the only channel to the child** — it must
pack every path / fact / decision the child needs into `input`, because
the child inherits **none** of the parent's conversation.

### 2. The `BrainSpawner` trait (`cogito-protocol`)

Sync run-to-completion, with a request struct so v0.3 can add fields
without breaking the signature:

```rust
#[async_trait::async_trait]
pub trait BrainSpawner: Send + Sync {
    /// Run a child agent to completion synchronously and return its
    /// final assistant text. The child is an independent top-level
    /// session; only the returned string crosses back to the caller.
    async fn run_to_completion(
        &self,
        req: DelegateRequest,
    ) -> Result<String, SpawnError>;
}

#[non_exhaustive]
pub struct DelegateRequest {
    pub role: String,                 // strategy name to resolve
    pub input: String,                // child's first user message
    pub parent_session_id: SessionId, // for child-side linkage
    pub parent_call_id: String,       // the delegate tool-call id
    pub parent_depth: u32,            // child opens at parent_depth + 1
}
```

`cogito-core::runtime::Runtime` implements `BrainSpawner`.

### 3. Drive-to-completion mechanism

The spawner reuses the ordinary session machinery — no bespoke turn loop:

1. `runtime.open_session(child_id, New)` with the **resolved role
   strategy** and a child `SessionMeta` (§7). The child gets its own
   actor task, broadcast, and JSONL log.
2. `child.subscribe()` **before** submitting (no race window).
3. `child.submit_user_text(input)`.
4. Loop on the broadcast until a terminal `StreamEvent::TurnCompleted`
   or `TurnFailed { reason }`. Looping (not single-read) means a child
   that pauses on its own async tool (`TurnPaused → TurnResumed →
   TurnCompleted`) is handled transparently.
5. **Extract output by replaying the child log** (`store.replay`,
   last `AssistantMessageAppended`) — this is the *persisted, verbatim
   final message*, the industry-standard return value.
6. `child.shutdown(deadline)` to tear the child actor down cleanly.

### 4. Role resolution

`role` is a **strategy name** (ADR-0026). The spawner holds an
`Arc<dyn StrategyRegistry>` (new optional `RuntimeBuilder` field);
`registry.get(role)` yields the child's `HarnessStrategy` (system prompt,
model params, tool filter, context policy). Unknown role →
`SpawnError::UnknownRole` → `ToolResult::Error`. v0.2 puts **no**
`spawnable_as_subagent` gate on strategies — any known strategy is a valid
role; the depth limit prevents runaway. The opt-in gate is a v0.3 concern.

### 5. Recursion depth guard

`ExecCtx` gains an additive `subagent_depth: u32` (default `0`). The
Runtime populates it per turn from the session's `SessionMeta`
(`subagent_depth`). `DelegateToolProvider::invoke` checks
`ctx.subagent_depth >= max_subagent_depth` (default `3`, set at provider
construction) and returns `ToolResult::Error { kind: DepthExceeded }`
before spawning; otherwise it passes `ctx.subagent_depth` as
`parent_depth`, and the spawner stamps `subagent_depth = parent_depth + 1`
into the child's `SessionMeta`. Because depth is persisted in the child's
seq=0 event and re-read at open, it survives replay and is not
illegal cross-turn struct state. (Per-role `max_subagent_depth` is a v0.3
upgrade.)

### 6. Spawner injection via `ExecCtx`

`ExecCtx` gains an additive `brain_spawner: Option<Arc<dyn BrainSpawner>>`.
The Runtime places a clone of itself-as-`BrainSpawner` on each turn's
`ExecCtx`. Injecting through `ExecCtx` (built per turn, after the Runtime
exists) rather than at `DelegateToolProvider` construction breaks the
Runtime⇄tools construction cycle and keeps Brain protocol-only:
`DelegateToolProvider` reads `ctx.brain_spawner`, and if it is `None`
(spawner not wired) returns a structured error.

### 7. Persistence and parent↔child linkage

The child is a **normal top-level session**: it writes its own
`<child_id>.jsonl` through the **shared** `ConversationStore`, for free.
Because `delegate` is synchronous and in-process, **the child never needs
cross-process resume** (a crash re-runs the parent's `delegate`, spawning
a fresh child); persistence is therefore for **audit / observability /
debuggability**, not correctness — and it is the event-sourcing-consistent
choice (an ephemeral in-memory child would be *more* code for *less*
visibility).

Parent↔child linkage is recorded **child-side only**, as **typed additive
fields on `SessionMeta`** (carried by the child's seq=0 `SessionStarted`):

```rust
pub struct SessionMeta {
    // ... existing: cogito_version, strategy, model, user_id, tenant_id, extra
    pub parent_session_id: Option<SessionId>, // new
    pub parent_call_id:    Option<String>,    // new — the delegate call_id
    pub subagent_depth:    u32,               // new — #[serde(default)], 0 = top-level
}
```

`role` is **not** a new field — it is the child's `SessionMeta.strategy`.
"This session is a subagent" is derivable from
`parent_session_id.is_some()`. Nothing is written to the **parent** log
(no event tree — that is v0.3).

This is backend-portable, which answers the SaaS/Postgres question
directly: the link is **event-payload data written via the existing
single-session `append`**, identical on JSONL and Postgres. The
`ConversationStore` trait needs **no SaaS-specific shape**. Cross-session
reconstruction ("all children of parent X", cost rollups, orphan GC) is a
cross-session query served by reading the backend directly (Inviolable #7,
ADR-0007): the v0.4 Postgres backend projects `parent_session_id` /
`tenant_id` out of the seq=0 `SessionMeta` into indexed columns for the
consumer's `SELECT … WHERE parent_session_id = $1 AND tenant_id = $2` —
a **backend implementation detail invisible to Brain**.

### 8. Live observability bridge

While the parent **model** stays blind (it gets only the final text as the
tool result), the **human** can watch the child work. The spawner — which
is already subscribed to the child broadcast to detect completion (§3) —
re-emits the child's `StreamEvent`s onto the **parent** session's
broadcast, tagged with the originating `delegate` call. `StreamEvent`
gains an additive optional attribution field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
subagent_call_id: Option<String>,
```

This mirrors the incumbent design (Claude Code's `parent_tool_use_id`,
Codex's thread attribution): one event stream, an attribution id, live
view and the parent model's clean context kept separate. The bridged
events are **broadcast-only — never persisted to the parent log** (the
child's own JSONL remains the persisted record), so this does not violate
"parent log stores no child state."

### 9. Failure semantics

Child turn failure or child Brain panic → `SpawnError` → the `delegate`
tool maps it to `ToolResult::Error` with an LLM-readable message
(Inviolable #5). The parent turn continues; the model decides what to do
with the error.

## v0.3 amendment, not built now

Recorded so the v0.2 seams are chosen for additive growth:

- `BrainSpawner` grows `spawn_agent` / `wait_agent` / `send_input` /
  `cancel_agent` (async, `JobManager`-backed); `delegate` /
  `run_to_completion` becomes sugar over `spawn + sync wait`.
- Parent-child **event tree** in the parent log
  (`SubagentSpawned` / `SubagentInputSent` / `SubagentCompleted` with
  `child_session_id`); `parent_session_id` / `depth` formalized as
  session metadata (the `SessionMeta` fields here are the seed).
- Per-role `spawnable_as_subagent` + `max_subagent_depth` on
  `HarnessStrategy`.
- `handed_tools` (expose a subset of the parent's tools to the child).
- Tenant propagation: copy the parent's `tenant_id` into the child's
  `SessionMeta` once `ExecCtx.tenant` lands (v0.4 / ADR-0014).
- Decision point: extract `cogito-subagent` crate if the module exceeds
  ~1k LoC with low dep overlap.

## Consequences

**Easier.**

- Ships in ~200 LoC reusing `open_session` / broadcast / store — no new
  crate, no async-job machinery.
- The child is fully debuggable (its own log) and independently
  replayable.
- SaaS/Postgres needs **zero** `ConversationStore` change; linkage rides
  `SessionMeta`, which already exists for the SaaS-catalog use case.
- All protocol touches are **additive** (ADR-0007/0019 precedent), so
  **no `SCHEMA_VERSION` bump**: `ExecCtx { brain_spawner, subagent_depth }`,
  `StreamEvent { subagent_call_id }`, `SessionMeta { parent_session_id,
  parent_call_id, subagent_depth }`.

**Harder / given up.**

- Sync `delegate` **blocks the parent turn** for the child's full
  duration (no pause/resume). Acceptable for v0.2; v0.3 makes it async.
- A crash mid-`delegate` **orphans** the partially-written child session
  (resume re-runs `delegate` → a fresh child). Documented property, not a
  bug — consistent with "no child state in the parent log."
- Orphan child sessions **accumulate** (no GC). A consumer/SaaS retention
  concern (ROADMAP "what we do not do"), made tractable by the
  `parent_session_id` marker.
- Two protocol surfaces grow now to be reused by v0.3 — a small,
  deliberate forward investment over a strictly throwaway minimal.

## Notes

- The `ExecCtx` module doc comment still says "v0.2 adds `storage`"; that
  is stale (storage moved to v0.5 by the 2026-05-22 rebalance). v0.2 adds
  `brain_spawner` + `subagent_depth`; the comment is corrected as part of
  this sprint.
- The JSON Schema artifact (`docs/schemas/conversation-event-v1.json`) is
  regenerated for the new `SessionMeta` fields, and the CI drift gate
  re-pinned.


---

## v0.3 amendment: Subagent full

**Status**: Proposed (draft, v0.3/v0.4). The v0.2 S2 minimal decision above
is Accepted and shipped (`cogito-core::runtime::subagent::DelegateToolProvider`,
`cogito-protocol::subagent::BrainSpawner::run_to_completion`, the additive
`SessionMeta` linkage fields, and the `ExecCtx { brain_spawner, subagent_depth }`
seams all exist in code). This section is a draft for human ratification; it
adds the full four-tool lifecycle on top of those seams **additively** and
must not be read as accepted.

### Why now

The v0.2 §"v0.3 amendment, not built now" list reserved exactly these seams.
The driver is praxis beginning SaaS integration: a delegating agent that
**blocks the parent turn for the child's whole duration** (the v0.2 sync
`delegate`, see Consequences §"Harder / given up") is unworkable when the
parent itself is a request handler that must stay responsive, fan out several
children, or let a human cancel a runaway child. v0.3 makes the parent→child
hop async and cancellable while keeping every v0.2 consumer's `delegate`
behaviour byte-for-byte unchanged.

### 1. Four tools, one provider

`DelegateToolProvider` grows from one tool to five (the v0.2 `delegate`
stays, see §6). The four new tools are exposed by the same provider's
`list()` and routed by name in `invoke()` — no new crate, no Brain change
(H05 sees four more `ToolDescriptor`s; the layer rule is untouched because
the provider still reads everything through `ExecCtx`):

- `spawn_agent(role, input) -> { child_session_id, job_id }`
  (`ExecutionClass::Async`). Resolves `role`, opens the child as an
  unregistered top-level session exactly as `run_to_completion` does today
  (steps 1-3 of the v0.2 "Drive-to-completion mechanism"), submits the
  child-drive future to `JobManager` instead of awaiting it inline, and
  returns the child session id plus the job id to the model. The parent
  turn does **not** pause here — `spawn_agent` returns synchronously with
  identifiers; the turn continues so the model can spawn siblings.
- `wait_agent(child_session_id) -> output` (`ExecutionClass::Async`). The
  only blocking tool. It returns `InvokeOutcome::Async(job_id)` for the
  child's drive job, so H01 records `TurnPaused { job_id }` and the parent
  turn parks on the existing async-tool pause/resume machinery (H08
  `on_complete` -> `JobCompletedRecorded` -> `TurnResumed`). On resume the
  tool replays the child log (the v0.2 last-`AssistantMessageAppended`
  extraction) and returns the child's final text.
- `send_input(child_session_id, input) -> ack` (`ExecutionClass::AlwaysSync`).
  Pushes an additional user turn into a still-live child via the child
  `SessionHandle::submit_user_text`. Requires the child handle to still be
  resolvable (see §3 registry note). Returns a structured ack, not the
  child's output — the model uses `wait_agent` to collect results. If the
  child has already reached a terminal turn, returns `ToolResult::Error`.
- `cancel_agent(child_session_id) -> ack` (`ExecutionClass::AlwaysSync`).
  Cancels the child's drive job (`JobManager::cancel`) and drives
  `child.shutdown(deadline)`. Idempotent: cancelling an already-finished or
  already-cancelled child is a success ack, not an error.

All five tools share the depth guard (§5) and the
`ctx.brain_spawner.is_none()` "not wired" error path that `delegate`
already implements.

### 2. Trait shape: extend `BrainSpawner` with default methods, not a new trait

**Decision: add the three new lifecycle methods directly to `BrainSpawner`,
each with a `default` body that returns a `SpawnError::Unsupported`
variant.** Do **not** introduce a separate `SubagentLifecycle` trait.

Justification, on the two axes the brief names:

- **v0.2 non-breaking.** Adding methods with default bodies to
  `BrainSpawner` does not break the v0.2 `MockSpawner` / `OkSpawner` test
  impls in `subagent.rs`, nor any consumer impl: they keep compiling and
  simply inherit "unsupported" for the new verbs while their
  `run_to_completion` keeps working. A *new* trait would force the Runtime's
  `RuntimeSpawner` (`builder.rs`) and every test double to implement two
  traits, and would force `ExecCtx` to carry a second `Option<Arc<dyn ...>>`
  — a wider, more error-prone seam for no gain.
- **Object-safety.** `BrainSpawner` is consumed as `Arc<dyn BrainSpawner>`
  (in `ExecCtx::brain_spawner` and the `RuntimeSpawner` impl), so every new
  method must stay object-safe: `async fn` via `#[async_trait]`, no generic
  type parameters, no `where Self: Sized`, `self: &Self` receivers only,
  and request/return types that are themselves `Send`. The new signatures
  obey this:

  ```rust
  #[async_trait::async_trait]
  pub trait BrainSpawner: Send + Sync {
      // v0.2, unchanged.
      async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError>;

      // v0.3, additive. Default bodies keep v0.2 impls compiling.
      /// Open the child and start driving it on a background job; return the
      /// child session id and the drive job id. Does not block.
      async fn spawn(&self, req: DelegateRequest) -> Result<SpawnHandle, SpawnError> {
          Err(SpawnError::Unsupported { verb: "spawn" })
      }
      /// Resolve the drive job for an outstanding child, for the parent turn
      /// to park on via `InvokeOutcome::Async`.
      async fn drive_job(&self, child: SessionId) -> Result<JobId, SpawnError> {
          let _ = child;
          Err(SpawnError::Unsupported { verb: "wait" })
      }
      /// Push an additional user turn into a still-live child.
      async fn send_input(&self, child: SessionId, input: String) -> Result<(), SpawnError> {
          let _ = (child, input);
          Err(SpawnError::Unsupported { verb: "send_input" })
      }
      /// Cancel the child's drive job and shut the child actor down.
      async fn cancel(&self, child: SessionId) -> Result<(), SpawnError> {
          let _ = child;
          Err(SpawnError::Unsupported { verb: "cancel" })
      }
  }
  ```

  `SpawnHandle { child_session_id: SessionId, job_id: JobId }` and
  `SpawnError::Unsupported { verb: &'static str }` are the only new
  protocol types; both are additive. `SpawnError` is already
  `#[non_exhaustive]`, so adding `Unsupported` does not break existing
  `match` arms that have a catch-all (the `delegate` tool's `map_spawn_error`
  already has a `_ =>` arm).

### 3. JobManager-backed async wait

`spawn`'s background work is the *same* drive loop `run_to_completion`
runs today (subscribe -> `submit_user_text` -> loop to terminal
`TurnCompleted`/`TurnFailed` -> `shutdown`), but submitted as a boxed
future to `JobSubmitter::submit_boxed` (`cogito-protocol::job`) rather than
awaited inline. The returned `JobId` is the drive job. `wait_agent` then
reuses the existing async-tool contract verbatim: it returns
`InvokeOutcome::Async(job_id)`, H08 records `JobSubmitted` is **not** re-run
(the job already exists) — instead `wait_agent` registers an
`on_complete` sink on that job id and H01 parks the parent on
`TurnPaused { job_id }`. The child's terminal `JobOutcome` (success /
failed / cancelled) drives `JobCompletedRecorded` -> `TurnResumed`, after
which `wait_agent` reads the child log for the final text. This is why the
async surface needs **no new turn-driver state**: the subagent wait rides
the same pause/resume path async tools already use.

Open registry question (see Open questions): `send_input` and `cancel_agent`
need the child `SessionHandle` to still be live. The child is opened
*unregistered* in v0.2 (the spawner holds the only handle, inside the drive
future). For v0.3 the spawner must keep a parent-call-scoped map
`child_session_id -> SessionHandle` for the lifetime of the drive job so
`send_input`/`cancel` can reach it. This map is **process-local runtime
state, not conversation state** (consistent with ADR-0028 §5: a live handle
is code/IO, not replayable state) — and it is exactly the
`get_session`/`close_session` registry concern ADR-0034 raises. The
cleanest landing is to register spawned children in the same `sessions`
DashMap that ADR-0034 makes reopenable, keyed by `child_session_id`, and
deregister on drive-job completion.

### 4. Additive event tree in the PARENT log

v0.2 wrote **nothing** to the parent log (linkage was child-side only on
`SessionMeta`). v0.3 adds three additive `EventPayload` variants written to
the **parent** session's log, giving the parent a replayable record of its
outstanding children. These follow the ADR-0007 additive-variant precedent
used by `JobSubmitted` / `ThinkingBlockRecorded` (ADR-0019) — **no
`SCHEMA_VERSION` bump** (it stays at 1; `EventPayload` is `#[non_exhaustive]`
and the variants default-absent on read of older logs):

```rust
// cogito-protocol::event::EventPayload — three new variants, snake_case tagged.
SubagentSpawned {
    call_id: String,          // the spawn_agent tool-call id
    child_session_id: SessionId,
    job_id: JobId,            // the child drive job
    role: String,
},
SubagentInputSent {
    child_session_id: SessionId,
    // input text is NOT inlined (event log is not a transcript cache,
    // ADR-0007); it is persisted in the CHILD's own log as its user turn.
},
SubagentCompleted {
    child_session_id: SessionId,
    outcome: JobOutcome,      // success / failed / cancelled — reuses the job type
},
```

`SubagentSpawned` is written when `spawn_agent` returns;
`SubagentCompleted` when the drive job reaches a terminal outcome (alongside
the existing `JobCompletedRecorded`). The parent model still never sees the
child's *intermediate* steps — these are coarse lifecycle markers, not the
child transcript. The child transcript remains its own `<child_id>.jsonl`
(v0.2 §7, unchanged). The live observability bridge (`subagent_call_id` on
`StreamEvent`, v0.2 §8) is unchanged and still broadcast-only.

### 5. Depth-limit enforcement (unchanged mechanism)

`DEFAULT_MAX_SUBAGENT_DEPTH = 3` already exists
(`subagent.rs`), checked in `DelegateToolProvider::invoke` against
`ctx.subagent_depth`. All four new tools reuse the identical guard before
spawning: `spawn_agent` rejects with `ToolResult::Error` when
`ctx.subagent_depth >= max_depth`; `wait_agent`/`send_input`/`cancel_agent`
do not deepen the tree and need no guard. The child still opens at
`parent_depth + 1`, stamped into the child `SessionMeta.subagent_depth`
(v0.2 §5), so depth survives replay. Per-role `max_subagent_depth` on
`HarnessStrategy` remains a future upgrade — v0.3 keeps the single
construction-time default; if praxis needs per-tenant caps we set it via the
provider built per `SessionSpec` (ADR-0028), not via a new protocol field.

### 6. `delegate` is sugar — zero behaviour change for v0.2 consumers

`run_to_completion` is redefined as `spawn` followed by an immediate inline
wait on the returned drive job, then the same log-replay extraction. The
`delegate` tool keeps its v0.2 contract exactly: `ExecutionClass::AlwaysSync`,
returns the child's final text or a `ToolResult::Error`, blocks the parent
turn. A consumer that only knows `delegate` sees no difference — same tool
name, same schema, same blocking semantics, same error mapping. The only
internal change is that `run_to_completion`'s default impl now layers over
`spawn` + wait instead of owning the drive loop directly. Because
`run_to_completion` keeps a concrete body on `RuntimeSpawner`, a spawner
that does *not* implement the v0.3 verbs (returns `Unsupported`) can still
keep its own monolithic `run_to_completion` — the sugar is opt-in.

### 7. Parent-resume-with-outstanding-child crash semantics (the load-bearing decision)

This is the first time parent↔child linkage must be **rebuildable from the
parent log** (Inviolable #3) — v0.2 sidestepped it because sync `delegate`
had no "outstanding child" state to recover (a crash mid-`delegate` simply
re-ran `delegate` and spawned a fresh child; the half-written child was an
accepted orphan, see v0.2 Consequences). With async spawn, a parent can
crash while one or more children are still running, and resume must not
hang forever waiting on a job that died with the old process, nor silently
lose the child.

The recovery contract:

1. **The parent log is the source of truth for "which children exist."**
   On resume, replaying the parent log yields every `SubagentSpawned` with
   its `child_session_id` and `job_id`, minus those with a matching
   `SubagentCompleted`. That difference is the set of *outstanding children*
   — reconstructed purely from the log, no struct state carried across the
   crash. This is what makes the v0.3 tree (§4) necessary and not merely
   decorative: it is the parent-side anchor Inviolable #3 demands.

2. **`JobId` does not survive a process restart.** `JobManager` is
   in-process (`cogito-jobs`); a fresh Runtime has an empty job registry, so
   the old `job_id` is dead. Therefore on resume an outstanding child's
   drive job is treated as **lost, not pending.** The parent does not block
   on it.

3. **Resolution policy (drafted, see Open questions for the live choice):**
   for each outstanding child, the resumed parent reads the *child's* log
   (`store.replay(child_session_id)`):
   - if the child log shows a terminal turn -> synthesize the missing
     `SubagentCompleted` from the child's last state and let any parked
     `wait_agent` resolve from the child log (the child finished even though
     the parent crashed; its `<child_id>.jsonl` is intact and authoritative);
   - if the child log shows no terminal turn -> the child died mid-flight
     with the process. The resumed parent writes
     `SubagentCompleted { outcome: Cancelled }` and any parked `wait_agent`
     returns a `ToolResult::Error` ("subagent did not survive parent
     restart") so the parent model decides whether to re-`spawn_agent`. We do
     **not** auto-respawn — re-execution policy is the model's, matching the
     v0.2 "the model decides what to do with the error" principle (§9).

4. **No new store method.** All of the above is the existing single-session
   `replay` applied to two session ids (parent, then each child). Reading a
   child's log from the parent's resume path is a cross-session *read*,
   served by the backend directly (Inviolable #7 / ADR-0007), identical on
   JSONL and Postgres. The Postgres backend's projection of
   `parent_session_id` out of seq=0 `SessionMeta` (v0.2 §7) is what lets a
   SaaS replica answer "find this parent's outstanding children" without
   scanning — but correctness does not depend on it; the parent log alone
   suffices.

The net property: a crashed parent with N outstanding children resumes
deterministically — finished children are reaped from their own logs,
in-flight children are marked cancelled and surfaced to the model as
errors — with **no cross-process job handoff** and **no child state stored
in the parent log beyond the spawn/complete markers**.

### 8. Crate extraction stays DEFERRED

The v0.2 list set the bar at "~1k LoC with low dep overlap" for splitting
out a `cogito-subagent` crate. The current module is **301 LoC**
(`cogito-core/src/runtime/subagent.rs`) plus 114 in
`cogito-protocol/src/subagent.rs`. v0.3's four tools, the
default-method trait growth, and the resume policy will add perhaps a few
hundred LoC — still comfortably under the 1k criterion, and still tightly
coupled to `open_inner` / `sessions` / `JobManager` inside the Runtime.
**No crate extraction in v0.3.** Re-evaluate only if `handed_tools` and
per-role gating (still future) push it past the threshold.

### Consequences (delta over v0.2)

**Easier.**

- Parent stays responsive: fan out several children, let a human cancel one,
  collect results out of order — none of which the v0.2 sync `delegate`
  allowed. Directly serves a praxis request-handler-as-parent.
- The parent now has a replayable record of its children
  (`SubagentSpawned`/`SubagentCompleted`), so crash recovery is
  deterministic instead of "orphan and re-run."
- All growth is additive: no `SCHEMA_VERSION` bump, no breaking trait
  change, v0.2 `delegate` consumers untouched.

**Harder / given up.**

- The spawner must keep a process-local `child_session_id -> SessionHandle`
  map for `send_input`/`cancel` (depends on the ADR-0034 registry making
  children resolvable). Lifetime management of that map is new surface.
- Resume now has a real reconciliation step (§7) — the resume-chaos suite
  must gain a "parent crashes with an outstanding child" scenario asserting
  the finished-child-reaped and in-flight-child-cancelled branches.
- Cross-process job handoff is explicitly **not** attempted: an in-flight
  child does not survive a parent restart (it is cancelled and surfaced as
  an error). Durable child execution across replicas is out of scope for
  v0.3.

### Open questions

- §7.3 resolution policy: confirm "no auto-respawn, surface as error to the
  model" vs. a strategy-level opt-in to auto-respawn idempotent children.
  Drafted as no-auto-respawn; needs human ratification.
- §3 registry: settle whether spawned children register in the ADR-0034
  `sessions` DashMap (cleanest, couples this ADR's landing to ADR-0034) or
  the spawner keeps a private per-parent map. Drafted as "register in the
  shared registry."
- `send_input` semantics on a child that is *between* turns vs. *mid-turn*:
  reuse the single-slot mid-pause input discipline (ADR-0028 §3) or reject
  mid-turn. Drafted as reuse-the-existing-discipline; confirm.
- Whether `wait_agent` on an already-completed child (the parent crashed,
  child finished, parent resumed) should return immediately from the child
  log without re-parking. Drafted as yes (read-through, no pause); confirm
  it composes with H08's `on_complete` registration.

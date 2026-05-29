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

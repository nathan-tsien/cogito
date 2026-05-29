# cogito-subagent — Hands / Runtime (v0.2 module in cogito-core::runtime::subagent)

The home of `delegate(role, input) -> output`: a single synchronous tool
that hands a self-contained subtask to a child agent and returns its final
message. The child is a fresh, independent Brain instance hosted by the same
Runtime; only the `input` string crosses into it and only the final
assistant text crosses back. v0.2 ships this as a **module**, not a crate —
`cogito-core::runtime::subagent` (`DelegateToolProvider`, the
`DELEGATE_TOOL_NAME` const, `DEFAULT_MAX_SUBAGENT_DEPTH = 3`) — because the
minimal surface is ~200 LoC reusing existing session machinery. v0.3 may
extract a `cogito-subagent` crate once the full lifecycle lands (ADR-0011
§"v0.3 amendment, not built now"). Like `cogito-sandbox`, this is **not** an
H-numbered Harness component: it is a Runtime-layer concern invisible to the
Brain except through the `ExecCtx.brain_spawner` protocol seam (ADR-0004).

Authoritative rationale: [ADR-0011](../adr/0011-subagent-execution-model.md).
Spec: [docs/superpowers/specs/2026-05-30-sprint-11-subagent-minimal-design.md](../superpowers/specs/2026-05-30-sprint-11-subagent-minimal-design.md).

> **Status**: v0.2 minimal — shipped Sprint 11.

## Position

```
Parent Brain (cogito-core::harness)   sees ToolProvider + BrainSpawner only
  │  H08 dispatch
  ▼
DelegateToolProvider (Hands)          AlwaysSync; reads ExecCtx, guards depth
  │  reads ctx.brain_spawner
  ▼
BrainSpawner (cogito-protocol)        ◄── the protocol seam (ADR-0004)
  │  run_to_completion(DelegateRequest)
  ▼
RuntimeSpawner(Arc<Runtime>)          Runtime: opens + drives the child
  │  open_inner(register = false)
  ▼
Child top-level session               own actor, broadcast, JSONL log
```

The Brain sees only two things from `cogito-protocol`: the `ToolProvider`
trait (which H08 dispatches) and the `BrainSpawner` protocol type (which the
tool reads off `ExecCtx`). It never names `Runtime`, `RuntimeSpawner`, or any
concrete session type — that would be a layer violation (ADR-0004: Hands
cannot import Runtime). The concrete `RuntimeSpawner` newtype is wired in by
the Runtime layer and injected as a trait object, exactly as
`cogito-sandbox::DirectExecutor` is wired behind the `CommandExecutor` seam.

## The two valves

The mental model is two one-way valves around a sealed child execution, the
agent-as-tool pattern that Claude Code, the OpenAI Agents SDK, and LangGraph
subagents all converge on.

- **Initial-context valve (parent → child, write-once).** The child starts
  with a **fresh context**. It inherits **none** of the parent's
  conversation. Only two values cross the boundary: `input` becomes the
  child's first user message, and `role` selects the child's strategy (its
  system prompt, model params, tool filter, context policy). The tool
  description makes the contract explicit to the model — "the child sees NONE
  of this conversation, so pack every file path, fact, and decision it needs
  into `input`." If it is not in `input`, the child does not have it.

- **Sealed execution.** While the child runs, its intermediate steps stay
  inside the child. The parent **model** is blind to them: it will only ever
  see the final text returned as the tool result. The **human**, however, is
  not blind — the spawner re-emits the child's stream onto the parent's
  broadcast (see "Drive-to-completion" and ADR-0011 §8), so a UI watching the
  parent shows live subagent progress. Two audiences, one stream, kept
  separate by an attribution id.

- **Result valve (child → parent, read-once).** When the child reaches a
  terminal turn, its **verbatim final assistant text** is returned as the
  `delegate` tool result (`ToolResult`). Nothing else crosses back — not the
  child's transcript, not its tool calls, not its token usage.

## The BrainSpawner seam

The capability to spawn a child Brain is the trait
`cogito_protocol::subagent::BrainSpawner`:

```rust
#[async_trait::async_trait]
pub trait BrainSpawner: Send + Sync {
    async fn run_to_completion(
        &self,
        req: DelegateRequest,
    ) -> Result<String, SpawnError>;
}

#[non_exhaustive]
pub struct DelegateRequest {
    pub role: String,                 // strategy name to resolve
    pub input: String,                // child's first user message
    pub parent_session_id: SessionId, // recorded child-side for linkage
    pub parent_call_id: String,       // the delegate tool-call id
    pub parent_depth: u32,            // child opens at parent_depth + 1
}

#[non_exhaustive]
pub enum SpawnError {
    UnknownRole { role: String },
    OpenFailed { reason: String },
    ChildFailed { reason: String },
    Timeout { seconds: u64 },
}
```

`run_to_completion` is **synchronous in spirit**: the caller awaits it inline
until the child reaches a terminal turn. There is no background job and no
`JobId` — that asynchrony is the v0.3 upgrade.

**Why the trait lives in `cogito-protocol`.** The layer rule (ADR-0004) says
Hands cannot import Runtime. `DelegateToolProvider` is Hands-adjacent and the
spawn capability lives in Runtime, so the only way to let the tool reach the
Runtime is through a Protocol trait it depends on instead. Brain and the tool
see only `BrainSpawner`; the `Runtime` implements it. `DelegateRequest` and
`SpawnError` are `#[non_exhaustive]` so v0.3 can add request fields (e.g.
`handed_tools`) and error variants without breaking call sites.

**Why it is injected via `ExecCtx`, not at construction.** `ExecCtx` carries
`brain_spawner: Option<Arc<dyn BrainSpawner>>`, set per turn after the Runtime
already exists. Injecting at `DelegateToolProvider::new` would create a
Runtime⇄tools construction cycle (the Runtime owns the tools, the tools would
need the Runtime). Per-turn injection breaks that cycle and keeps Brain
protocol-only. The concrete implementation is
`RuntimeSpawner(Arc<Runtime>)` — a newtype that owns exactly the
`Arc<Runtime>` that `run_to_completion` needs to call `open_inner`, and which
lets a spawned child itself delegate (its own session's `ExecCtx` carries a
fresh `RuntimeSpawner`). If `brain_spawner` is `None` (spawner not wired), the
tool returns a structured error rather than panicking.

**Error mapping (Inviolable #5).** `DelegateToolProvider` maps every
`SpawnError` to a `ToolResult::Error`. `SpawnError::Timeout` maps to
`ToolErrorKind::Timeout` with `retryable: true` (a child that ran out of time
is transient — the parent strategy may retry). The other three variants
(`UnknownRole`, `OpenFailed`, `ChildFailed`) map to
`ToolErrorKind::InvocationFailed` with `retryable: false` (deterministic;
retrying the same args will not help). The depth-guard rejection and a missing
spawner are also `InvocationFailed`; malformed tool args are `InvalidArgs`.

## Drive-to-completion

`RuntimeSpawner::run_to_completion` reuses the ordinary session machinery — no
bespoke turn loop:

1. **Resolve the role.** `strategy_registry.get(role)` yields the child's
   `HarnessStrategy`. Unknown role → `SpawnError::UnknownRole`.
2. **Open the child** via `open_inner(child_id, New, strategy, Some(meta),
   parent_depth + 1, register = false)`. `register = false` means the child is
   a real top-level session — own actor task, own broadcast, own JSONL log —
   but is **not** added to the Runtime's in-memory session registry; it is a
   private child, not a session a surface can address by id.
3. **Subscribe before submitting.** `child.subscribe()` is called before any
   input is sent, so there is no race window where early child events are
   missed.
4. **Submit** `child.submit_user_text(input)`.
5. **Loop** on the broadcast stream until a terminal turn —
   `StreamEvent::TurnCompleted` (stop and replay) or `TurnFailed { reason }`
   (return `SpawnError::ChildFailed`). Looping rather than reading a single
   event means a child that **pauses on its own async tool**
   (`TurnPaused → TurnResumed → TurnCompleted`) is handled transparently;
   intermediate events are forwarded and otherwise ignored. A lagged or closed
   broadcast falls through to the replay.
6. **Backstop deadline.** The whole drive is wrapped in a
   `CHILD_DRIVE_TIMEOUT` of 300s. A wedged child that never reaches a terminal
   turn cannot hang the parent forever: the timeout fires and yields
   `SpawnError::Timeout`. The child does **not** inherit the parent's cancel
   token in v0.2, so this backstop is the only guard against an unbounded
   child turn.
7. **Tear down** with `child.shutdown(CHILD_SHUTDOWN_GRACE)` (5s grace),
   regardless of how the drive ended.
8. **Replay for the result.** `store.replay(child_id, 0)` is walked
   newest-first for the last non-empty
   `EventPayload::AssistantMessageAppended { text }` — the persisted, verbatim
   final message. A child that completed with **no** assistant text yields
   `""` in v0.2; distinguishing "completed-empty" from real output so the
   parent can surface a clearer signal is a v0.3 item.

## Persistence and parent-child linkage

The child is a **normal independent top-level session**: it writes its own
log through the **shared** `ConversationStore`, for free. In the JSONL store
the layout is **flat** — `<root>/<session_id>.jsonl`, the same path shape a
top-level session uses, not a nested `sessions/` directory. Because
`delegate` is synchronous and in-process, the child never needs cross-process
resume (a parent crash re-runs `delegate`, spawning a fresh child), so
persistence here is for audit, observability, and debuggability rather than
correctness — and it is the event-sourcing-consistent choice (an ephemeral
in-memory child would be more code for less visibility).

**Linkage is recorded child-side only**, as typed additive fields on
`SessionMeta`, carried by the child's seq=0 `SessionStarted` event:

- `parent_session_id: Option<SessionId>`
- `parent_call_id: Option<String>` — the `delegate` tool-call id
- `subagent_depth: u32` — 0 for a top-level session, `>= 1` for a child

`role` is **not** a new field — it is the child's `SessionMeta.strategy`.
"This session is a subagent" is derivable from `parent_session_id.is_some()`.
Nothing is written to the **parent** log beyond the ordinary
`ToolResultRecorded` for the `delegate` call; there is no parent-side event
tree until v0.3.

**Invariant.** A child's `SessionMeta` always pairs `subagent_depth >= 1`
with `Some(parent_session_id)` and `Some(parent_call_id)`. `RuntimeSpawner` is
the sole producer of child sessions and always sets
`subagent_depth = parent_depth + 1` alongside both parent fields, so the
invariant holds by construction. A smart constructor that enforces it as a
type-level guarantee is deferred to v0.3.

**SaaS / Postgres.** The `ConversationStore` trait is **unchanged** — it stays
single-session (Inviolable #7). Cross-session parent → child queries ("all
children of parent X", cost rollups, orphan GC) are a **backend** detail, not
a trait method (ADR-0007). The JSONL backend (v0.1) answers them by reading
files directly; a Postgres backend (v0.4) projects `parent_session_id` out of
the seq=0 `SessionMeta` into an indexed column and serves
`SELECT … WHERE parent_session_id = $1` — invisible to Brain, no new trait
method.

**Runtime lifetime retention.** Every session's `SessionDeps` carries
`Some(RuntimeSpawner(Arc::clone(self)))`, so any session can itself delegate.
The spawned actor task owns that `Arc<Runtime>` clone, which means the Runtime
stays alive until each session's actor exits — dropping the external
`Arc<Runtime>` handles alone does **not** tear it down. This is intentional: a
child mid-delegate must keep the Runtime alive long enough to finish. There is
no Arc cycle, because a `SessionHandle` holds no back-reference to the Runtime.

## Depth, failure, and resume

**Depth guard.** `ExecCtx` carries `subagent_depth: u32`, populated per turn by
the Runtime from the session's `SessionMeta`. `DelegateToolProvider::invoke`
checks `ctx.subagent_depth >= max_depth` (default
`DEFAULT_MAX_SUBAGENT_DEPTH = 3`, set at provider construction and overridable
via `ToolsConfig.max_subagent_depth` in `cogito.toml`) and returns a
`ToolResult::Error` before spawning. Otherwise it passes `ctx.subagent_depth`
as `parent_depth`, and the spawner stamps `subagent_depth = parent_depth + 1`
into the child. Because depth is persisted in the child's seq=0 event and
re-read at open, it survives replay and is not illegal cross-turn struct
state. Per-role `max_subagent_depth` is a v0.3 upgrade.

**Failure.** Child turn failure, child Brain panic, or the backstop timeout
all surface as a `SpawnError`, which the `delegate` tool maps to a
`ToolResult::Error` with an LLM-readable message (Inviolable #5: structured
errors, no panics or `unwrap`s reaching the model). The parent turn continues;
the model decides what to do with the error.

**Resume.** Because `delegate` is synchronous and in-process, the resume story
is the standard one — there is no subagent-specific resume logic. If the
parent crashes mid-`delegate`, resume re-runs the `delegate` tool from the
parent's event log, which spawns a **fresh** child. The partially-written
child session from the crashed attempt is **orphaned** (no GC). This is a
documented property, not a bug — it is consistent with "the parent log stores
no child state," and v0.3 revisits it alongside the parent-child event tree.

## Not in v0.2 (v0.3 amendment)

The following are deliberately out of scope for v0.2 and recorded in ADR-0011
§"v0.3 amendment, not built now" so the v0.2 seams are chosen for additive
growth:

- The async four-tool lifecycle (`spawn_agent` / `wait_agent` /
  `send_input` / `cancel_agent`, `JobManager`-backed); `delegate` becomes
  sugar over `spawn + sync wait`.
- The parent-child **event tree** in the parent log (`SubagentSpawned` /
  `SubagentInputSent` / `SubagentCompleted` with `child_session_id`).
- Per-role `spawnable_as_subagent` opt-in gate and per-role
  `max_subagent_depth` on `HarnessStrategy`.
- `handed_tools` — exposing a subset of the parent's tools to the child.
- Tenant propagation (copy the parent's `tenant_id` into the child's
  `SessionMeta` once `ExecCtx.tenant` lands).
- Extraction of a dedicated `cogito-subagent` crate (decision point: if the
  module exceeds ~1k LoC with low dep overlap).
- Parent → child cancel-token propagation (today the 300s backstop is the
  only guard).
- A dedicated subagent chaos scenario.

## References

- [ADR-0011](../adr/0011-subagent-execution-model.md) (subagent execution
  model — v0.2 minimal + v0.3 amendment)
- [ADR-0004](../adr/0004-brain-hands-session-boundaries.md) (layering — why
  `BrainSpawner` is a Protocol trait)
- [ADR-0007](../adr/0007-event-log-as-cross-language-contract.md) (event log as
  cross-language contract — why linkage rides `SessionMeta`)
- [H08 Tool Dispatcher](H08-tool-dispatcher.md) (sets `ExecCtx.call_id`
  before `invoke`; dispatches `delegate` like any other `AlwaysSync` tool)
- `crates/cogito-protocol/src/subagent.rs`,
  `crates/cogito-core/src/runtime/subagent.rs`,
  `crates/cogito-core/src/runtime/builder.rs` (`RuntimeSpawner`)
</content>
</invoke>

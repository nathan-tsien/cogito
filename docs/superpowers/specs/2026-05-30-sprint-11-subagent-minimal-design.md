# Sprint 11 — Subagent (S2 minimal) design

**Date**: 2026-05-30
**Version / sprint**: v0.2 Extensibility · Sprint 11
**ADR**: `docs/adr/0011-subagent-execution-model.md` (ratifies the
decisions; read it first)
**Status**: design approved, pending implementation

## Goal

Give Brain a `delegate(role, input) → output` tool: hand a bounded
subtask to a child agent that runs to completion in a fresh, isolated
session and returns only its final text. Synchronous, ~200 LoC, **no new
crate** — the module lives in `cogito-core::runtime::subagent`.

Non-goals (v0.3): async spawn/wait/cancel lifecycle, parent-child event
tree, `handed_tools`, per-role spawnable gate. See ADR-0011
§"v0.3 amendment".

## Mental model (the contract)

A `delegate` call is a **sealed sub-session with two narrow valves**:

- **In (parent → child):** only `input` crosses. `role` selects the
  child's own strategy (system prompt + model + tools). The child sees
  **none** of the parent's history — so the model must pack everything
  the child needs into `input`. (This is the "agent-as-tool" model, not
  "handoff".)
- **Sealed middle:** the child runs its own multi-turn loop. The parent
  **model** never sees the child's intermediate steps — only the final
  text. The **human** can watch via a live observability bridge (§6).
- **Out (child → parent):** the child's final assistant message, verbatim,
  becomes the `delegate` tool result. Failure → `ToolResult::Error`.

## Architecture

```
                cogito-core::runtime
  ┌──────────────────────────────────────────────────────────┐
  │  Runtime  ── impl BrainSpawner ──────────────┐            │
  │    holds: store, model, tools, job_mgr,       │            │
  │           strategy_registry (new), handle     │            │
  │                                               ▼            │
  │  runtime::subagent::DelegateToolProvider                  │
  │    impl ToolProvider                                       │
  │      invoke("delegate", {role,input}, ctx):               │
  │        depth guard on ctx.subagent_depth                  │
  │        spawner = ctx.brain_spawner                        │
  │        spawner.run_to_completion(DelegateRequest{..})     │
  └──────────────────────────────────────────────────────────┘
                         │ (protocol trait only — layer rule)
  cogito-protocol:  BrainSpawner, DelegateRequest, SpawnError
                    ExecCtx{ +brain_spawner, +subagent_depth }
                    StreamEvent{ +subagent_call_id }
                    SessionMeta{ +parent_session_id, +parent_call_id,
                                 +subagent_depth }
```

Brain (`harness/`) is untouched: `delegate` is just another tool in the
composite provider, and the spawner reaches Brain only as
`ExecCtx.brain_spawner: Arc<dyn BrainSpawner>` (a protocol type).

## Components

### C1 — Protocol additions (`cogito-protocol`)

All additive; **no `SCHEMA_VERSION` bump** (ADR-0007/0019 precedent).

- `subagent.rs` (new module): `BrainSpawner` trait, `DelegateRequest`
  (`#[non_exhaustive]` struct), `SpawnError` (`thiserror`):
  `UnknownRole { role }`, `DepthExceeded { depth, max }`,
  `SpawnerUnavailable`, `ChildFailed { reason }`, `Store(StoreError)`.
- `exec_ctx.rs`: add `brain_spawner: Option<Arc<dyn BrainSpawner>>` and
  `subagent_depth: u32`. `open_ended` defaults them to `None` / `0`. Fix
  the stale "v0.2 adds storage" doc comment.
- `stream.rs`: add `subagent_call_id: Option<String>`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`) to the
  `StreamEvent` variants the bridge forwards (at minimum `TextDelta`,
  `TurnStarted`, `TurnCompleted`, `TurnFailed`, `ToolDispatchStarted/Ended`).
  Decision: add it as a sibling field on those variants (keeps internal
  tagging intact) rather than a wrapper variant.
- `session.rs`: add `parent_session_id: Option<SessionId>`,
  `parent_call_id: Option<String>`, `subagent_depth: u32`
  (`#[serde(default)]`) to `SessionMeta`. Roundtrip + forward-compat
  tests extended.

### C2 — `DelegateToolProvider` (`cogito-core::runtime::subagent`)

`impl ToolProvider`:

- `list()` → one `ToolDescriptor` for `delegate` with a JSON schema
  `{ role: string, input: string }`, `ExecutionClass::AlwaysSync`, and the
  "pack everything into `input`" description.
- `invoke("delegate", args, ctx)`:
  1. Parse/validate args → `ToolResult::Error { InvalidArgs }` on failure.
  2. `if ctx.subagent_depth >= self.max_depth` → `Error { DepthExceeded }`.
  3. `let spawner = ctx.brain_spawner.ok_or(SpawnerUnavailable)?`.
  4. `spawner.run_to_completion(DelegateRequest { role, input,
     parent_session_id: ctx.session_id, parent_call_id: <this call_id>,
     parent_depth: ctx.subagent_depth }).await`.
  5. `Ok(text)` → `Sync(Output([Text(text)]))`; `Err(e)` → mapped
     `Error`.
- Construction: `DelegateToolProvider::new(max_depth)` (default 3). Holds
  no `Runtime` / spawner ref — the spawner arrives via `ExecCtx`.

> `call_id`: H08 owns the tool-call id. `invoke` needs it for
> `parent_call_id`. Confirm during implementation whether `ExecCtx`
> already carries the call id; if not, the smallest fix is to pass it in
> (it is per-dispatch, like `turn_id`). Flagged as the one open wiring
> detail.

### C3 — `Runtime: impl BrainSpawner` (`cogito-core::runtime`)

`run_to_completion(req)`:

1. `strategy = self.strategy_registry.get(&req.role)?` (→ `UnknownRole`).
2. Build child `SessionMeta`: `strategy = role`, `model` from
   strategy, `parent_session_id = Some(req.parent_session_id)`,
   `parent_call_id = Some(req.parent_call_id)`,
   `subagent_depth = req.parent_depth + 1`. (Tenant inheritance deferred
   to v0.4.)
3. `child_id = SessionId::new()`; open the child via an internal path
   that uses `strategy` + this `SessionMeta` instead of the Runtime
   default (see C4).
4. Subscribe → `submit_user_text(req.input)` → bridge + await terminal
   (C5/§6).
5. On `TurnCompleted`: `replay(child_id, 0)`, take the last
   `AssistantMessageAppended` text. On `TurnFailed { reason }` →
   `Err(ChildFailed { reason })`.
6. `child_handle.shutdown(short_deadline)`; return the text.

### C4 — Runtime wiring

- `RuntimeBuilder`: add optional
  `strategy_registry(Arc<dyn StrategyRegistry>)`. Stored on `Runtime`.
  If absent, `run_to_completion` returns `UnknownRole`/`SpawnerUnavailable`
  consistently (delegate degrades to a structured error, never a panic).
- Per-turn `ExecCtx` construction (turn driver / dispatcher path): set
  `brain_spawner = Some(self.clone() as Arc<dyn BrainSpawner>)` and
  `subagent_depth = <session meta subagent_depth>`. Thread the session's
  depth from `SessionState` (read from seq=0 `SessionMeta` at open).
- Internal child-open path: factor the existing `open_session` body so
  the child can be opened with an explicit `(strategy, SessionMeta)`
  override. `SessionStarted` is still written exactly once at seq=0.

### C5 — CLI wiring (`cogito-cli chat`)

- Build `DelegateToolProvider::new(max_depth)` and add it to the
  composite (after builtin/run_tests/bash, before MCP), so the tool
  surface includes `delegate`.
- Pass the already-built `FsStrategyRegistry` into
  `RuntimeBuilder::strategy_registry`.
- `max_subagent_depth` read from config `[tools]`/`[subagent]` (default 3).

## Data flow (happy path)

```
parent turn: model emits ToolUse delegate{role:"reviewer", input:"…"}
  H08 → DelegateToolProvider.invoke(.., ctx{subagent_depth:0, brain_spawner})
    depth 0 < 3 ✓
    spawner.run_to_completion(role:"reviewer", input, parent_session, call_id, depth:0)
      registry.get("reviewer") → strategy
      open child session C (meta: parent=parent_session, call_id, depth:1)
      subscribe(C); submit_user_text(input)
      ── child runs its own turns; spawner forwards C's StreamEvents to
         the PARENT broadcast tagged subagent_call_id=call_id ──
      StreamEvent::TurnCompleted(C)
      replay(C) → last AssistantMessageAppended → "…final review…"
      shutdown(C)
    Ok("…final review…")
  ToolResult::Output([Text("…final review…")])  → recorded in PARENT log
parent turn: model sees the tool result, continues
```

Child log `C.jsonl` holds the full child transcript;
`C.SessionStarted.meta.parent_session_id` links it back. Parent log holds
only the normal `ToolUseRecorded` / `ToolResultRecorded` for the
`delegate` call.

## Error handling

| Situation | Result to parent model |
|---|---|
| Bad args | `ToolResult::Error { InvalidArgs }` |
| `ctx.subagent_depth >= max` | `Error { DepthExceeded }` |
| Unknown `role` | `Error` (from `UnknownRole`) |
| Spawner not wired (`ctx.brain_spawner == None`) | `Error` (from `SpawnerUnavailable`) |
| Child turn ends `TurnFailed` | `Error` (from `ChildFailed { reason }`) |
| Child Brain panics | Runtime `catch_unwind` on the child task → child `TurnFailed` → `ChildFailed` (Inviolable #5) |
| Process crash mid-`delegate` | resume re-runs `delegate`; a fresh child spawns; old child log orphaned (documented) |

## Testing

- **Protocol unit tests**: serde roundtrip + forward-compat (unknown
  field ignored) for new `SessionMeta` fields, `StreamEvent`
  `subagent_call_id`, `DelegateRequest`. `ExecCtx` default values.
- **`DelegateToolProvider` unit tests** (with a mock `BrainSpawner` via a
  test stub on `ExecCtx`): happy path → `Output`; depth guard →
  `DepthExceeded`; missing spawner → error; bad args → `InvalidArgs`;
  `UnknownRole` mapping.
- **Integration test** (`crates/cogito-core/tests/` or
  `cogito-cli`): parent session invokes `delegate`, child session
  (`MockModelGateway` scripted) runs to completion, parent receives the
  final text; assert (a) returned text, (b) a separate child JSONL exists,
  (c) child `SessionMeta.parent_session_id` / `parent_call_id` /
  `subagent_depth == 1` are set, (d) the parent log contains no child
  internal events. — **This also discharges the roadmap's "child runs to
  completion, parent receives final text" acceptance item.**
- **Depth E2E**: a strategy whose `allowed_tools` includes `delegate`,
  delegating recursively, errors at `max_depth` rather than looping.
- **Resume-chaos**: a `delegate_then_text` scenario is **optional** for
  v0.2 (the child is in-process and not independently resumed). At
  minimum assert the documented property: crash mid-delegate → on resume
  the parent re-dispatches `delegate`. A full `subagent_*` chaos scenario
  is a v0.3 item (it needs the event tree). Note this explicitly so the
  gap is intentional, not silent.

## Out of scope / explicitly deferred

- Async lifecycle, event tree, `handed_tools`, per-role gate, tenant
  inheritance — all v0.3+ (ADR-0011 §"v0.3 amendment").
- Orphan child-session GC — consumer/SaaS concern.
- Persisting the observability bridge — bridge is broadcast-only by
  design.

## Task checklist (for the implementation plan)

1. `cogito-protocol`: `subagent.rs` (`BrainSpawner`, `DelegateRequest`,
   `SpawnError`); `ExecCtx` fields + doc fix; `StreamEvent` field;
   `SessionMeta` fields; serde tests; `lib.rs` re-exports.
2. Regenerate `docs/schemas/conversation-event-v1.json`; re-pin the CI
   drift gate.
3. `cogito-core::runtime::subagent::DelegateToolProvider` + unit tests.
4. `Runtime: impl BrainSpawner` (`run_to_completion`) + internal
   child-open path (strategy + meta override).
5. `RuntimeBuilder::strategy_registry`; per-turn `ExecCtx` population
   (`brain_spawner`, `subagent_depth`); thread session depth from
   `SessionMeta`.
6. Live observability bridge in `run_to_completion` (forward child
   `StreamEvent`s to parent broadcast tagged `subagent_call_id`).
7. `cogito-cli chat`: register `delegate`, pass strategy registry, read
   `max_subagent_depth`.
8. Integration test (acceptance) + depth E2E + protocol/unit tests.
9. Docs: ADR-0011 (done); update `ARCHITECTURE.md` §"Subagent layer" and
   the `ExecCtx`/`SessionMeta` trait-contract rows; `docs/components`
   touch for H08 (delegate is `AlwaysSync`); `ROADMAP.md` Sprint 11
   checklist; `CHANGELOG.md` (public-API additions).
10. `make fmt && make fix CRATE=… && make test CRATE=…` green.

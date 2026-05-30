# ADR-0028: Per-session provider injection (`SessionSpec`)

## Status

Accepted (v0.2 Sprint 12, 2026-05-30).

Pulls forward one slice of the v0.4 SaaS-ready theme (see ROADMAP
"v0.4 SaaS-ready" and the Sprint 12 deviation note). Motivated by a
concrete consumer requirement: run cogito as a multi-tenant API server
behind one process, where each user request gets a different tool /
skill surface, and that surface can change mid-session.

## Context

Today every provider trait object — `ToolProvider`, `SkillProvider`,
`StrategyRegistry`, `ModelGateway`, `JobManager` — is fixed on the
`Runtime` at build time and shared by every session the Runtime opens
(`Runtime` struct in `runtime/builder.rs`). `open_session(id, mode)`
takes no per-session parameters; `open_inner` clones the Runtime's
global Arcs into the per-session `SessionDeps` / `SessionState`.

A SaaS API server cannot live with this:

- One process serves many tenants/users. The consumer explicitly does
  **not** want to manage a separate `Runtime` instance per tenant.
- Different users run different tasks → different tool surfaces (e.g.,
  tenant A has MCP server X attached, tenant B does not).
- The surface must be **mutable during a live session**: the clearest
  case is a user issuing a command mid-session that adds an MCP server
  or a skill, which must take effect on the next turn.
- On resume the active surface may **differ** from the one the session
  was first opened with (because of accumulated mid-session changes).

The good news, established by tracing the provider flow: the Brain
(H01–H11) already reads providers from a **per-turn** `TurnDeps` that is
rebuilt on every turn from per-session state. Nothing in the harness
assumes a global provider origin. The only thing pinning providers to
"global" is the `open_session` API boundary and the clone sites in
`open_inner`. This makes per-session injection a concentrated,
Brain-invisible change.

## Decision

### 1. `SessionSpec` — per-session provider overrides

A new value type carries optional per-session overrides; an absent
field falls back to the Runtime's global default.

```rust
// cogito-core::runtime
pub struct SessionSpec {
    pub tools:     Option<Arc<dyn ToolProvider>>,
    pub skills:    Option<Arc<dyn SkillProvider>>,
    pub strategy:  Option<HarnessStrategy>,   // reuses open_inner's existing per-session strategy slot
    pub tenant_id: Option<String>,            // stamped into SessionMeta
    pub user_id:   Option<String>,            // stamped into SessionMeta
}
```

The Runtime keeps its build-time providers as the **base / fallback**.
A spec field, when `Some`, replaces the corresponding provider **for
that session only**.

### 2. `open_session_with(id, mode, spec)`

```rust
impl Runtime {
    pub async fn open_session_with(
        self: &Arc<Self>, id: SessionId, mode: OpenMode, spec: SessionSpec,
    ) -> Result<SessionHandle, RuntimeError>;
}
```

The existing `open_session(id, mode)` is retained and defined as
`open_session_with(id, mode, SessionSpec::default())` (all-`None`), so
every current caller keeps working unchanged.

### 3. Providers are mutable session state, not open-time constants

Per-session providers live in the **mutable** `SessionState` owned by
the session actor (today `tools` lives in the immutable `SessionDeps`;
it moves into `SessionState` alongside `skills`). A new mailbox command
swaps them at runtime:

```rust
// SessionCommand gains a variant; SessionHandle gains a method
impl SessionHandle {
    pub async fn update_session(&self, spec: SessionSpec) -> Result<(), RuntimeError>;
}
```

**Effect timing: next turn boundary.** Because `TurnDeps` is rebuilt
per turn from `SessionState`, a swapped Arc is picked up by the next
turn automatically. Swapping mid-turn is forbidden — the tool surface
(H05) and the model call for the in-flight turn are already committed to
the previous tool list. If a turn is in flight when `update_session`
arrives, the swap is applied before the next turn drains (same single-
slot discipline as mid-pause user input). In an interactive REPL the
command naturally arrives between turns, so the change feels immediate.

### 4. Composition belongs to the caller; core swaps whole Arcs

The core performs **no incremental merge**. "Add an MCP server" means
the caller (CLI/TUI surface, or the consumer's API server) rebuilds a
complete `ToolProvider` — e.g. `CompositeToolProvider::new([builtin,
previous_plugins, new_mcp])` — and hands the finished Arc to
`update_session`. The core just replaces `SessionState.tools`. This
keeps the Runtime free of composition policy and puts assembly where
tenant context is known.

### 5. Resume: the caller is the source of truth for the active surface

Provider **identity is not persisted** by the core, and is not
reconstructed from the event log. Rationale: a provider is *code*, not
state — it cannot be rebuilt by replaying events, and the inviolable
"state rebuilds from the log" rule (AGENTS.md) is about conversation
state, not injected dependencies.

Therefore on resume the caller supplies the **current** `SessionSpec`
via `open_session_with(id, Resume, spec)`. The caller already holds the
tenant→surface mapping (the REPL processed the `/add-*` commands; the
API server tracks per-session attachments; `SessionMeta.tenant_id` is
persisted and readable to help). The spec supplied on resume may equal
or differ from the open-time spec — the core makes no assumption either
way.

Optional, diagnostics-only: the active provider identity (MCP server
names, skill plugin ids) MAY be stamped into `SessionMeta.extra` or a
lightweight event for observability. v0.2 does **not** rely on it for
reconstruction. Fully self-describing, caller-agnostic multi-replica
resume is deferred to v0.4 (relates to ADR-0014 TenantContext).

### 6. Brain unchanged

H01–H11 are untouched. They already read `deps.tools` / `state.skills`
through the per-turn `TurnDeps`; `ExecCtx` carries context, not
providers. The change is entirely in the runtime layer's API boundary
and session-actor state.

### 7. Change surface (four sites)

1. `Runtime::open_session_with` — new public entry; `open_session`
   delegates to it.
2. `open_inner` — build `SessionDeps` / `SessionState` from the spec
   (spec field `Some` wins, else clone Runtime default); stamp
   `tenant_id` / `user_id` into `SessionMeta`.
3. Move the mutable providers (`tools`, plus the already-present
   `skills`, optionally `strategy`) into `SessionState`.
4. `SessionCommand::UpdateSession` branch in the session loop that
   swaps the Arcs.

## Consequences

**Easier**:
- One process, many tenants: per-session tool/skill surfaces without
  per-tenant `Runtime` instances.
- Live mutation: attach an MCP server or skill mid-session, effective
  next turn, no session restart.
- Natural foundation for v0.4 TenantContext (`tenant_id` / `user_id`
  now flow into `SessionMeta`).

**Harder**:
- Resume correctness now depends on the caller re-supplying the right
  spec; this contract must be documented for consumers.
- `SessionState` gains mutable provider slots → the resume-chaos suite
  must cover a mid-session surface change across crash boundaries.

**Given up**:
- Self-describing resume (any replica rebuilds the surface from the log
  alone) — deferred to v0.4.
- Mid-turn provider changes — explicitly out; next-turn boundary only.
- Incremental provider merge in core — caller composes instead.

## Resume-chaos scenario (new)

`session_spec_mutated_then_resume`: open with spec A → run a turn →
`update_session` to spec B (adds one MCP server) → run a turn that uses
the new tool → inject crash at each boundary → resume with
`open_session_with(id, Resume, B)` → assert all four oracles
(prefix-immutable / terminal-equivalent / tool-mapping-equivalent /
final-text-equivalent) hold.

## References

- ADR-0006 (Runtime + H01 execution model) — the model this extends
- ADR-0011 (subagent) — `open_inner`'s existing per-session `strategy`
  and `meta_override` slots that this generalizes
- ADR-0014 (TenantContext propagation, v0.4) — downstream consumer of
  the `tenant_id` / `user_id` introduced here
- ADR-0018 (MCP integration) — `CompositeToolProvider` composition the
  caller reuses
- ADR-0021 (plugin manifest + loader) — produces the per-session
  providers a SaaS server feeds into `SessionSpec`
- AGENTS.md §"Inviolable design principles" — state-from-log rule and
  its boundary (conversation state vs injected dependencies)

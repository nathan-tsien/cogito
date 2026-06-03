# ADR-0014: TenantContext propagation

## Status

Proposed (draft, v0.3/v0.4).

Draft for human ratification. This is the next slice of the v0.4
SaaS-ready theme after ADR-0028 (per-session provider injection), which
introduced `tenant_id` / `user_id` into `SessionMeta`. This ADR is about
runtime *propagation* of that identity to the tool / hook / store seam at
dispatch time; it is explicitly **not** about enforcement (ROADMAP
"What we explicitly do not do" — enforcement is the consumer's job).

## Context

ADR-0028 added `tenant_id` and `user_id` to `SessionSpec` and stamped them
into `SessionMeta` (`crates/cogito-protocol/src/session.rs`, fields
`tenant_id: Option<String>` / `user_id: Option<String>`). That gives us
**log-level identity**: the seq=0 `SessionStarted` payload records who the
session belongs to, written once at open in
`runtime::builder::open_inner` (`crates/cogito-core/src/runtime/builder.rs`
~line 222, only when `!session_exists`).

What `SessionMeta` does **not** give us is a *runtime handle*. When a tool
or hook executes mid-turn, it receives an `ExecCtx`
(`crates/cogito-protocol/src/exec_ctx.rs`), constructed per turn in
`runtime::session_loop::spawn_turn_driver` (~line 478). Today `ExecCtx`
carries `session_id`, `turn_id`, `call_id`, `deadline`, `cancel`,
`subagent_depth`, `brain_spawner`, `workspace`, `skill_roots` — but no
tenant identity. A tool that wants to scope an outbound call by tenant
(e.g. a consumer's MCP tool that must select a tenant-scoped credential or
namespace its store keys) has no in-hand value to read; it would have to
re-load `SessionMeta` from the store out of band. The same gap applies to
hooks (H09 policy gates) and to a `ConversationStore` that wants to
partition rows by tenant.

So the forces are:

- Identity already persists (ADR-0028) but is **log-only** — available
  after the fact by reading the event log, not at the point of dispatch.
- The Brain (H01-H11) may import only `cogito-protocol` (ADR-0004), so
  any propagated handle must be a protocol-layer value type, threaded
  through `ExecCtx` (which the Brain already constructs and clones to
  tools / hooks).
- Cross-process resume already works: a fresh `Runtime` has an empty
  registry, shares the store, and the caller re-supplies the
  `SessionSpec` (ADR-0028 §5). On resume `open_inner` does **not**
  re-derive `SessionMeta` (the `!session_exists` guard skips the stamp),
  so any runtime handle must be reconstructable from what is durably
  available.
- Enforcement (rejecting a cross-tenant access, rate limiting, quota) is
  the consumer's responsibility per the ROADMAP. Cogito propagates; it
  does not police.

## Decision

### 1. Add an optional `TenantContext` value type to `cogito-protocol`

A new protocol-layer value type, carried by `ExecCtx` as an `Option`:

```rust
// cogito-protocol
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TenantContext {
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
}
```

```rust
// added to ExecCtx (crates/cogito-protocol/src/exec_ctx.rs)
pub tenant: Option<Arc<TenantContext>>,
```

`Arc` so the per-dispatch clone of `ExecCtx` (Brain hands each tool / hook
a clone) stays cheap. `Option` so the common single-tenant / CLI path
pays nothing and `ExecCtx::open_ended` keeps a zero-config constructor
(`tenant: None`). The hand-written `Debug` impl gains one line
(`.field("tenant", &self.tenant)` — `TenantContext` is `Debug`, unlike
`brain_spawner`), and the "list every field above" maintenance note keeps
its promise.

### 2. What this adds beyond ADR-0028 `SessionMeta`

This is the crux and must not be muddled:

- `SessionMeta.tenant_id` / `.user_id` are **log-only identity**: written
  once at seq=0, read by replaying the event log. They answer "who did
  this session belong to?" after the fact.
- `ExecCtx.tenant` is a **runtime-propagated handle available at dispatch
  time**: a tool / hook / store impl reads it synchronously while it is
  executing, with no store round-trip. It answers "on whose behalf am I
  running *right now*?" in the hot path.

They are the same identity in two forms. `SessionMeta` is the durable
record of truth; `TenantContext` is the in-process delivery of that truth
to code that runs mid-turn.

### 3. Construction and the resume-reconstruction rule

`TenantContext` is **derived, never independently authored**:

- At session open, `open_inner` already has `spec.tenant_id` /
  `spec.user_id`. It builds the `TenantContext` from the same values it
  stamps into `SessionMeta`, stores it in `SessionState` (next to
  `subagent_depth`), and `spawn_turn_driver` clones it into each turn's
  `ExecCtx`.
- **On resume**, `open_inner` is entered with `session_exists == true` and
  skips the `SessionMeta` stamp. The `TenantContext` is therefore
  **rebuilt from the persisted `SessionMeta`** read back from the store at
  open, not from the resume-time `SessionSpec`. Rule: *the persisted
  seq=0 `SessionMeta` is the source of truth for tenant identity on
  resume.* This keeps identity stable across a crash even if the caller's
  resume-time spec omitted (or, by mistake, differed on) `tenant_id` /
  `user_id`. (Provider identity stays caller-supplied per ADR-0028 §5;
  tenant identity is conversation state and follows the state-from-log
  rule, AGENTS.md.)
- This makes `TenantContext` satisfy the inviolable "can this be rebuilt
  from the event log?" test: yes — from the seq=0 event.

A subagent child inherits the parent's `TenantContext` through the same
`open_inner` path that already propagates `subagent_depth` and parent
linkage (ADR-0011); a delegated child runs on behalf of the same tenant.

### 4. Scope is propagation only

Cogito does **no** validation, auth, or access control on `TenantContext`.
It is opaque pass-through, exactly as `SessionMeta` is today
(`session.rs` module docs: "Cogito performs no validation or auth").
Tools, hooks, and the consumer's store impl read it and decide policy.
Enforcement — rejecting cross-tenant access, quotas, rate limits — lives
in the consumer (praxis), which brings its own store persistence and
gateway routing. Per the KEY FACTS, cogito's own postgres store is not on
the consumer's critical path, so this ADR deliberately does not specify a
tenant-partitioning scheme for any cogito-owned store.

### 5. Brain unchanged in spirit

H01-H11 gain nothing to reason about: they construct `ExecCtx` (already
do) and clone it to tools / hooks (already do). The Brain never reads
`tenant` itself. The change is additive on a protocol value type plus two
runtime wiring sites (`open_inner` derive-and-store, `spawn_turn_driver`
clone-into-`ExecCtx`).

## Consequences

**Easier**:

- A tenant-aware tool / hook / store reads identity synchronously from
  `ExecCtx.tenant` in the dispatch hot path — no out-of-band
  `SessionMeta` reload.
- The consumer can key tenant-scoped credentials, namespaces, or store
  partitions at the exact point of use, on the way to multi-replica
  behind its own gateway.
- Subagents and resumed sessions carry the correct tenant automatically.

**Harder**:

- One more `ExecCtx` field to keep in sync (constructor + hand-written
  `Debug` + the maintenance note that enumerates fields).
- The resume-reconstruction rule (rebuild from persisted `SessionMeta`,
  not from resume-time spec) is a subtle contract that must be tested in
  the resume-chaos suite and documented for consumers, since it
  intentionally differs from the provider-identity rule of ADR-0028 §5.

**Given up**:

- Enforcement in core — explicitly out of scope; consumer owns it.
- A fixed, cogito-defined tenant-partitioning scheme for any cogito store
  — not specified here.

## Alternatives considered

- **Let tools reload `SessionMeta` from the store.** Rejected: a store
  round-trip on every tenant-sensitive dispatch is wasteful, and it
  couples tool code to the store seam the Brain is meant to hide.
- **Pass `tenant_id` as a bare `Option<String>` on `ExecCtx`.** Rejected:
  user identity is also needed, and a named struct leaves room for the
  shape question below without churning the `ExecCtx` signature again.
- **Stamp tenant onto every event payload.** Rejected: redundant with the
  seq=0 `SessionMeta` and bloats the log; identity is session-scoped, not
  per-event.

## Open questions

- **Shape: fixed `tenant_id` / `user_id` vs. arbitrary consumer labels.**
  This draft proposes the fixed two-field shape to mirror `SessionMeta`
  exactly. A consumer running a richer isolation model (org / project /
  region, or a free-form label map) might want `TenantContext` to carry an
  arbitrary `Map<String, String>` (paralleling `SessionMeta.extra`). Open
  for the human: keep the fixed shape (simplest, matches ADR-0028), or
  admit a labels map (more flexible, but invites the question of which
  labels resume must reconstruct and whether they belong in `extra`).
- **Does `TenantContext` need its own persisted home, or is reading it
  back off `SessionMeta` at open sufficient?** This draft assumes the
  latter (no new event, no schema bump). Confirm before implementation.
- **Version placement.** Drafted against the v0.4 SaaS-ready theme; could
  be pulled into v0.3 alongside the subagent work if a consumer needs
  tenant-scoped tools sooner. Human to sequence.

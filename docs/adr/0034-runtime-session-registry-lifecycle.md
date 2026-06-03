# ADR-0034: Runtime session-registry lifecycle (`close_session` / `get_session`)

## Status

Proposed (v0.4 SaaS-ready theme; not blocking v0.3).

Driven by a consumer reverse-requirement: praxis RR-7 (recorded in praxis
`docs/cogito-reverse-requirements.md`), tracked as cogito issue #55. praxis
consumes only the public `Runtime` / `Session` surface and will not reach
into cogito internals, so this is a request to surface an entry point, not a
patch against internals.

## Context

The `Runtime` keeps live sessions in an in-memory registry:

```rust
// runtime/builder.rs
sessions: DashMap<SessionId, SessionHandle>,
```

This registry is **insert-only**. Verified against `v0.2.0` (commit
`dd15168`) and re-verified on `main`:

- `open_inner` inserts on success (`builder.rs:355`); nothing ever removes.
- `open_inner` returns `RuntimeError::SessionAlreadyOpen` when
  `register && self.sessions.contains_key(&id)` (`builder.rs:140-141`).
- `SessionHandle::shutdown` (`handle.rs:217`) drains the mailbox and tears
  the actor down, but does **not** remove the registry entry. The handle's
  `Drop` even logs a leak warning if the last clone drops without
  `shutdown()` — evidence that "shut down" and "deregistered" are already
  two separate concerns that nothing reconciles.
- The public surface (`open_session`, `open_session_with`, builder setters)
  exposes **no `close_session` and no `get_session`**.

**Consequence:** within a single live `Runtime`, any id that was ever
`open_session`'d can never be opened again — `OpenMode::Resume` included.
Every reopen hits `SessionAlreadyOpen`.

v0.2's additions do not close this gap. `open_session_with` (ADR-0028) and
`OpenMode::Attach` address per-session provider injection and store-state
tolerance respectively — both about *store* state, neither about the
*in-memory* registry collision.

The plumbing for "do not register" already exists: `open_inner` carries an
internal `register: bool` (`builder.rs:127-136`), used today for ephemeral
subagent children that are deliberately kept out of the registry. So the
registry is already designed to be optional per-session; what is missing is
a public way to (a) look up a live entry and (b) remove one.

### Forces

- **Same-`Runtime` resume after handle eviction** is the whole point. Two
  consumer scenarios (both from praxis):
  1. *Idle-handle eviction → lazy re-resume.* Free memory for idle sessions,
     then `Resume` on the next request that touches them.
  2. *Multi-node bounce-back.* A session drifting back to a node whose
     `Runtime` previously opened it.
  Both hit `SessionAlreadyOpen` today.
- **Cross-process restart already works** — a *fresh* `Runtime` has an empty
  registry, so reopen succeeds. That is what praxis ships now (its Phase 1 /
  2a). This ADR is specifically about reopen within the *same* process.
- **Layer placement.** `get_session` / `close_session` are Runtime-layer
  surface on the `Runtime` struct. They touch the registry and drive the
  per-session actor — no Brain (H01–H11) involvement, no `cogito-protocol`
  trait change. ADR-0004 is not engaged.
- **Coupling shutdown to deregistration.** The `Drop` leak warning shows the
  hazard of letting the two drift: a remove-only API that frees the slot
  while the actor still runs would *invert* the current leak (slot gone,
  task alive) and is just as confusing.

## Decision

Add two public methods to `Runtime`. No `cogito-protocol` change; both are
pure Runtime surface.

### 1. `get_session` — live-handle lookup

```rust
impl Runtime {
    /// Return the live handle for `id` if a session is currently registered
    /// in this `Runtime`, else `None`. Cheap registry clone; does not touch
    /// the store and never opens or resumes. Returns `None` for an id that
    /// was never opened, or one already removed by `close_session`.
    pub fn get_session(&self, id: SessionId) -> Option<SessionHandle> {
        self.sessions.get(&id).map(|e| e.value().clone())
    }
}
```

Lets a consumer reuse a live handle instead of reopening — turning the
`SessionAlreadyOpen` error from a dead end into a "check first, reuse if
present" pattern.

### 2. `close_session` — deregister, driving shutdown itself

`close_session` **drives shutdown and then removes the entry**, rather than
remove-only. Rationale: the `Drop` warning proves teardown is easy to forget;
keeping the two coupled means a caller can never free the slot while leaking
the actor task.

```rust
impl Runtime {
    /// Shut the session's actor down (up to `deadline`) and remove it from
    /// the in-memory registry, so a later `open_session(id, Resume)` for the
    /// same id succeeds within this same `Runtime`.
    ///
    /// Idempotent: returns `Ok(None)` if `id` is not registered (never
    /// opened, or already closed), or if the actor had already exited.
    /// `Ok(Some(outcome))` carries the `ShutdownOutcome` of the drive.
    pub async fn close_session(
        &self,
        id: SessionId,
        deadline: Duration,
    ) -> Result<Option<ShutdownOutcome>, RuntimeError> {
        // Atomically claim the handle. Concurrent close_session callers race
        // here; exactly one wins the remove and drives shutdown, the rest see
        // `None`. Removing *before* awaiting shutdown also means get_session
        // returns `None` for the duration of teardown — consistent.
        let Some((_, handle)) = self.sessions.remove(&id) else {
            return Ok(None);
        };
        match handle.shutdown(deadline).await {
            Ok(outcome) => Ok(Some(outcome)),
            // Actor already gone / shutting down: the slot is removed, which
            // is all the caller needs. Absorb, don't surface.
            Err(SessionError::SessionClosed { .. })
            | Err(SessionError::ShuttingDown { .. }) => Ok(None),
        }
    }
}
```

Resolving the issue's three open questions:

- **Does `close_session` require the handle be shut down first?** No — it
  drives shutdown itself. The caller does not have to hold or shut down a
  handle beforehand; passing just the id is enough.
- **Idempotency?** Yes. The atomic `DashMap::remove` makes the registry the
  single source of truth: a missing entry → `Ok(None)`. Double-close,
  close-after-crash, and concurrent close all converge on `Ok(None)` for the
  losers.
- **In-flight turn?** Inherited from `SessionHandle::shutdown(deadline)`,
  unchanged: it waits up to `deadline` for the in-flight turn, force-cancels
  on overrun, and reports that via `ShutdownOutcome::Clean {
  in_flight_cancelled: Some(_) }`.

### Safe close → reopen ordering

`close_session(...).await` returns only after the actor has exited. A
consumer that wants to re-resume **must sequence** the two:

```rust
rt.close_session(id, deadline).await?;     // old actor has exited here
let h = rt.open_session(id, OpenMode::Resume).await?;
```

Awaiting `close_session` before `open_session` guarantees the old actor has
drained (no further store appends) before the resumed actor reads
`latest_seq` and continues at `latest_seq + 1`. Issuing `open_session`
concurrently with an un-awaited `close_session` for the same id is a caller
bug: two actors could briefly write the same log. We document the ordering
rather than add locking — the sequential form is the natural usage and
matches both praxis scenarios (eviction then later re-resume).

## Consequences

What becomes easier:

- praxis ADR-0011 **Phase 2b** (in-process idle reaper) and multi-node
  bounce-back unblock: evict an idle handle via `close_session`, re-`Resume`
  on the next request.
- `get_session` gives consumers a non-throwing way to check liveness and
  reuse, instead of catching `SessionAlreadyOpen`.
- Shutdown and deregistration stay coupled, so the existing `Drop` leak class
  ("handle dropped without shutdown") gains a clean, explicit counterpart.

What we give up / watch:

- The close → reopen ordering contract is a documented invariant, not a
  type-enforced one. A consumer that races them can double-write the log.
- `close_session` taking `&self` (not `&Arc<Self>`) is fine because it does
  not open; `get_session` likewise. If a future variant needs to *reopen*
  inside `close_session`, it would need `&Arc<Self>` like `open_session`.

## Alternatives considered

- **Remove-only `close_session(id)`**, caller shuts the handle down first.
  Rejected: frees the registry slot while the actor may still run, inverting
  the `Drop` leak; pushes ordering correctness onto every caller.
- **Auto-deregister on actor exit.** The per-session actor removes its own
  registry entry when its loop ends (via a weak back-reference to the
  `DashMap`). Elegant — `shutdown()` would then free the slot for free, and
  even an abnormal actor exit would self-clean. But it adds a Runtime
  back-reference into the actor and a teardown-ordering subtlety, and it
  still does not give consumers an explicit "evict this id now" verb, which
  is exactly what praxis asked for. Deferred as a possible later enhancement
  layered *under* `close_session`, not a replacement for it.
- **Per-tenant `Runtime` instances** (one registry each). Rejected upstream
  by ADR-0028's premise: the consumer explicitly does not want a `Runtime`
  per tenant.

## Open questions (to settle when scheduled into v0.4)

- Should `close_session` also call `ConversationStore::close(id)` to release
  per-session backend resources (file handle / connection slot), or is that
  already the actor's responsibility on shutdown? Confirm against the actor's
  shutdown path before implementing; if the actor does not close the store,
  `close_session` should, after the actor exits.
- Whether to expose a registry snapshot (`session_ids() -> Vec<SessionId>`)
  for an idle reaper to enumerate candidates. Not requested by RR-7; add only
  if a concrete consumer needs it.

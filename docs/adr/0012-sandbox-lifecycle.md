# ADR-0012: Sandbox lifecycle (lazy provisioning, pets-vs-cattle)

## Status

Proposed (draft) — **DEFERRED (2026-06-03), not scheduled.**

Deep production hardening, deliberately parked. Resolution: the execution
**seam is already in the right place** (`CommandExecutor` in
`cogito-protocol::command`; `DirectExecutor` in `cogito-sandbox`, selected by
the `build_executor(&SandboxConfig)` tagged factory), so a future
`SandboxedExecutor` slots in as a Hands-layer swap, invisible to the Brain and
to tools. We do **not** build the real sandbox (namespaces / seccomp / chroot /
cgroups or a remote executor) now.

**Trigger to build it:** cogito executes code reachable by an untrusted party —
a multi-tenant shared process running tenant-authored code, or a `bash`/exec
tool exposed to a model whose input is attacker-influenceable without a human
gate. Until then, the current posture is honest and documented: default
builtin tools are file ops confined to the `Workspace` single root
(ADR-0030/0031), `DirectExecutor` is explicitly "not a security boundary," and
arbitrary `sh -c` is opt-in (the `cogito-jobs` bash path + bash-running
skills), which the consumer chooses whether to expose.

**Pending input (sizes this ADR):** does the consumer (praxis) wire bash/exec
into the model's tool surface, and is the model's input reachable by untrusted
parties? That single answer sizes both this ADR and ADR-0013. Scope when
built stays strictly *lifecycle* (provisioning, reuse, teardown, resource
budgets); credential isolation is the separate ADR-0013.

## Context

cogito today ships exactly one execution backend: `DirectExecutor`
(`crates/cogito-sandbox/src/executor.rs`), selected by the
`build_executor(&SandboxConfig)` tagged factory. It runs `sh -c <command>`
on the host with no namespaces, seccomp, chroot, or cgroup limits, and by
default inherits the parent environment (`DirectConfig { root, inherit_env:
true }`). Its own module doc is explicit: "Not a security boundary." The
`SandboxConfig` enum has a single tag, `Direct`, and both the crate docs and
ADR-0027 flag the isolating / remote variants as reserved for "v0.4
(ADR-0012 / ADR-0013)". `SandboxError::Config` is a reserved variant that
"`Direct` never errors today" — it exists precisely so an isolating backend
can fail to provision.

There is no notion of a "sandbox instance" anywhere in the code. A
`DirectExecutor` is a cheap `Clone` struct holding a `DirectConfig`; there is
nothing to provision, reuse, or tear down. Per-call inputs are `CommandSpec
{ command, cwd, timeout, max_output_bytes }` plus an `ExecCtx` carrying
`session_id` / `turn_id` / `cancel` / `deadline` / `workspace` / etc. The
only resource controls that exist are per-call: a wall-clock `timeout` and an
output byte cap. There is no per-session memory or CPU budget, no aggregate
budget across a session's many `bash` calls, and no place to put one.

The adjacent workspace ADRs already gesture at this ADR. ADR-0031 §4 states
that the SaaS workspace root is "Backed by the sandbox FS
(`SandboxWorkspace`, v0.4 / ADR-0012)" with lifecycle "cattle /
lazy-provisioned (ADR-0012) — created on first file op, destroyed at session
end." ADR-0030 §5 places `SandboxWorkspace` in "the `cogito-sandbox` v0.4
redesign (Phase 3)." So a sandbox in cogito is really *two coupled
capabilities* over the same isolation primitive: a `CommandExecutor` (run a
process) and a `Workspace` (a confined file tree). This ADR's lifecycle
decisions must cover both, because for an isolating backend they are the same
underlying instance (a tenant volume + the processes that run against it).

**Why this is deferrable.** The decisions below only have teeth when cogito
is the trust boundary for code it did not write. The current and near-term
consumer story does not require that:

- praxis brings its own deployment topology and is expected to run cogito's
  process execution either disabled or behind its own infrastructure-level
  isolation (containers / VMs / per-tenant pods it already operates). When
  the host is operator-trusted, `DirectExecutor` plus the existing per-call
  `timeout` is sufficient, and OS / container limits set by the operator
  already cap memory and CPU outside cogito's knowledge.
- ADR-0027 already routed *all* cogito-originated subprocess spawning that
  matters (the `bash` tool) through the `CommandExecutor` seam, and routed
  command/URL admission (blocking `rm -rf /`, SSRF) to H09 hooks rather than
  the sandbox. So the seam needed for isolation already exists and is
  unused-by-design; nothing is blocked by leaving this ADR in Proposed.

The trigger that makes this ADR non-deferrable is: **a single cogito process
that executes untrusted or tenant-supplied code (a SaaS REPL/agent where the
end user controls what `bash` runs).** At that point host execution is
unsafe, infra-level isolation is too coarse (one process, many tenants), and
cogito must own a per-session isolation instance with a lifecycle. Until
then this stays a design reservation.

## Decision

All of the following describe behavior of a **future non-`Direct`** backend
(call it `IsolatedExecutor` + `SandboxWorkspace`, both in the
`cogito-sandbox` v0.4 redesign). `DirectExecutor` is unaffected and remains
the default; choosing it means "lifecycle is the operator's problem, not
cogito's."

### 1. The unit of isolation is the session, lazily provisioned

A sandbox **instance** (an isolation context: namespace set / container / VM
/ tenant volume — the mechanism is the backend's choice and out of scope
here) is scoped to **one session**, matching the per-session, ephemeral
workspace granularity already decided in ADR-0031 §1. One session = one
isolation instance shared by that session's `CommandExecutor` calls and its
`SandboxWorkspace`.

Provisioning is **lazy: on first need, not at session open.** A session that
opens, runs a few model turns, and closes without ever invoking `bash` or a
file tool must provision **no** sandbox. The trigger is the first
`CommandExecutor::run` or first `Workspace` operation for that session.
Rationale:

- Most turns never touch the filesystem or shell; eager provisioning at
  `open_session` would pay container/VM startup cost on every session,
  including read-only Q&A sessions.
- It keeps `open_session` / `open_session_with` cheap and synchronous-feeling
  and avoids coupling session admission to sandbox-backend health.
- It composes with resume: a resumed session re-provisions lazily on the
  next exec, so a fresh Runtime / fresh replica needs no special handling
  (consistent with ADR-0028's "providers are code, re-supplied by the
  caller" stance).

### 2. Pets-vs-cattle: instances are cattle, fresh-per-session

Sandbox instances are **cattle**: anonymous, disposable, never repaired,
never reused across sessions. A new session gets a **fresh** instance; a
session never inherits another session's instance. Concretely:

- No cross-session pooling of *stateful* instances. (A backend MAY keep a
  warm pool of *blank* pre-initialized instances as a latency optimization,
  but a blank instance carries no prior session's filesystem or process
  state — it is functionally fresh. Pooling is an implementation detail
  invisible to the seam, not a lifecycle guarantee.)
- Within a single session the instance **is** reused across that session's
  turns and across that session's many `bash` calls — it must be, because
  ADR-0031 requires the workspace tree to persist between turns ("the model
  writes `gen.py` in one turn and runs it the next"). So: pet-like *within*
  a session (stable identity, accumulated state, the thing the model is
  "working in"), cattle *across* sessions.
- Cross-session durability remains **opt-in and caller-side**, exactly as
  ADR-0031 §1/§4 decided: seed the fresh instance's tree from object storage
  at provision time, persist it at teardown. cogito-core never makes an
  instance durable on its own.

### 3. Teardown is tied to session end, with a best-effort + reaper backstop

An instance is destroyed when its session ends (normal close, or
`Runtime::close_session` per ADR-0034). Because teardown is I/O against an
external isolation backend and can fail or be skipped by a crash, it cannot
rely solely on the close path:

- **Best-effort on close**: session shutdown drives instance teardown within
  the close deadline (ADR-0034's `close_session(id, deadline)`); failure to
  tear down is logged, not fatal to close.
- **Reaper backstop**: orphaned instances (process/replica crashed before
  teardown; session never formally closed) are reclaimed by an out-of-band
  reaper keyed on session id + a liveness/TTL marker. This is a backend /
  consumer responsibility, not Brain or Turn-Driver logic — consistent with
  the rule that the Brain sees only `dyn CommandExecutor` / `dyn Workspace`
  and knows nothing of instances. The reaper design itself is an open
  question (below); this ADR only mandates that teardown not depend on a
  clean shutdown.

### 4. Resource budgets: per-session caps, enforced at the backend, surfaced as structured errors

Two distinct controls, kept separate:

- **Per-call** (exists today, unchanged): `CommandSpec.timeout` (wall clock)
  and `max_output_bytes`. These stay per-call and backend-independent.
- **Per-session budget** (new, isolating-backend only): memory and CPU caps
  applied to the *whole* session instance, not per call — e.g. a memory
  ceiling (cgroup `memory.max`-style) and a CPU share/quota, plus optionally
  an aggregate wall-clock or disk-quota for the workspace tree. These are
  **construction-time policy on the backend config**, mirroring how `env` /
  `root` already live on `SandboxConfig` rather than on each `CommandSpec`
  (ADR-0027 alternative #2). The hook point is therefore the
  `SandboxConfig` non-`Direct` variant (new tag), carrying a
  `budget: ResourceBudget { memory, cpu, ... }`, consumed by `build_executor`
  when constructing the isolating backend and by the matching
  `SandboxWorkspace`.

Where the budget comes from per session: the caller composes it the same way
it composes per-session providers (ADR-0028 / ADR-0031 §2 — caller-side, from
`tenant_id` / `user_id`). The concrete plumbing — whether a per-session
budget rides on `SessionSpec` alongside `workspace`, or is baked into the
tenant-specific `Arc<dyn CommandExecutor>` the caller already injects — is an
open question; the latter requires **no protocol change** and is the
preferred default, keeping `SessionSpec` and `ExecCtx` untouched.

Enforcement is the **backend's** job (cgroups / VM limits / container
limits); cogito does not meter syscalls. When a session exceeds a budget, the
backend surfaces it as a structured signal on the existing seam, not a panic:
a budget-exceeded process maps to a `CommandOutcome` (non-zero exit, or a new
`timed_out`-like flag if we choose to distinguish OOM-kill from normal exit)
or, for provisioning-time refusal, to `CommandError::Spawn` /
`SandboxError::Config`. This stays inside the ADR-0027 contract that
"non-zero exit and timeout are outcomes, not errors."

### 5. Brain and protocol are unchanged

H01–H11 see only `dyn CommandExecutor` and `dyn Workspace`. None of
provisioning, reuse, teardown, or budgets is visible to the Brain, the Turn
Driver, or the event log. `CommandSpec` / `CommandOutcome` / `ExecCtx` need
no new fields for the preferred design (budget baked into the injected
backend). Adding an isolating backend is a `cogito-sandbox`-internal change
plus a new `SandboxConfig` tag — the seam from ADR-0027 / ADR-0030 was built
for exactly this and absorbs it additively, with no `SCHEMA_VERSION` bump
(the executor and workspace are runtime-only, never serialized).

## Consequences

**Easier**:
- A SaaS consumer running untrusted code gets a per-session isolation unit
  with a defined lifecycle and a place to set memory/CPU caps, without any
  Brain change — the seam was reserved for this.
- Lazy provisioning means read-only / no-exec sessions pay zero sandbox cost,
  and resume needs no instance-reconstruction logic.
- Lifecycle aligns 1:1 with the already-decided per-session ephemeral
  workspace (ADR-0031), so a sandbox instance and its workspace tree are the
  same lifetime — no second lifecycle model to reconcile.

**Harder**:
- Teardown correctness now depends on a reaper for the crash path; an
  orphaned-instance leak is a real failure mode an operator must monitor.
- Backends must implement real OS-level budget enforcement (cgroups / VM
  limits); getting OOM-kill vs. normal-exit signalling right on the
  `CommandOutcome` contract is fiddly.
- A warm-pool optimization, if a backend wants it, must prove a pooled
  instance is genuinely blank — a subtle correctness/security obligation.

**Given up**:
- Cross-session instance reuse / persistent per-user "home" sandboxes as a
  default (opt-in caller-side seeding only, per ADR-0031).
- Per-call resource budgets beyond the existing `timeout` / `max_output_bytes`
  (budgets are per-session, construction-time).
- Any pretense that `DirectExecutor` participates in this: it has no
  lifecycle, and choosing it means the operator owns isolation and limits.

## Alternatives considered

1. **Provision eagerly at session open.** Rejected: pays container/VM
   startup on every session including read-only ones, couples session
   admission to sandbox-backend health, and conflicts with the lazy,
   first-file-op model ADR-0031 already wrote down.
2. **Pets across sessions (reuse a per-user instance).** Rejected as the
   default: defeats the isolation guarantee (one tenant's leftover state /
   processes leak into the next session) and turns teardown into stateful
   repair. Durability is provided instead by opt-in object-storage seeding
   (ADR-0031), which keeps instances cattle.
3. **Per-call resource budgets on `CommandSpec`.** Rejected: a session is
   many `bash` calls; the meaningful cap is aggregate memory/CPU for the
   whole instance, which is construction-time policy on the backend, not
   per-call data — same reasoning ADR-0027 used to keep `env` / `root` off
   `CommandSpec`.
4. **Meter resources inside cogito (count bytes / syscalls / time).**
   Rejected: cogito is not a kernel; real enforcement belongs to cgroups /
   VM / container limits the backend configures. cogito only surfaces the
   resulting signal as a structured `CommandOutcome` / error.
5. **Fold this into ADR-0013 (credentials).** Rejected: lifecycle (when an
   instance exists, who reuses it, when it dies, how big it may get) is
   orthogonal to *what secrets that instance can see*. Keeping them separate
   keeps each decision auditable on its own.

## Open questions

1. Reaper design: where does the liveness/TTL marker for orphaned instances
   live (the `ConversationStore`? a sandbox-backend-native registry?), and
   who runs the reaper (the consumer, a cogito background task, or the
   backend itself)? This ADR mandates a backstop but does not pick one.
2. Budget plumbing: bake the per-session `ResourceBudget` into the
   caller-injected `Arc<dyn CommandExecutor>` (no protocol change, preferred)
   vs. add a `budget` field to `SessionSpec` / `ExecCtx`. Decide when a
   concrete isolating backend lands.
3. OOM-kill signalling: does exceeding the memory cap surface as a plain
   non-zero `CommandOutcome`, or do we add an explicit flag (like the
   existing `timed_out`) to distinguish kernel-OOM-kill from a normal
   non-zero exit? Affects the `CommandOutcome` contract.
4. Isolation mechanism itself (namespaces vs. containers vs. microVMs) and
   the exact `SandboxConfig` non-`Direct` tag shape — deliberately left to
   the v0.4 `cogito-sandbox` redesign; this ADR fixes the lifecycle, not the
   mechanism.
5. Whether `SandboxWorkspace` and `IsolatedExecutor` are forced to share one
   instance handle (they should, to keep file writes visible to subsequent
   `bash` runs per ADR-0031 §5) or are merely coordinated — confirm when both
   are built together.

## References

- ADR-0027 — `CommandExecutor` seam (the seam this extends; reserves
  ADR-0012/0013 for isolation + limits; admission is H09's job, not the
  sandbox's)
- ADR-0030 — `Workspace` seam (`SandboxWorkspace` placed in the v0.4
  cogito-sandbox redesign)
- ADR-0031 — workspace provisioning + scoping (per-session ephemeral,
  cattle / lazy-provisioned, exec-cwd unification — the lifecycle model this
  ADR generalizes to the isolation instance)
- ADR-0028 — per-session provider injection (caller-side composition of the
  tenant-specific executor/workspace this lifecycle hangs off)
- ADR-0034 — Runtime session-registry lifecycle (`close_session(id, deadline)`
  is the teardown trigger)
- ADR-0013 (this batch) — credential isolation (the deliberately-separate
  "what can the instance see" decision)
- Code: `crates/cogito-sandbox/src/{executor.rs,config.rs,lib.rs}`
  (`DirectExecutor`, the single `Direct` tag, `build_executor`, the reserved
  `SandboxError::Config`), `crates/cogito-protocol/src/command.rs`
  (`CommandSpec` / `CommandOutcome` / `CommandError`),
  `crates/cogito-protocol/src/exec_ctx.rs` (`ExecCtx` fields)

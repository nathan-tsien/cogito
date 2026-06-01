# ADR-0031: Workspace provisioning and scoping

## Status

Accepted (2026-06-01). Decides the provisioning / scoping / lifecycle model
for the `Workspace` seam (ADR-0030), for both the Local (TUI/CLI) and SaaS
multi-user profiles, and resolves the workspace-root ↔ command-execution-cwd
relationship that ADR-0030 left open.

## Context

ADR-0030 locked the `Workspace` *seam* — a rooted, confined working tree
injected as `dyn Workspace` — but deliberately deferred four operational
questions: who picks the root, what lifetime it has, how multi-user isolation
works, and whether the workspace root relates to the command-execution cwd.

These are not Brain concerns (the Brain only sees `dyn Workspace`), but they
determine how the Surface/consumer wires the seam, and they differ sharply
between a single-user local TUI and a multi-tenant SaaS process. This ADR
decides them.

## Decision

### 1. Granularity and lifetime: per-session, ephemeral

The workspace unit is the **session**. One `Arc<dyn Workspace>` per session,
stable across turns within the session, torn down at session end.

- Per-session (not per-turn) because skill work spans turns: the model writes
  `gen.py` in one turn and runs it the next; the tree must persist between
  them.
- Per-session (not per-user) because session-granular roots give isolation
  between concurrent sessions for free (the seam's root confinement applies).
- Ephemeral by default — no per-user durable "home" in the baseline.
  Cross-session durability is an opt-in the consumer layers later (seed from /
  persist to object storage), not a seam default.

### 2. Injection: `SessionSpec.workspace` (caller-side, ADR-0028)

Add `workspace: Option<Arc<dyn Workspace>>` to `SessionSpec`, threaded to
`ExecCtx.workspace` per turn — exactly the mechanism `tools` / `skills` /
`strategy` already use (ADR-0028). `None` falls back to the Runtime default.

Multi-tenancy stays **caller-side**: cogito-core never constructs a
tenant-rooted workspace. The consumer composes it from `tenant_id` / `user_id`
(already on `SessionSpec`) and injects it via `open_session_with`. The Brain
stays tenancy-agnostic — it cannot tell "a tenant is active" from "a workspace
is present", same as ADR-0028's stance for the other providers.

### 3. Local profile (TUI / CLI): root = project cwd

Default root = the directory where `cogito chat` was launched — the user's
project. Files the agent writes are visible in the project tree, and this
matches the de-facto process-cwd that `bash` already runs in.

- Configurable via `--workspace <dir>` / `cogito.toml` (e.g. point at a
  dedicated scratch dir if the user prefers isolation).
- Single trust domain: concurrent local sessions MAY share the tree.
  Collisions are a hygiene concern, not a security one — the user is one
  person on one machine.

### 4. SaaS profile: per-tenant/session root, sandbox-backed

Root = `<tenant_volume>/<tenant_id>/<session_id>/` (or a sandbox filesystem
that maps there). The consumer provisions it per session.

- Backed by the sandbox FS (`SandboxWorkspace`, v0.4 / ADR-0012) so isolation
  is real (not just lexical) and the same root is the sandboxed exec cwd.
- Lifecycle: cattle / lazy-provisioned (ADR-0012) — created on first file op,
  destroyed at session end. Durability is opt-in via object storage
  (v0.4 `cogito-storage-s3`): seed the session tree from, and persist it to,
  a per-user prefix.
- Isolation hardening — canonicalize + deny symlinks escaping the root
  (ADR-0030 open question Q4) — is a **hard precondition** before
  tenant-adjacent roots share a filesystem. The Local profile's lexical guard
  is insufficient here.

### 5. Exec cwd unification

The **session workspace root is the default cwd** for `bash` /
`CommandExecutor` when the call gives no explicit cwd.

- ADR-0030 left the workspace-root ↔ exec-cwd relationship open. An earlier
  inclination was to keep them separate, on the worry that a turn may activate
  multiple skills with no single "skill cwd". That worry was about *per-skill*
  directories; the *session* workspace root is stable and unique, so unifying
  on it is unambiguous and is the right call: it makes "write a file → run a
  script → read its output" self-consistent across the file tools and the
  shell.
- Skill bundled files (ADR-0029) are materialized as subdirectories under the
  session root (Phase 2/3), so they are reachable from the same cwd.
- Mechanism: the session workspace root is supplied as the executor's base
  cwd; `bash`'s `cwd` argument resolves relative to it. Absolute cwd args stay
  allowed under the Local profile and confined under SaaS.

## Consequences

**Easier**:
- One coherent "where am I working" per session, shared by the file tools and
  the shell — no path-alignment guesswork by the model.
- SaaS isolation falls out of per-session roots + sandbox backing; tenancy
  stays caller-side, so the Brain is unchanged.
- The Local default (project cwd) matches user expectation for "work on my
  project" and unifies with the existing bash cwd.

**Harder**:
- `SessionSpec` and `ExecCtx` gain a `workspace` field (additive; every
  construction site sets `None`, as with `brain_spawner`).
- The execution layer must learn the session workspace root (a wiring change
  where `bash` / the `CommandExecutor` are constructed or where `ExecCtx` is
  built) to honor the unified cwd.
- ADR-0030's lexical confinement must be hardened (symlinks / canonicalize)
  before the SaaS profile is enabled.

**Given up**:
- A per-user durable home as a default (opt-in only).
- Cross-session file persistence locally beyond whatever lives in the project
  cwd.

## References

- ADR-0030 — `Workspace` seam (this ADR answers its deferred provisioning
  questions and resolves the workspace-root ↔ exec-cwd relationship)
- ADR-0028 — per-session provider injection (`SessionSpec`, caller-side
  composition — the mechanism `workspace` reuses)
- ADR-0012 (planned, v0.4) — sandbox lifecycle, lazy provisioning, pets-vs-cattle
- ADR-0029 — skill bundled-file path exposure / materialization under the root
- Complete-skill-support design §3–§4:
  `docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`

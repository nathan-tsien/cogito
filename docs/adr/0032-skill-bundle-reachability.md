# ADR-0032: Skill-bundle reachability via a read-only skill-root scope

## Status

Proposed (2026-06-02). Opens Phase 2 of the complete-skill-support path
(`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md` §5.2/§6).
Builds on ADR-0029 (bundled-file path exposure), ADR-0030 (`Workspace` seam),
ADR-0031 (workspace provisioning + exec-cwd unification), and ADR-0028
(per-session provider injection). Resolves the reachability question ADR-0030
flagged as a "Phase-2/3 concern".

> Supersedes this ADR's own first draft, which proposed **copying** bundles
> into the workspace ("materialization"). Investigation of the reference
> implementations (see Context) showed neither copies; this revision adopts a
> read-only scope instead. "Materialization" (physical placement) survives
> only as the **SaaS** mechanism, deferred to Phase 3.

## Context

A Skill is a directory: `SKILL.md` plus bundled `scripts/`, `references/`,
`assets/`, possibly nested several levels deep. ADR-0029 surfaces the skill's
**absolute host directory** in the `<skill ... root="...">` header.

Two Phase-1 changes broke reachability of the *bundled* files:

- **`read_file` (and `grep`/`glob`/`list_dir`/`edit`) became workspace-confined**
  (PR #41): absolute paths and `..`-escapes are rejected. A bundle living
  outside the workspace root is no longer readable by these tools — a
  regression against ADR-0029's premise that "the model reads bundled files
  via `read_file`/`bash`".
- **The SaaS sandbox has an isolated filesystem**: it cannot see the host's
  skill directory at all, so an absolute host path is meaningless there.

`bash` is unaffected in the **Local** profile (its `DirectExecutor` is not
confined and reaches absolute host paths), but is confined in SaaS.

**How the reference implementations handle multi-level bundles.** Both Claude
Code and OpenAI Codex use **progressive disclosure + read-in-place**, and
**neither copies/materializes** the bundle:

- Only each skill's `name`/`description` (and, in Codex, its path) is loaded up
  front. The `SKILL.md` body loads on activation; nested files load on demand.
- **Claude Code** *injects* the `SKILL.md` body into context on trigger, and the
  model reaches nested files **in place** via the `Read`/`Bash`/`Glob` tools,
  resolving relative paths against the skill dir (`${CLAUDE_SKILL_DIR}`).
- **Codex** is more tool-driven: per a published teardown, the model reportedly
  reads the `SKILL.md` and nested files via its file-read tool from the exposed
  path. Its local sandbox reaches out-of-workspace skill roots via
  `sandbox_workspace_write.writable_roots` / a "runtime extra skill roots" API
  — i.e. an **allowed-roots set**, not a copy.

**Where cogito already sits.** cogito follows the Claude Code split: the
`SkillRegistry` reads `SKILL.md` once at scan time into `SkillRecord.body`, and
`SkillInjector` injects that body into the system-prompt suffix. **The body
never goes through `read_file`.** Only the *nested* bundled files
(`scripts/`/`references/`/…) are reached on demand via `read_file`/`bash`.

So the gap to close is narrow and specific: let the read-class file tools reach
the **nested bundled files** of registered skills, in place, without copying —
exactly the Codex "extra skill roots" shape, layered on top of cogito's
(stricter than CC/Codex) Phase-1 confinement.

## Decision

### 1. A read-only skill-root scope, injected per session (zero Brain delta)

Carry the registered skills' on-disk directories as a read-only set on
`ExecCtx`, alongside `workspace`:

```rust
/// Read-only roots the read-class file tools may resolve into, in addition to
/// the writable `workspace` (ADR-0032). Each is an absolute skill directory.
pub skill_roots: Vec<PathBuf>,   // empty when no skills / no bundles
```

It is composed **caller-side per session** (ADR-0028), exactly like
`workspace` (ADR-0031): the Surface enumerates the registered skills'
directories via a new read-only `SkillProvider::skill_roots() -> Vec<PathBuf>`
accessor (the registry already holds each `SkillRecord.root`) and threads the
set into `ExecCtx` per turn (session-stable). H01–H11 and the injector are
unchanged; `SkillProvider` gains only that accessor; the model only ever sees
tool results. **The `Workspace` trait and its single-root confinement
(ADR-0030) are untouched** — the scope lives *beside* the workspace, not inside
it (no multi-root `Workspace`).

### 2. Read-class tools resolve into the scope; nothing is copied

`read_file` and `list_dir` accept a path whose **lexically resolved** absolute
form falls within a registered `skill_roots` entry — the same lexical,
non-`canonicalize` discipline `LocalWorkspace` already uses (ADR-0030 Q4) — as a
**read-only** access; otherwise they keep the Phase-1 workspace-confined
behavior. Such a read is serviced **directly** (host `tokio::fs` in the Local
profile), **bypassing the `Workspace` seam** — `LocalWorkspace` cannot serve it
because the path is outside its root (see §6 for the SaaS counterpart). `bash`
is unchanged (Local already reaches absolute roots). The bundle is **read in
place** — matching Claude Code / Codex.

Because resolution is lexical, a symlink *inside* a bundle that points outside
it is not blocked in v0.2; skill dirs are operator/loader-authored (trusted)
until Phase 3, when `canonicalize`-based hardening lands (see Open questions).

The model addresses bundled files by the absolute root from the
`<skill root="...">` header (ADR-0029) plus the relative path from `SKILL.md`,
e.g. `read_file "<root>/scripts/html2pptx.py"`. Deep trees need no special
handling: `SKILL.md` is the index, and the model navigates with
`read_file`/`list_dir` (and `bash`), as in the reference tools.

### 3. Scope is the nested bundle only; body injection unchanged

The `SKILL.md` body stays **injected** by `SkillInjector` (the Claude Code
pattern cogito already uses) — the read-scope serves only the on-demand reads
of nested files. Moving the body to a model-read (Codex style) is out of scope.

### 4. Which tools get the scope, and which deliberately do not

- **In scope (read-only):** `read_file`, `list_dir`. (There is no `exists`
  builtin tool — existence is observed by listing/reading.)
- **Not in scope:** `write_file`, `edit` (bundles are read-only — you cannot
  mutate a skill's own files), and `grep`/`glob` (these search/enumerate the
  *working tree*; recursing them into skill roots is a later opt-in, not a v0.2
  need). Keeping the surface small bounds the trust widening below.

### 5. Granularity: all registered skills' roots, session-stable

Whitelist every **registered** skill's directory for the session (not only the
per-turn *activated* ones). They are operator/loader-controlled and read-only,
so the marginal risk over "activated only" is negligible, and a session-stable
set avoids per-turn churn in the injected `ExecCtx`.

### 6. SaaS: physical placement, and why the mechanism differs by profile

The two profiles reach bundles by **different mechanisms**, unified only at the
model level (`read_file "<root>/scripts/x.py"` works in both):

- **Local** uses the read-scope above: skill roots are host dirs *outside* the
  workspace, read **directly** (no copy), bypassing `Workspace`.
- **SaaS** cannot do that: the sandbox cannot see host dirs, and a host-direct
  read would breach tenant isolation. So the bundle must be **physically
  placed** (copy or read-only mount) into the tenant sandbox — at which point it
  lives *inside* the sandbox workspace and is read through the **sandbox
  `Workspace`** like any other workspace file; the Local host-direct branch is
  **not used**.

This ADR defines the **seam** (`ExecCtx.skill_roots`) and the **Local**
realization (zero copy). The **SaaS** realization — placement + roots resolving
inside the sandbox — is Phase 3 / ADR-0012 work.

## Consequences

**Easier**:
- Matches the reference implementations (read-in-place, progressive
  disclosure); no Local copy, no project-tree pollution.
- The `Workspace` seam, its contract suite, and the loader→registry→injector
  body-injection chain are all **untouched**.
- Restores `read_file` access to bundled files that Phase-1 confinement removed,
  cheaply and with a small, explicit trust widening.

**Harder**:
- **`read_file`'s contract widens**: "absolute paths rejected" becomes
  "rejected unless resolved within a registered read-only skill root." Its
  tests change; the SaaS safety claim becomes "cannot read outside workspace
  ∪ skill_roots" rather than "outside workspace".
- A new `ExecCtx` field plus a **second read backend** in `read_file` /
  `list_dir`: skill-root reads go **direct to the host fs** in Local (bypassing
  `Workspace`), while in SaaS they go through the sandbox `Workspace` after
  placement — so **the skill-root read mechanism differs by profile** (the
  model-visible behavior stays uniform). This is the price of keeping
  `Workspace` single-root rather than making it multi-root.
- **SaaS still needs physical placement** (Phase 3) — the seam is defined now,
  the SaaS body is scheduled.
- Symlink/canonicalize hardening of the scope is required before skill roots
  become tenant-controlled (Phase 3).

**Given up**:
- **Path uniformity across profiles** and the **resume-path improvement** that
  copying would have bought. Local keeps the absolute host `root=` (ADR-0029),
  so the persisted `SystemPromptInjected.suffix` still carries an absolute
  path and ADR-0029's v0.4 multi-replica "re-resolve at prompt-build" caveat
  **remains open** (a copy-to-workspace approach would have closed it). Accepted
  in exchange for matching the reference tools and not copying.

## Alternatives considered

1. **Copy/materialize the bundle into the workspace** (this ADR's first draft).
   Uniform workspace-relative paths, resume-friendly, file tools untouched —
   but duplicates files, writes a hidden tree into the Local project cwd, and
   diverges from how Claude Code / Codex actually work (neither copies).
   **Rejected for Local**; **retained as the SaaS placement mechanism**
   (Phase 3).
2. **Read-only mounts inside the `Workspace` trait** (multi-root Workspace).
   Tools untouched, but loosens the just-locked single-root `Workspace`
   contract and its contract suite. Rejected: a bigger disturbance to the
   Phase-1 seam than widening `read_file`.
3. **Symlink / bind-mount the bundle into the workspace at a uniform path.**
   Would give uniform paths and `bash` reach without copying, but a Local
   symlink escaping root is rejected by our own confinement guard (ADR-0030
   Q4) and bind-mounts need privileges. Rejected for Local v0.2; revisit as a
   SaaS placement option.
4. **SaaS-only; Local on absolute paths via `bash` alone (no read-scope).**
   Leaves Local `read_file` unable to read bundled files (the ADR-0029
   regression). Rejected — the read-scope restores `read_file` parity cheaply.

## Open questions

1. `grep`/`glob` into skill roots: excluded in v0.2 (search = working tree).
   Revisit if skills want searchable references.
2. Pseudo-XML `root="…"` attribute escaping (ADR-0029 TODO): still deferred to
   Phase 3, when skill roots become tenant-controlled.
3. Symlink/canonicalize hardening of `skill_roots` resolution: required before
   Phase 3.
4. SaaS physical-placement mechanism (copy vs read-only mount): Phase 3 /
   ADR-0012.

## References

- ADR-0029 — skill bundled-file path exposure (the absolute-root header the
  read-scope makes reachable again)
- ADR-0030 — `Workspace` seam (single-root confinement, left intact)
- ADR-0031 — workspace provisioning + exec-cwd unification (`bash` already
  reaches absolute roots in Local)
- ADR-0028 — per-session provider injection (caller-side composition reused for
  `skill_roots`)
- ADR-0023 — bundled-script execution (Position A: read/run scripts via
  `read_file`/`bash`; the read-scope is what makes those reads reachable)
- Reference implementations: Claude Code Agent Skills
  (progressive disclosure, `${CLAUDE_SKILL_DIR}`, read-in-place) and OpenAI
  Codex skills (file-read tool, `sandbox_workspace_write.writable_roots` /
  runtime extra skill roots — the read-scope precedent)
- Complete-skill-support design §4 (profiles), §5.2 (workspace + tools), §6
  (Phase 2 row)
- Skill machinery touched: `cogito-protocol/src/exec_ctx.rs` (new
  `skill_roots`); `cogito-protocol/src/skill.rs` + `cogito-skills/src/registry.rs`
  (new read-only `SkillProvider::skill_roots()` accessor);
  `cogito-tools/src/builtins/{read_file,list_dir}.rs` (read-scope branch).
  Unchanged: the injector (`cogito-context/src/injector/skill.rs`), the
  `Workspace` trait (`cogito-protocol/src/workspace.rs`), and the
  loader→registry→injector body-injection chain.

# Complete Skill Support — design and phased implementation path

Date: 2026-06-01
Status: Draft for review
Scope: Full lifecycle of agentskills.io Skills in cogito, from
instruction injection to bundled-script execution and artifact delivery,
under two deployment profiles — Local (CLI/TUI) and SaaS (multi-tenant,
in-process, behind a consumer gateway).

Related: ADR-0020 (skill loader), ADR-0021 (plugin loader), ADR-0023
(bundled-script execution, deferred), ADR-0027 (CommandExecutor seam),
ADR-0028 (per-session provider injection), ADR-0029 (bundled-file path
exposure, this series), and the planned v0.4 ADR-0012/0013/0014
(sandbox / credential isolation / TenantContext) + v0.5 ADR-0009
(StorageSystem).

---

## 1. Thesis — the extension mechanism is the deliverable

cogito is a Brain ("thinking only"): it decides, it never acts. Execution
and storage are Hands, injected as `dyn` trait objects at Runtime build
time (ADR-0004), and swappable per session (ADR-0028). The payoff for
Skills is the organizing principle of this whole design:

> A Skill is a **Brain-level contract** — "activate this skill" means
> "inject its instructions and give the model a handle to its bundled
> files and an environment to run them in." That contract is satisfied by
> **Hands-level capability seams** (file I/O, command execution, artifact
> storage, dependency runtime). The *same skill bundle* must produce the
> *same model-visible behavior* whether the injected Hands are the Local
> profile (host filesystem + `DirectExecutor`) or the SaaS profile
> (per-tenant isolated sandbox + object store).

So "complete Skill support" is **not** new Brain logic. The Brain (H01–H11)
and the skill bundle stay identical across profiles. The work is:

1. Complete the missing Hands seams (writable workspace, execution env,
   dependency runtime, artifact storage).
2. Route skill bundles through those seams.
3. Provide two concrete profiles behind the seams, and prove they are
   behaviorally equivalent.

This keeps cogito a library, not infrastructure: SaaS consumers plug their
own sandbox/storage behind the seams; cogito ships reference local impls.

### 1.1 What is actually being designed

The deliverable of this document is the **extension mechanism** — the
stable set of seams and the injection/composition rules through which a
Skill acquires capability. The concrete implementations (a real container
sandbox, an S3 backend, the file-mutation tools) are downstream work, each
its own sprint. What must be locked now is the *shape of the extension
points*, so that every later implementation slots in additively and the
Brain+Hands core is never disturbed.

Said plainly: **get the seams right; defer the bodies.**

### 1.2 The core that must not move

These are inviolable for the entire skill effort (they restate AGENTS.md /
ADR-0004 in this context):

- **Brain H01–H11 logic is frozen.** No skill-specific branch enters the
  Turn Driver, dispatcher, or any component. A skill is *data* (its
  instructions) plus *injected providers* (its capability). The Brain
  cannot tell "a skill is active" apart from "some providers are present."
- **The turn FSM and event-sourcing are untouched.** New facts (artifacts,
  activations) arrive only as *additive* event variants under ADR-0007
  (no `SCHEMA_VERSION` bump); state is still rebuildable from the log.
- **Layer boundaries hold.** `cogito-core::harness` imports only
  `cogito-protocol`. No new `use cogito_sandbox::*` / `cogito_tools::*` in
  the Brain. H01 stays the only coordinator; H02–H11 never call each other.
- **Existing seams change only additively.** `ToolProvider`,
  `CommandExecutor`, `SkillProvider`, `ModelGateway`, `ConversationStore`,
  `JobManager` evolve by new methods with defaults or new struct fields —
  never by signature breaks that ripple into the Brain.

If a proposed skill feature requires editing a Brain component or breaking
a seam signature, that is the signal the design is wrong — re-express it as
a new seam or an additive field.

### 1.3 The extension points (and the rule for adding more)

cogito already has the mechanism; this effort extends its seam *set*, not
its shape. The mechanism has three parts:

1. **Seam = a `cogito-protocol` trait.** Capability is a trait object the
   Brain sees as `dyn Trait` via `ExecCtx`. Today: `ToolProvider`,
   `ModelGateway`, `ConversationStore`, `JobManager`, `CommandExecutor`,
   `SkillProvider`, `HookProvider`, `BrainSpawner`.
2. **Implementation = a Hands crate.** A concrete impl lives in a Hands
   crate (`cogito-tools`, `cogito-sandbox`, …) and is wired in by the
   Runtime layer, never by the Brain.
3. **Composition + injection.** Providers are aggregated (the
   provider-aggregation pattern, `CompositeToolProvider`) and injected at
   Runtime build, and now *per session and mid-session* via `SessionSpec`
   / `update_session` (ADR-0028). This is exactly the mechanism by which a
   tenant's skill surface is swapped without touching the Brain.

Every new skill capability in this document is therefore classified as one
of: **(A)** a new additive seam (protocol trait), **(B)** an additive field
/ default method on an existing seam, or **(C)** a new Hands impl behind an
existing seam. **Brain delta is always zero.**

| New capability | Extension type | Seam | Brain delta |
|---|---|---|---|
| Skill root exposure | B — field on `SkillContent` | `SkillProvider` | none |
| Writable working tree | A — new trait `Workspace` | new | none (injected via `ExecCtx`) |
| File-mutation tools | C — new impls | `ToolProvider` | none |
| Bundled-script execution | C — uses existing | `CommandExecutor` | none |
| Dependency runtime | B — field on skill metadata | `SkillProvider` | none |
| Artifact storage | A — `StorageSystem` (v0.5) | new | none |
| Artifact fact | B — additive event variant | event log | none (recorder writes; ADR-0007) |
| Per-tenant isolation | B — `TenantContext` on `ExecCtx` | existing | none |
| Per-session skill/workspace/storage | B — `SessionSpec` fields | ADR-0028 | none |

The rule for any future skill capability not yet listed: express it as A,
B, or C with a "Brain delta: none" line, or it does not ship.

## 2. Why today's support is incomplete

PR #35 fixed `/skill` routing so the SKILL.md body is now injected. But a
skill like the upstream `pptx` skill (scripts + references + assets,
produces a binary `.pptx`) still cannot run. The gaps, by capability
pillar:

| Pillar | Need | Today | Gap |
|---|---|---|---|
| P1 Instructions | Inject SKILL.md; activate via sigil/slash | Works (ADR-0020, PR #35) | — |
| P2 Bundled files | Read skill's `scripts/`/`references/`/`assets/` | `read_file` exists, but the skill's own dir is never surfaced | Path exposure (ADR-0029); no write tool |
| P3 Execution | Run bundled/authored scripts with deps | `bash` exists (host `sh -c`) | No dep provisioning; no isolation for SaaS; execution policy deferred (ADR-0023) |
| P4 Artifacts | Produce + persist + deliver binary outputs | none | No StorageSystem (v0.5), no artifact event/delivery |

Cross-cutting gaps: no file-mutation tools (`write_file`/`edit`/`glob`/
`grep`); no working-tree seam (so file I/O can't be redirected for SaaS);
no per-tenant isolation/budgets for running untrusted skill scripts.

Current builtin tool inventory (for reference): `read_file`, `web_fetch`
(cogito-tools); `bash`, `sleep`, `run_tests` (cogito-jobs). Execution
底座 is `cogito-sandbox::DirectExecutor` = `sh -c` on the host, explicitly
"Not a security boundary" (ADR-0027).

## 3. Capability model and layer mapping

Everything new lands in the layer ADR-0004 dictates. Brain never imports a
Hand; new capability is always a `cogito-protocol` trait injected via
`ExecCtx`.

| Capability | New/changed contract (Protocol) | Impl (Hands) | Surface |
|---|---|---|---|
| Skill root exposure | `SkillContent.root: Option<PathBuf>` (ADR-0029) | discovery already has it | injector header |
| Writable working tree | **`Workspace` trait** (new) — rooted read/write/list/glob | `LocalWorkspace` (host); `SandboxWorkspace` (SaaS) | — |
| File-mutation tools | (use `Workspace`) | `write_file`/`edit`/`glob`/`grep`/`list_dir` in cogito-tools | TUI tool pane |
| Command execution | `CommandExecutor` (exists, ADR-0027) | `DirectExecutor` (host); sandboxed executor (SaaS, ADR-0012) | — |
| Dependency runtime | **no custom descriptor** (ADR-0033 — Skills standard has none) | local: agent self-heal via `bash`; SaaS: pre-baked runtime image | — |
| Artifact storage | `StorageSystem` (v0.5 ADR-0009) + **`ArtifactProduced` event** | `storage-local` (v0.5); `storage-s3` (v0.4) | TUI link / signed URL |
| Per-tenant isolation | `TenantContext` on `ExecCtx` (v0.4 ADR-0014) | sandbox + cred proxy (ADR-0012/0013) | consumer gateway |
| Per-session skill+workspace+storage | extend `SessionSpec` (ADR-0028) | caller-composed Arcs | server |

The single most load-bearing new seam is **`Workspace`**: a rooted,
sandboxable working tree distinct from `StorageSystem` (which is a
blob/URI store). Skills need a POSIX-ish scratch tree (write a `.py`, run
it, read its output); that is not the same abstraction as durable blob
storage. `read_file` migrates onto `Workspace` so all file I/O can be
redirected for SaaS.

## 4. The two execution profiles

Define a **Skill Execution Profile** = the bundle of injected Hands a
session runs with. Two reference profiles:

Workspace provisioning, scoping, and the exec-cwd relationship are decided in
ADR-0031: per-session ephemeral tree, injected via `SessionSpec.workspace`
(caller-side), and the session workspace root is the default exec cwd.

### Local profile (CLI / TUI / dev)
- Skill roots: discovered on host FS (`.cogito/skills/...`); `root` =
  real host path.
- `Workspace` = `LocalWorkspace` rooted at the **project cwd** (where
  `cogito chat` launched), configurable; one ephemeral tree per session
  (ADR-0031). Bundled files read in place.
- `CommandExecutor` = `DirectExecutor` (`sh -c` on host), default cwd = the
  session workspace root (ADR-0031 §5).
- Dependencies: host-installed (python3, python-pptx, libreoffice, fonts);
  cogito best-effort preflight-checks and warns.
- `StorageSystem`: local fs / `blob://` → local dir (v0.5).
- Artifacts: written to an output dir; TUI shows a clickable path.
- Isolation: none (single trust domain — the user's own machine).

### SaaS profile (multi-tenant, single process, behind gateway)
- Skill roots: per-tenant, supplied via `SessionSpec` (ADR-0028);
  **materialized** into the tenant's isolated workspace at session/turn
  start (bundle copied/mounted into the sandbox); `root` = sandbox path.
- `Workspace` = `SandboxWorkspace` — no host FS access; rooted per
  tenant/session (`<tenant>/<session>/`), one ephemeral tree per session,
  injected via `SessionSpec.workspace` (ADR-0031); also the sandboxed exec
  cwd. Durability is opt-in via object storage.
- `CommandExecutor` = sandboxed executor (ADR-0012): container/microVM/
  remote; no host network except via credential proxy (ADR-0013).
- Dependencies: a pre-baked **skill-runtime image** (python + python-pptx
  + libreoffice + fonts), or per-skill declared runtime resolved to a
  layer. No reliance on host packages.
- `StorageSystem`: object store (`storage-s3`, v0.4) with per-tenant
  prefix; artifacts addressed by `blob://tenant/...`.
- Artifacts: persisted to the tenant blob namespace; delivered via signed
  URL surfaced through the consumer gateway; recorded as `ArtifactProduced`.
- Isolation: per-`TenantContext` (ADR-0014); per-session resource budgets
  (mem/CPU/time, v0.4); credentials scoped via proxy, never raw secrets.

The Brain and the skill bundle are byte-identical across the two. Only the
injected Arcs differ — exactly the ADR-0028 mechanism. **Equivalence test
(new contract test):** a fixture skill must yield identical model-visible
tool-call sequences and final text under Local and Sandbox profiles,
differing only in where files/exec/artifacts physically live.

### Open decision — SaaS execution isolation technology

The seam (`CommandExecutor` + `Workspace`) is fixed; the concrete SaaS
backend is an operational choice with cost/security tradeoffs. cogito's
"library not infra" stance argues for **defining the seam and shipping
reference impls, letting the consumer own the sandbox**:

- A. Subprocess + OS user/namespace isolation — cheapest; weak boundary.
- B. Container-per-session (OCI / containerd) — **recommended baseline**;
  mature, image bakes deps, good-enough isolation.
- C. microVM (Firecracker / gVisor) — strongest; heavier ops.
- D. Consumer-provided execution service — cogito calls the injected
  `CommandExecutor`/`Workspace`; the consumer's infra is the sandbox.

Recommendation: treat **D as the primary contract** (the seam already
exists per ADR-0027 — just broaden it to cover the working tree), and ship
**B as the reference SaaS impl** in `cogito-sandbox`'s v0.4 redesign. A is
the local default (DirectExecutor). C is a consumer opt-in. This needs
sign-off before Phase 3.

## 5. Sub-designs

### 5.1 Bundled-file path exposure (ADR-0029)
Add `SkillContent.root: Option<PathBuf>`; injector prepends a one-line
resolvable-path header so SKILL.md's relative refs (`scripts/...`) resolve.
Absolute paths stay out of the event log (ADR-0007) — resolved from the
live registry at injection time. In SaaS, `root` is the *materialized*
sandbox path, not the discovery path. This is Phase 0 — smallest unblock.

### 5.2 `Workspace` seam + file-mutation tools
- New `cogito-protocol::Workspace` trait: `read(path)`, `write(path, bytes)`,
  `list(dir)`, `glob(pattern)`, `exists`, `remove`, all rooted and
  path-traversal-guarded. Injected via `ExecCtx.workspace`.
- `LocalWorkspace` (host, rooted at cwd) and later `SandboxWorkspace`.
- Migrate the `read_file` builtin onto `Workspace`; add `write_file`,
  `edit` (string-replace), `glob`, `grep`, `list_dir` builtins.
- Rationale: without redirectable file I/O, SaaS cannot contain skill
  scratch writes. New ADR.

### 5.3 Execution policy (finalize ADR-0023)
Adopt **Position A** (scripts-as-data + implicit invocation) as the
baseline, now enabled by ADR-0029: the model reads/runs bundled scripts via
`read_file`/`bash`; the loader optionally maps script paths back to their
skill for implicit activation. Position C (auto-register scripts as tools)
is a later ergonomic upgrade (Phase 5). Position B (build-time `` !`cmd` ``
inlining) stays out of scope unless it becomes a portable agentskills.io
standard.

### 5.4 Skill dependency / runtime descriptor (new ADR)
Optional SKILL.md frontmatter (or a sibling manifest): declared runtime and
packages, e.g. `runtime: python3`, `requires: [python-pptx, libreoffice]`.
- Local: best-effort preflight check; warn (do not hard-fail) if missing.
- SaaS: resolve to a runtime image / layer; reject activation if the
  tenant's sandbox image lacks the runtime. Keeps "what does this skill
  need" machine-readable rather than buried in prose.

### 5.5 Artifacts (StorageSystem v0.5 + delivery)
- Skill outputs written through `StorageSystem`; returned to the model as a
  `blob://` URI (and, for images, `ContentBlock::Image` in v0.5).
- New additive event `EventPayload::ArtifactProduced { uri, mime, skill,
  bytes_len }` (ADR-0007 additive; no schema bump).
- Delivery: Local → output dir + TUI clickable line; SaaS → object store +
  signed URL surfaced via consumer gateway. New ADR.

### 5.6 Surfaces
- TUI: `[skill] activating: X` (exists) + show bundled-file root + stream
  script `bash` output in the tools pane + render `ArtifactProduced` as a
  clickable/openable line; handle binary download.
- CLI: same, non-interactive; print artifact path / URI.
- Consumer server (SaaS): `SessionSpec` carries tenant skill roots +
  workspace handle + storage namespace; artifacts via signed URL;
  `ArtifactProduced` forwarded to the product. Extends ADR-0028.

## 6. Phased path (mapped to the version plan)

Each phase opens an extension point additively (the seam shape from §1.3)
and may land its full implementation in a later sprint — the seam is the
commitment, the body is schedulable. Every row is an A/B/C extension with
zero Brain delta; nothing here reopens H01–H11.

| Phase | Lands in | Deliverable | Unblocks |
|---|---|---|---|
| 0 | v0.2 Sprint 13 / patch | ADR-0029: `SkillContent.root` + injector header | Script-bearing skills *readable*; `bash`-run locally if host has deps |
| 1 | v0.2.x → early v0.3 | `Workspace` seam + `write_file`/`edit`/`glob`/`grep`/`list_dir`; migrate `read_file`; finalize ADR-0023 Position A | Skills can author files & run scripts cleanly (Local profile) |
| 2 | v0.2.x → v0.3 | **Shipped as:** skill-bundle *reachability* via a read-only skill-root scope (ADR-0032, replaced "materialization" for Local — no copy); dependency handling decided with no custom descriptor (ADR-0033 — Local agent self-heal, SaaS image pre-bake; replaced the "declare + host-check" descriptor). git/plugin skills carrying scripts (ADR-0022) stays on the plugin track | Script-bearing skills reachable + runnable (Local); deps handled without forking the Skills standard |
| 3 | v0.4 (SaaS-ready) | Sandbox executor (ADR-0012) + credential isolation (ADR-0013) + TenantContext (ADR-0014) + `SandboxWorkspace` + per-session workspace/storage namespace (extend ADR-0028) + resource budgets | **SaaS profile real**: untrusted skill scripts run isolated per tenant |
| 4 | v0.5 | `StorageSystem` + `ArtifactProduced` + binary read + `ContentBlock::Image` + multimedia tools | Binary artifacts (pptx/pdf) persisted & delivered; multimodal skill outputs |
| 5 | v0.6+ | ADR-0023 Position C (scripts-as-tools); skill marketplace; runtime-image management; soak/load hardening | Ergonomic + ecosystem maturity |

End-state checkpoint: after Phase 4, the `pptx` skill runs end-to-end under
both profiles — Local (TUI shows the produced `.pptx` path) and SaaS
(isolated sandbox renders it, object store holds it, gateway returns a
signed URL).

## 7. ADR ledger

- ADR-0029 (drafted, implemented) — bundled-file path exposure. **Phase 0.**
- ADR-0023 (finalized 2026-06-02) — adopted Position A (scripts-as-data;
  read via `read_file`/ADR-0032, run via `bash`/ADR-0027/0031 §5). **Phase 1/2.**
- ADR-0030 / ADR-0031 (drafted, implemented) — `Workspace` seam +
  provisioning/scoping + exec-cwd unification; the file-mutation tool set
  (`read_file` migrated, `write_file`/`list_dir`/`edit`/`grep`/`glob`).
  **Phase 1.**
- ADR-0032 (proposed) — skill-bundle reachability via a read-only skill-root
  scope (`ExecCtx.skill_roots`; `read_file`/`list_dir`/`exists` resolve into
  registered skill dirs; no Local copy; body injection unchanged; SaaS physical
  placement deferred to Phase 3). Aligns with how Claude Code / Codex reach
  bundles in place. **Phase 2.**
- ADR-0033 (proposed) — skill runtime dependencies: decided **not** to invent
  a custom descriptor (the Agent Skills standard defines no `runtime`/`requires`
  — only the free-text `compatibility` field; deps live in prose). Local =
  agent self-heal via `bash` (ADR-0023 Position A); SaaS = pre-baked runtime
  image (popular tools + skill-declared libs, out-of-band image build) +
  optional activation fast-fail; safe cogito-driven auto-install deferred to
  Phase 3 (sandbox / ADR-0012). No code change in v0.2. **Phase 2.**
- New — artifact production & delivery (`ArtifactProduced`, blob namespace,
  delivery). **Phase 4** (event可 land earlier).
- Extend ADR-0028 — `SessionSpec` workspace + storage namespace + tenant
  skill roots. **Phase 3.**
- Leverage planned ADR-0012 / 0013 / 0014 (sandbox / cred / tenant) for the
  SaaS profile. **Phase 3.**

## 8. Invariants and guardrails

- Brain imports only `cogito-protocol`; every new capability is an injected
  trait object (ADR-0004). No `use cogito_sandbox::*` in `harness/`.
- No absolute host paths in the event log (ADR-0007). Artifacts and skill
  roots are referenced by stable URI / resolved-at-injection, never
  persisted as host paths.
- Profile equivalence is a contract test, not a hope: one fixture skill,
  two profiles, identical model-visible behavior.
- Resume / multi-replica: skill activation is already an event; the
  materialized workspace and artifacts must be rebuildable or re-fetchable
  on any replica (ties to v0.4 self-describing resume). A turn must not
  depend on host-local scratch that a sibling replica cannot reconstruct.
- SaaS never runs skill scripts in the host trust domain — Phase 3's
  sandbox executor is a hard precondition for enabling execution under the
  SaaS profile.

## 9. Risks and decisions needing sign-off

1. **SaaS isolation technology** (§4) — confirm "seam + reference container
   impl, consumer owns sandbox" (recommended) vs cogito shipping a
   first-class managed sandbox. Blocks Phase 3 design.
2. **`Workspace` vs reuse `StorageSystem`** — recommend a dedicated
   `Workspace` seam (working tree) distinct from `StorageSystem` (blob).
   Confirm before Phase 1.
3. **Dependency provisioning ownership** — cogito ships the descriptor +
   a reference runtime image; the consumer owns the image registry and
   patching. Confirm the boundary.
4. **Execution default-on vs opt-in** — even Position A lets the model run
   arbitrary `bash`; confirm per-strategy / per-tenant gating so a skill
   cannot execute unless the session policy allows it.

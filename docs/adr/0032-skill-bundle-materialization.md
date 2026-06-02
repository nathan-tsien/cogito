# ADR-0032: Skill-bundle materialization into the workspace

## Status

Proposed (2026-06-02). Opens Phase 2 of the complete-skill-support path
(`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md` §5.2/§6).
Builds on ADR-0029 (bundled-file path exposure), ADR-0030 (`Workspace` seam),
ADR-0031 (workspace provisioning + exec-cwd unification), and ADR-0028
(per-session provider injection). Resolves the materialization question
ADR-0030 flagged as a "Phase-2/3 concern".

## Context

A Skill is a directory: `SKILL.md` plus bundled `scripts/`, `references/`,
`assets/`. Today (ADR-0029) the loader carries the skill's **absolute host
directory** in `SkillContent.root` and the injector renders it into the
`<skill ... root="...">` header with a one-line "resolve relative paths
against this root" hint. The model reaches bundled files by that absolute
path via `bash` / `read_file`.

That works in the Local profile because `DirectExecutor` and the host
filesystem can see the skill directory. It **breaks in the SaaS profile**:
the per-tenant sandbox (ADR-0012, v0.4) has its own filesystem and cannot see
the host's skill directory, so an absolute host path is unreachable. ADR-0030
explicitly deferred reconciling "model sees absolute skill paths" with
"`Workspace` takes root-relative paths", noting: *"when bundled files are
materialized into the workspace, the model will reference them by their
workspace-relative path."* This ADR decides how.

Two further forces:

- **Phase 1 unified the exec cwd on the session workspace root** (ADR-0031
  §5): `bash`, `read_file`, `write_file`, `list_dir`, `edit`, `grep`, `glob`
  all operate against one rooted, confined working tree per session. A skill
  bundle that lives *inside* that tree is reachable by every one of those
  tools with no special-casing.
- **The thesis is zero Brain delta** (spec §1). Materialization must be a
  Hands-level capability, injected per session; H01-H11 and the skill bundle
  stay identical across profiles.

## Decision

### 1. Materialize bundles into the workspace, both profiles

On activation, a skill's bundled files are copied into the session workspace
under a reserved prefix:

```
<workspace-root>/.cogito/run/skills/<activation-name>/…
```

`<activation-name>` is the registered name (`pptx`, or `myplugin:pptx` for a
plugin skill — `:` is path-safe). `SKILL.md` itself is **not** copied (its
body is already injected); everything else under the skill directory is copied
recursively.

The injected `root` then becomes that **workspace-relative path**
(`.cogito/run/skills/pptx`), identical in shape across profiles:

- **Local**: `<cwd>/.cogito/run/skills/pptx/…` — under the project cwd
  (ADR-0031 §3). The `.cogito/run/` prefix is reserved scratch; the loader
  ensures `.cogito/run/.gitignore` (`*`) exists so materialized bundles never
  pollute the user's VCS.
- **SaaS**: `<tenant-sandbox>/.cogito/run/skills/pptx/…` — inside the tenant
  workspace, reachable by the sandboxed `bash`.

Same `root="…"` value, same relative structure, same model-visible behavior —
the spec §4 parity requirement holds *literally*, not just structurally.

Rejected alternatives: see "Alternatives considered".

### 2. Mechanism: a `MaterializingSkillProvider` decorator (zero Brain delta)

Materialization is a `SkillProvider` decorator living in a Hands crate
(`cogito-skills`), wrapping the base provider and holding the session
`Arc<dyn Workspace>`:

```rust
pub struct MaterializingSkillProvider {
    inner: Arc<dyn SkillProvider>,
    workspace: Arc<dyn Workspace>,
}
```

On `get(name)` it (a) fetches the base `SkillContent` (host `root`), (b) copies
the bundle into `.cogito/run/skills/<name>/` through `Workspace::write`
(confined by construction), and (c) returns a `SkillContent` whose `root` is
the workspace-relative staged path. `list` / `is_registered` / `get_metadata`
pass through unchanged.

It is composed **caller-side and per session** (ADR-0028): the Surface builds
`MaterializingSkillProvider::new(base, session_workspace)` and injects it via
`SessionSpec.skills`. The Brain only ever sees `dyn SkillProvider`; the
injector renders whatever `root` it is handed; **no H01-H11 change, no injector
change, no new Brain-facing trait.** A session without a workspace (or whose
base `root` is `None`) gets a pass-through — the decorator simply isn't
wrapped, or returns the base content unchanged.

### 3. `SkillProvider::get` becomes async

Copying is I/O, but `get` is synchronous today. Both call sites
(`SkillInjector::inject` and `build_body_blocks`) already run in `async fn`s,
so `get` is changed to:

```rust
async fn get(&self, name: &str) -> Option<SkillContent>;
```

This is a contained `cogito-protocol` change (the trait is runtime-only, not a
wire/event type, so no `SCHEMA_VERSION` impact). `list` / `is_registered` /
`get_metadata` stay synchronous — the H06 sigil filter and the registry block
must remain cheap and need no I/O. Materialization is therefore **lazy**: only
activated skills are copied, not every registered one.

### 4. Idempotent, materialize-once-per-session

`get` is called per turn for each active skill; the copy must be idempotent.
The decorator skips copying when the destination already exists for the
session (a skill activated in turn 3 and re-referenced in turn 5 is copied
once). Re-materialization across a *crash/resume* is harmless (overwrite) but
the existing injector idempotency (a `SystemPromptInjected` event already
present for the turn short-circuits injection) means `get` is not even called
on a resumed turn.

### 5. Event-log / resume

The materialized `root` (`.cogito/run/skills/pptx`) is **workspace-relative and
machine-independent**, so the `SystemPromptInjected.suffix` that persists it
(ADR-0029 decision-point 4) no longer freezes a host-absolute path into the
log. This is a strict improvement over Phase 0 for the v0.4 multi-replica case:
a replica re-resolves the same relative path against its own workspace root.

## Consequences

**Easier**:
- One reachability model: bundled files live in the workspace, reachable by
  every Phase-1 file tool and by `bash` (shared cwd), in both profiles.
- SaaS skill execution becomes possible at all (the sandbox can now see the
  bundle) with zero Brain change — just a different injected `Workspace`.
- The persisted skill root stops being a host-absolute path (resume-friendly).

**Harder**:
- `SkillProvider::get` gains `async` — a `cogito-protocol` trait change that
  touches `SkillRegistry`, the injector call sites, and any test doubles.
- Local sessions write a hidden `.cogito/run/skills/…` tree into the project
  cwd. Mitigated by the reserved prefix + auto-`.gitignore`; users wanting
  full isolation point `--workspace` at a scratch dir (ADR-0031 §3).
- Copy cost on first activation (bounded: bundles are scripts/templates, not
  dependency trees — runtimes/packages are ADR-0033's concern, not copied).

**Given up**:
- Activating a skill is no longer pure prompt-assembly; it has a file-I/O side
  effect. That side effect is confined to a Hands impl behind the
  `SkillProvider` seam, so the layering rule holds.

## Alternatives considered

1. **SaaS-only materialization; Local keeps the absolute host root.** Avoids
   touching the Local project tree, but diverges the literal `root=` value by
   profile and keeps two reachability models. Rejected: undercuts the spec §4
   "same skill, same behavior" thesis and leaves the Phase-0 absolute-path
   resume caveat in place for Local.
2. **Session staging dir outside the project, exposed as a second read root.**
   Keeps the Local project clean and unifies paths, but breaks the
   single-rooted `Workspace` invariant (ADR-0030/0031) by introducing a second
   mount/root. Rejected for v0.2: a large complication to the seam for a
   hygiene gain the reserved `.cogito/run/` prefix already mostly buys.
3. **A dedicated `SkillMaterializer` Hands trait invoked by the Turn Driver.**
   Explicit, but adds a call from H01 → Brain delta. Rejected: the decorator
   achieves the same with zero Brain change.
4. **Eager materialization of all skills at session open** (keeps `get` sync).
   Avoids the trait change but copies every registered skill — wasteful with
   many plugin skills, and pays the cost even for sessions that activate none.
   Rejected in favor of lazy + async `get`.

## Open questions

1. Pseudo-XML attribute escaping (ADR-0029 TODO): still deferred. Skill
   directories remain operator/loader-authored in Phase 2; tenant-controlled
   skill roots (Phase 3) require escaping the `root="…"` attribute regardless
   of materialization.
2. Symlinks inside a bundle: copy-follow vs copy-as-link. v0.2: follow and
   copy file contents (the destination is a flat confined tree); revisit if a
   skill legitimately ships symlinks.
3. Cleanup: materialized trees are ephemeral with the session (ADR-0031 §1) in
   SaaS; locally they persist under `.cogito/run/` until the user clears it.
   A `cogito` housekeeping command is out of scope here.
4. Cross-session caching of identical bundles (content-addressed staging) is a
   later optimization, not a v0.2 need.

## References

- ADR-0029 — skill bundled-file path exposure (the absolute-root header this
  refines into a workspace-relative one)
- ADR-0030 — `Workspace` seam (the "materialize into the workspace" note this
  answers)
- ADR-0031 — workspace provisioning + exec-cwd unification (why an in-workspace
  bundle is reachable by `bash` and the file tools)
- ADR-0028 — per-session provider injection (the caller-side composition the
  decorator reuses)
- ADR-0023 — bundled-script execution (Position A: read/run scripts via
  `read_file`/`bash`; materialization is what makes those reads/runs reachable)
- Complete-skill-support design §4 (profiles), §5.2 (workspace + tools), §6
  (Phase 2 row)
- Skill machinery touched: `cogito-protocol/src/skill.rs` (`SkillProvider`,
  `SkillContent.root`); `cogito-skills/src/registry.rs`,
  `crates/cogito-skills/src/discovery.rs`; `cogito-context/src/injector/skill.rs`
  (`build_body_blocks`); `cogito-protocol/src/workspace.rs`

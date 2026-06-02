# ADR-0033: Skill runtime dependencies — no custom descriptor; agent self-heal (Local) + image pre-bake (SaaS)

## Status

Proposed (2026-06-02). Phase 2 of the complete-skill-support path
(`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md` §5.4/§6/§7).
Records a **decision not to build** a custom dependency descriptor in v0.2, and
where the real mechanism lives instead. Builds on ADR-0020 (skill loader /
frontmatter), ADR-0023 (Position A — read/run bundled scripts via
`read_file`/`bash`), ADR-0029/0032 (bundle reachability). Sibling to the SaaS
sandbox work (ADR-0012, Phase 3).

> Supersedes this ADR's own first draft, which proposed inventing optional
> `runtime` + `requires` SKILL.md frontmatter fields plus a Local PATH preflight
> check. Investigation (see Context) showed (a) the Agent Skills standard defines
> no such fields, and (b) the agent loop already handles missing dependencies
> more accurately than a static preflight. This revision drops both.

## Context

After ADR-0029/0030/0031/0032, a script-bearing skill (e.g. `pptx`) is reachable
and runnable in the Local profile: its `SKILL.md` body is injected, its bundled
files are readable in place, and the model can run a script with `bash`. But
whether that `bash` call succeeds depends on the **host environment** having the
right tools (`python3`, `python-pptx`, `libreoffice`, fonts). Today that need is
in the skill's prose, surfaced only as an opaque runtime failure
(`bash: python3: command not found` / `ModuleNotFoundError`).

The first draft of this ADR proposed making the requirement machine-readable via
two new frontmatter fields (`runtime: python3`, `requires: [python-pptx,
libreoffice]`) plus a best-effort Local PATH preflight that warns. Two findings
killed that approach.

**Finding 1 — the Agent Skills standard defines no such fields.** The canonical
SKILL.md frontmatter (agentskills.io specification, cross-checked against
Anthropic's Claude API best-practices and the Claude Code skills reference) is:

- `name` (required), `description` (required)
- `license` (optional), `compatibility` (optional), `metadata` (optional),
  `allowed-tools` (optional, experimental)
- plus Claude Code vendor extensions (`when_to_use`, `model`, `effort`, `agent`,
  `hooks`, `paths`, `shell`, `disable-model-invocation`, `user-invocable`, …)

**None** of these is `runtime` / `requires` / `dependencies` / `interpreter` /
`packages`. The standard's *only* dependency-related field is **`compatibility`**
— a **free-text** human string (≤500 chars), e.g.
`compatibility: Requires Python 3.14+ and uv` — explicitly **not** structured.
The standard's guidance is to "list required packages in your SKILL.md and verify
they're available" — i.e. **in prose / the body**, not a typed field. cogito
inventing `runtime`/`requires` would fork the standard for non-standard keys that
spec-compliant consumers ignore. Per project policy we follow the agentskills.io
standard (ADR-0020), so we do not invent these fields.

**Finding 2 — the agent loop already self-heals, more accurately.** cogito is an
agent: with `bash` + ADR-0023 Position A, the model can run a script, observe the
real failure, `pip install python-pptx` (or report the gap), and retry — today,
with no new machinery. A static PATH preflight gives a *weaker, less accurate*
signal than actually running ("declared but not on PATH" misses venvs, pip
packages-vs-binaries, aliases, wrappers). Building frontmatter grammar + a parser
+ a preflight module + Surface rendering to produce that weaker signal is not
worth it.

So the question is not "what descriptor do we add" but "where does dependency
satisfaction actually belong" — and the answer differs by profile.

## Decision

### 1. Do not invent `runtime` / `requires` (stay on the standard)

cogito adds **no** custom dependency/runtime frontmatter fields in v0.2. Skills
declare their needs the way the Agent Skills standard prescribes: in prose (and,
where authors choose, the standard's free-text `compatibility` field). The loader
already ignores unknown frontmatter keys (`RawFrontmatter` is not
`deny_unknown_fields`), so a skill carrying `compatibility`/`metadata` parses
fine today — we simply do not give those fields runtime semantics yet.

### 2. Local: dependency satisfaction is the agent loop's job (self-heal via bash)

In the Local profile, a missing dependency is handled by the **agent loop**, not
by cogito machinery: the model runs the script via `bash`, sees the real error,
and either installs the dependency and retries or reports precisely what is
missing. This is the most accurate signal available and needs **zero new code** —
it falls out of ADR-0023 Position A + the existing `bash` tool. No preflight, no
warning pass, **zero Brain delta** (the model is the decider; `bash` is the hand).

We deliberately do **not** build cogito-driven auto-install into the Local *global
host*: installing there (global `pip`, `apt`/`sudo`, version conflicts) is
destructive and hard to reverse. Doing it *safely* requires an isolation boundary
(a managed per-session environment), which is Phase 3 — see Decision 4.

### 3. SaaS: a pre-baked runtime image, fed by skill prose out of band

In the SaaS profile, scripts run in a pre-baked runtime image (spec §4). The
intended answer to "what does this skill need" is **image composition**, not a
runtime check inside the tenant:

- the base image **pre-installs popular tools** (python3, node, libreoffice,
  common fonts) so most skills work out of the box;
- the long tail (a skill needing something unusual) is covered by the
  **operator/consumer's image-build tooling**, which may read each skill's prose
  `compatibility`/body to decide what to add — an **out-of-band, build-time**
  input, not a cogito runtime feature;
- optionally a **fast-fail at activation** if the tenant image is known to lack a
  required capability (so the user gets a clear reason instead of a deep sandbox
  failure). This gate, and any capability manifest it reads, is **Phase 3 /
  ADR-0012** work.

cogito at runtime never installs into a tenant; the image is the ground truth.

### 4. Safe controlled auto-install is deferred to Phase 3 (sandbox)

If we later want cogito itself (rather than the model improvising) to resolve a
missing dependency, it must install into an **isolated, disposable managed
environment** (venv / sandbox layer), never the host. That isolation boundary is
exactly what the Phase 3 sandbox (ADR-0012) + `SandboxWorkspace` introduce. Any
"detect → install → retry" automation is scheduled there, where it can be safe.

## Consequences

**Easier**:
- We stay faithful to the Agent Skills standard — no forked, non-portable
  frontmatter that compliant consumers ignore (ADR-0020 honored).
- Zero new machinery and zero Brain delta in v0.2: Local dependency handling is
  the agent loop we already have; the signal (a real run) is more accurate than a
  PATH probe would have been.
- The SaaS story is correctly placed in image/ops, with a clean Phase 3 home for
  the optional activation gate.

**Harder**:
- A missing Local dependency still surfaces first as a runtime `bash` error, not
  an up-front structured warning. We accept this: the model can read that error
  and act, and in practice it is the same loop a human would run.
- "What a skill needs" remains semi-structured (prose / free-text
  `compatibility`), so SaaS image tooling that wants to automate pre-install must
  parse prose or rely on a curated list — acceptable for an out-of-band build
  step, and revisitable if the standard later adds a structured field.

**Given up**:
- The machine-readable, queryable dependency list the first draft promised. If a
  concrete need arises (e.g. SaaS image automation at scale), the standard's
  `metadata` map is the sanctioned escape hatch, or we revisit if/when
  agentskills.io standardizes a structured field. We do not pre-build it.

## Alternatives considered

1. **Invent `runtime` + `requires` frontmatter + Local PATH preflight** (this
   ADR's first draft). Rejected: the fields are not in the Agent Skills standard
   (forking it for keys compliant tools ignore), and the PATH preflight is a
   weaker signal than the agent simply running the script. Over-engineering.
2. **Give the standard `compatibility` field runtime semantics now** (parse it,
   check/gate on it). Rejected for v0.2: it is deliberately free-text (≤500
   chars, prose), not reliably machine-checkable, and gating on it in Local
   repeats the preflight mistake. May feed SaaS image tooling later (Open
   question 1).
3. **cogito auto-installs missing Local deps into the host.** Rejected:
   destructive / irreversible (global pip, sudo). Safe auto-install needs
   isolation → Phase 3 (Decision 4).
4. **Use the `metadata` k/v map for a private `cogito:runtime` key.** Possible
   escape hatch, but pre-building it now has no consumer (Local self-heals, SaaS
   pre-bakes). Deferred until a real need (Given up).

## Open questions

1. **SaaS image automation.** When Phase 3 builds the sandbox, decide whether the
   image-build/consumer tooling parses skill `compatibility`/prose to auto-compose
   the runtime image, or relies on a curated capability list. Out of band either
   way.
2. **Activation fast-fail gate.** The optional "tenant image lacks capability ⇒
   reject activation with a clear reason" path — design with ADR-0012; needs a
   capability manifest the gate can read.
3. **Phase 3 controlled auto-install.** Whether cogito (vs. the model) drives
   detect→install→retry inside the managed sandbox environment, and its policy.
4. **Standard evolution.** Revisit if agentskills.io adds a structured
   runtime/dependency field upstream; adopt the standard's shape rather than a
   cogito-private one.

## References

- ADR-0020 — skill loader + SKILL.md frontmatter (we stay on its
  agentskills.io-compatible field set; unknown keys already ignored)
- ADR-0023 — bundled-script execution, Position A (the `bash`/`read_file` path the
  Local self-heal relies on)
- ADR-0029 / ADR-0032 — bundled-file path exposure + reachability (the reads/runs
  whose host this concerns)
- ADR-0028 — per-session provider injection (where a SaaS activation gate would
  ride, Phase 3)
- ADR-0004 — Brain/Hands/Session boundaries (why the decider is the model + Hands,
  never new Brain logic)
- ADR-0012 — SaaS sandbox (Phase 3 home of the activation gate + safe auto-install)
- Agent Skills standard frontmatter: agentskills.io specification (fields: `name`,
  `description`, `license`, `compatibility`, `metadata`, `allowed-tools`);
  Anthropic Claude API "Agent Skills" best-practices ("list required packages in
  your SKILL.md"); Claude Code skills reference (vendor extensions — still no
  `runtime`/`requires`). Confirms no structured dependency field exists upstream.
- Complete-skill-support design §4 (profiles), §5.4 (this descriptor question),
  §6 (Phase 2 row), §7 (ledger)
- No code change in v0.2: the loader (`cogito-skills`), `SkillMetadata`, the
  injector, and the event log are all untouched. The Local mechanism is the
  existing agent loop; the SaaS mechanism is image composition (Phase 3).

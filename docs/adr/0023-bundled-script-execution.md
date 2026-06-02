# ADR-0023: Bundled-script execution in Skills (Position A — scripts as data)

## Status

**Accepted — Position A finalized 2026-06-02** (complete-skill-support Phase 1/2;
spec §5.3). Adopts scripts-as-data + read/run via existing tools. Position B
(build-time `` !`cmd` `` inlining) and Position C (scripts-as-tools) remain out of
scope — see Decision.

This ADR was created during the 2026-05-22 roadmap rebalance as a **deliberate
deferral** that recorded the design space without choosing
(see [spec §2.6 B-defer](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)).
That deferral text is preserved below as history; the finalized decision is in
**Decision**. The trigger to revisit ("a concrete use case surfaces") was met: the
complete-skill-support design (ADR-0029/0030/0031/0032/0033) made script-bearing
skills reachable and runnable, and the `pptx` skill is the concrete consumer.

## Context

The agentskills.io specification (and SKILL.md format adopted by
ADR-0020) allows a skill folder to contain a `scripts/` subdirectory:

```
my-skill/
  SKILL.md
  scripts/
    fetch.py
    process.sh
```

Different platforms handle these differently:

- **Codex** — scripts are **not auto-executed**. The SKILL.md
  instructions tell the model where scripts live; the model reads
  them via `read_file` / runs them via `bash` if the agent's normal
  tool permissions allow. Codex's special trick: **implicit
  invocation** — when the model runs `bash skills/X/scripts/y.py`,
  Codex detects the path and auto-activates skill X if not already
  active (via `detect_implicit_skill_invocation_for_command`).
- **Claude Code** — `SKILL.md` can embed `` !`shell command` `` or
  fenced ` ```! ` blocks that **execute at skill load time** and
  inline the output into the prompt before the model sees it. This
  is a **build step**, not a runtime tool. Can be disabled
  org-wide via `disableSkillShellExecution: true`.
- **Manus** — runs in an isolated VM; scripts are uploaded as part
  of the skill bundle but execution model is opaque (handled by
  Manus's sandbox).

cogito v0.1 Sprint 7 adopts **B-defer**: skill `scripts/` directories
may exist but are **not executed by the loader**. The model can
access them via existing tools (`read_file`, `bash`).

Three plausible future positions need design — none chosen yet:

### Position A — Codex-style: scripts as data + implicit invocation

Scripts live on disk; model reads/runs them through normal tools.
Loader optionally tracks "which script paths belong to which skill"
to support implicit activation: when the model runs
`bash skills/X/scripts/y.py`, activate skill X. Cheapest to ship.

### Position B — Claude Code-style: build-time inline substitution

`SKILL.md` may embed `` !`cmd` `` / ` ```! ` blocks that the loader
executes during skill activation, before injecting content into the
context. Output is inlined as text. Requires sandboxing decisions
(what's allowed to run? whose environment? PATH? resource limits?).

### Position C — Auto-register scripts as tools

Each `scripts/<file>` becomes a tool `skill__<skill>__<file>` with
parameters inferred from the script's shebang/argparse/usage block.
Most powerful, most complex; requires parameter schema inference
and a sandbox story.

## Decision

**Adopt Position A: scripts as data, read and run via the existing tools.** A
skill's `scripts/` files are reached with `read_file` and executed with `bash`;
the loader does not execute anything. Position B and Position C stay out of scope
(B unless `` !`cmd` `` becomes a portable agentskills.io standard; C is a Phase-5
ergonomic upgrade). This is the same model Claude Code and Codex use
(progressive disclosure + read/run in place), and it required **no Brain change**
— it composes seams that already exist.

The pieces that make Position A real (all shipped):

- **Reach** — `read_file`/`list_dir` resolve a bundled file by its absolute
  skill-root path through the read-only skill-root scope (**ADR-0032**;
  `ExecCtx.skill_roots`, wired from `SkillProvider::skill_roots()`).
- **Run** — the `bash` tool (`cogito-jobs::BashTool`) runs commands through the
  injected `CommandExecutor` (**ADR-0027**); with no explicit `cwd` it runs in the
  **workspace root** (**ADR-0031 §5** exec-cwd unification), so script output
  lands in the session workspace, not the bundle.
- **Locate** — the activated skill's bundle root is surfaced to the model in the
  `<skill root="...">` header (**ADR-0029**), so SKILL.md's relative `scripts/…`
  references resolve.
- **Dependencies** — host runtime/packages are handled by the agent loop
  (self-heal via `bash`) in Local and by the pre-baked image in SaaS
  (**ADR-0033**); cogito declares no custom descriptor.

End-to-end coverage: `crates/cogito-jobs/tests/skill_script_e2e.rs` drives a
Runtime turn that `read_file`s a bundled script then `bash`-runs it, asserting the
artifact is produced in the workspace. The `pptx` skill is exercised manually in
`docs/experiments/2026-06-02-skill-support-phase2.md`.

Resolving the original **open design questions**:

1. *Which position?* **A only.** Implicit-invocation-by-script-path (Codex's
   trick) is **not** implemented — explicit `$Skill` sigil + model-invocation
   suffice; revisit if a need appears.
2. *Sandbox boundary?* Local = `DirectExecutor` subprocess via the
   `CommandExecutor` seam; SaaS = sandbox executor in Phase 3 (ADR-0012). No
   Sandbox v2 lifecycle needed — the seam was enough.
3. *Permission model?* The `bash` tool is gated like any tool: the tool-allow set
   + Hooks (H09) + the `[tools].bash` / `[skills]` config. Global off = do not
   register `bash` (or disable skills).
4. *Position C parameter schema?* N/A — C is deferred (Phase 5).
5. *Output capture?* `bash` returns a structured `{ stdout, stderr, exit_code }`
   payload; limits/streaming are the executor's concern.

The format remains **read-compatible**: SKILL.md files written for Claude Code or
Codex load and now also *run* their scripts via Position A.

## When to revisit

Revisit this ADR when **any one** of the following triggers:

- A team-internal plugin requires script execution to function
  (concrete use case)
- A second cogito consumer requests the feature
- v0.3 Subagent (S1 full) ships and a richer execution model exists
  to leverage (subagents can encapsulate "run this script as a sub-
  task")
- Claude Code's `` !`cmd` `` pattern matures into agentskills.io
  spec (turning it into a portable standard rather than a Claude-Code
  extension)

## Open design questions (resolved at finalization — see Decision)

These were the questions left open by the deferral; their answers are recorded in
**Decision** above. Preserved here for the original framing.

1. Which position(s)? A is cheapest; B is most user-friendly; C is
   most powerful. Combinations are possible (e.g., A + B but not C).
2. Sandbox boundary: pure subprocess, OS user isolation, container,
   or `cogito-sandbox`? Reuse must not require Sandbox v2 lifecycle.
3. Permission model: explicit allow per-skill in `cogito.toml`?
   Global on/off via `disable_skill_shell_execution`?
4. Parameter schema for position C: hand-written JSON schema in
   skill folder vs inference from script source? (Inference is
   hostile to correctness; hand-written is friction.)
5. Output capture: stdout-only? stderr separately? size limits?

## Consequences (of deferring)

**Easier**:
- v0.1 Sprint 7 ships cleanly with no sandbox / permission decisions
- Bundled scripts in upstream SKILL.md files don't block loading
- More design space available once real use cases surface

**Harder**:
- Users expecting Claude Code parity will be surprised that
  `` !`cmd` `` blocks don't execute — needs prominent doc note
- Skills depending on script execution must instruct the model to
  `read_file` + `bash` manually, which is more verbose

**Given up**:
- Day-one feature parity with Claude Code skills that use `` !`cmd` ``
  substitution
- Implicit invocation by script path (Codex's trick) — minor; users
  can use sigil `$SkillName` for explicit activation

## References

- Rebalance spec: [`docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md`](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md) §2.6
- ADR-0020 — Skill loader (the parent decision, defers to this ADR)
- Codex implicit invocation: `codex-rs/core-skills/src/invocation_utils.rs`
- Claude Code `` !`cmd` `` docs: https://code.claude.com/docs/en/skills (§ "Inject dynamic context")

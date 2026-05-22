# ADR-0023: Bundled-script execution in Skills (deferred design)

## Status

Proposed — **deliberately deferred** (target finalization: v0.3+ TBD).

This ADR exists to **record the deferral and the design space**, not
to make a decision. Created during the 2026-05-22 roadmap rebalance
(see [spec §2.6 B-defer](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)).

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

**Defer.** No execution path implemented in v0.1 or v0.2. The format
is **read-compatible**: SKILL.md files written for Claude Code or
Codex load successfully into cogito, but their `scripts/` content is
inert until this ADR is finalized.

The B-defer position is recorded as a placeholder so the question is
**explicitly open** rather than silently ignored.

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

## Open design questions (for whichever sprint finalizes this ADR)

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

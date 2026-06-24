# ADR-0042: Skill activation — tool-call primary channel

## Status

Accepted (v0.3). Supersedes ADR-0020 §1 (K5 sigil-primary activation).

## Context

ADR-0020 chose sigil-in-prose (`$SkillName`) as the model activation
channel and explicitly rejected a `load_skill` tool, on two premises: that
no platform used tool-call activation, and that a tool round trip was pure
overhead versus free context injection. Both premises no longer hold:
Claude Code ships a `Skill` tool and NousResearch hermes-agent uses
`skill_view(name)`; and the round-trip cost is dominated by the cost of a
missed activation. Sigil-in-prose is out-of-distribution for tool-trained
models, the injected index carried no forcing instruction, and the sigil
guardrails (code-fence skipping, exact-name matching) create false
negatives — so activation triggered unreliably.

## Decision

1. Add an `activate_skill(name)` builtin tool as the primary activation
   channel. It returns the skill's full SKILL.md body as its `ToolResult`,
   delivered in-turn and persisted by `ToolResultRecorded`.
2. Keep sigil + slash as always-on fallbacks (multi-model portability — the
   original ADR-0020 driver). Three channels run in parallel.
3. The injected index becomes `## Skills (mandatory)` with an imperative
   "you MUST call `activate_skill`" instruction (sigil noted as the no-tools
   fallback), scope-precedence ordering, scope grouping, and logged caps.
4. No new event variant: `ToolUseRecorded` + `ToolResultRecorded` are the
   tool-channel activation record. `SkillActivated` stays sigil/slash-only.
   No `SkillActivationChannel` variant added.
5. `SkillInjector` dedups against prior successful `activate_skill` calls so
   a tool-loaded body is never re-injected via the sigil path.
6. Filtering uses existing metadata only (scope, disable-model-invocation,
   caps) — no frontmatter change (honors ADR-0033).

## Consequences

Brain delta is zero (additive Hands tool + additive protocol renderer +
context-injector presentation). Reliable activation on tool-capable models;
portability preserved for vLLM/SGLang via sigil. Spec:
`docs/superpowers/specs/2026-06-23-skill-activation-redesign-design.md`.

Instruction/tool coupling (embedder note): the `## Skills (mandatory)` block
unconditionally instructs the model to call `activate_skill`, but registering
that tool is a separate Surface decision — the reference CLI and TUI add it
(behind `if let Some(skills)`), and an embedder composing its own
`ToolProvider` could surface the instruction while the tool is absent. This
degrades gracefully: the same instruction names the `$<name>` sigil fallback,
and the sigil channel remains active, so a model with no `activate_skill` tool
still has a working activation path. Embedders that wire a `SkillProvider`
should also register `activate_skill` (or rely on the sigil fallback). A
future refinement could derive the instruction's tool mention from the
turn's actual tool surface rather than emitting it unconditionally.

## References

- ADR-0020 (superseded §1), ADR-0029, ADR-0033, ADR-0004, ADR-0007.
- NousResearch hermes-agent `prompt_builder.py` / `skill_*.py`.

# ADR-0020: Skill loader (`cogito-skills`)

## Status

Accepted (Sprint 7, 2026-05-23).

This ADR captures decisions ratified in the
[2026-05-22 roadmap rebalance spec](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
(§2.5 + §2.6 + §4.1) and finalized during Sprint 7 when the loader was
implemented. See the "Sprint 7 closure notes" section at the bottom for
the locked-in values (sigil regex, description cap, repo-root stop set,
code-fence handling).

## Context

The roadmap rebalance promotes Skill loader from "post-Sprint-2 spike"
to v0.1 Sprint 7. The motivation is to give the team a **userland**
extension surface (markdown + TOML, no Rust required) before v0.1 tag,
so that domain knowledge and workflows can ship in parallel with engine
work.

The skill format converged industry-wide on **agentskills.io** — an
Anthropic-originated open standard now adopted by 30+ agent platforms
(Claude Code, Codex, Cursor, Manus, Goose, Letta, …). A skill is a
folder containing a `SKILL.md` file with YAML frontmatter (`name`,
`description` minimum) plus optional `scripts/` / `references/` /
`assets/` subdirectories. Loading follows **progressive disclosure**:

1. **Discovery** — only name+description loaded at startup
2. **Activation** — full SKILL.md injected when task matches description
3. **Execution** — bundled scripts can be referenced/executed

Three platforms surveyed for activation mechanics (see rebalance spec
§2.5 for details):

- **Codex** — sigil-based: model writes `$SkillName`; harness regexes it
- **Claude Code** — natural language ("I'll use the X skill") + user
  `/skill-name` + built-in `Skill` tool for a few commands
- **Manus** — user `/SKILL_NAME` slash only; no model auto-activation

**No platform uses tool-call activation** (e.g., `load_skill(name)`
as a tool). Reason: activation is "inject instructions into context",
which is essentially free; a tool round trip is overhead.

cogito is a multi-model runtime (Anthropic + OpenAI-compat + vLLM/
SGLang). Natural-language activation depends on model training that
private deployments cannot guarantee. Sigil activation is model-
portable — any instruction-following model can emit `$Name`.

## Decision

### 1. Activation mechanism: **K5 — sigil-based + slash command (dual channel)**

- Model channel: model emits `$SkillName` (or `$plugin:name`) in its
  text reply; H06 Stream Demultiplexer detects via regex; emits
  `ModelEvent::SkillActivationRequested`; H11 Context Manage injects
  full SKILL.md as a user-role message before the next turn.
- User channel: CLI / TUI surface accepts `/skill <name>` from the user
  and emits the same activation event.
- **No `load_skill` tool.** Skills are not registered as tools.

Sigil regex: starting point is roughly `\$[a-zA-Z0-9_:-]+` with these
guardrails (final form locked in Sprint 7):

- (a) Sigil matches only if the captured name is in the registered skill
  set; unknown `$X` is treated as literal text.
- (b) System prompt instructs the model: "to refer to a literal `$Name`,
  wrap in backticks."
- (c) Optional: skip sigils inside fenced code blocks / inline code
  (deferred to Sprint 7 implementation evaluation).
- (d) Codex's exact regex consulted as starting reference.

### 2. Scope precedence

Skills are discovered from four scopes, with conflict resolution by
precedence (high → low):

1. **Repo** — `.cogito/skills/<name>/SKILL.md` in cwd and every ancestor
   directory up to repo root (monorepo support)
2. **User** — `~/.cogito/skills/<name>/SKILL.md`
3. **Plugin** — `<plugin>/skills/<name>/SKILL.md` (Plugin loader v0.2;
   skills get auto-namespaced `<plugin_id>:<name>`, so no conflicts
   with user/repo bare names)
4. **System** — cogito-bundled skills, feature-gated (optional, off by
   default for v0.1)

Higher scope's bare-name skill shadows lower scope's; plugin-namespaced
skills never conflict with bare-name skills by construction.

### 3. SKILL.md frontmatter schema

Required fields:
- `name` — skill identifier (kebab-case)
- `description` — short summary used in system prompt's
  "Available Skills" block

Optional fields (compatible with Claude Code):
- `disable-model-invocation: true` — only user slash can trigger
- `user-invocable: false` — only model sigil can trigger
- `version` — semver, recorded in `SkillActivated` event

### 4. Bundled scripts: **deferred (B-defer)**

`scripts/` directories may exist in skills but cogito **does not
execute them in v0.1**. The model can read script contents via
`read_file` and execute via `bash` (subject to existing tool
permissions). Auto-registering scripts as tools, sandbox model,
parameter schemas, and dynamic injection (Claude Code's `` !`cmd` ``)
are explicitly out of scope; see ADR-0023 placeholder.

### 5. Crate layout

New crate `cogito-skills` in the Hands layer:

```
cogito-skills/
  src/
    lib.rs            # SkillRegistry, SkillProvider trait
    discovery.rs      # scope-based filesystem scan
    metadata.rs       # SKILL.md frontmatter parser
    activation.rs     # sigil regex + matching
```

Trait `SkillProvider` (in `cogito-protocol`) is implemented by
`cogito-skills` and consumed by H04 / H06 / H11 via dependency
injection at Runtime build time. Brain (`cogito-core::harness`) sees
`dyn SkillProvider`, no direct import.

### 6. New event variant

```rust
EventPayload::SkillActivated {
    skill_name: String,    // bare or "<plugin_id>:<name>"
    source: SkillSource,   // Repo | User | Plugin { plugin_id } | System
    recorded_event_id: EventId,
}
```

Additive under ADR-0007 (no `SCHEMA_VERSION` bump).

## Consequences

**Easier**:
- Team members ship Skills as pure markdown + TOML; zero Rust required
- Model-portable activation (private vLLM / SGLang deployments work
  identically to Anthropic-hosted)
- Direct compatibility with `SKILL.md` files written for Claude Code or
  Codex (loader reads the same format)
- Event log records activation as a discrete event → resume / chaos
  oracles unchanged

**Harder**:
- Sigil disambiguation edge cases (shell `$VAR`, SQL `$1`, template
  `${name}`) require care — Sprint 7 addresses with guardrail (a) +
  system-prompt instruction (b)
- "Available Skills" block in system prompt consumes context budget;
  needs character cap per skill (Claude Code uses 1536 chars; cogito
  adopts similar bound, exact value locked in Sprint 7)

**Given up**:
- Native Anthropic skill activation (Claude Code's natural-language
  path) — sigil works on Anthropic too, but doesn't leverage any
  model-side training. Acceptable tradeoff for multi-model parity.
- Auto-script-execution (B-defer); revisitable in ADR-0023 if a
  consumer use case demands it.

## References

- Rebalance spec: [`docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md`](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md) §2.5–§2.6 + §4.1
- agentskills.io specification: https://agentskills.io/specification
- Codex skill loader source: `codex-rs/core-skills/` (Apache-2.0)

## Sprint 7 closure notes

Final values locked in during Sprint 7 implementation:

- **Sigil regex**: `\$([A-Za-z][A-Za-z0-9_:-]{0,63})` — first character must
  be an ASCII letter; subsequent characters allow letters, digits, `_`,
  `:`, `-`; total identifier length bounded at 64 characters. Rejects
  `$1` (SQL placeholder), `$_foo` (shell convention), `${name}` (template
  substitution).
- **Description cap**: 1024 characters per skill in the "Available Skills"
  system-prompt block. Skills with longer `description` fields are
  truncated at load time with a warning event.
- **Repo-root stop**: discovery walks upward from CWD and stops at the
  first directory containing `.git/` OR `cogito.toml`, or when reaching
  the filesystem root. `.skills/` directories found along the way are
  merged (closer-to-CWD wins on name collision).
- **Code-fence skip**: sigils inside fenced code blocks (` ``` ` /  `~~~`)
  and inline code spans (`` ` ``) are NOT expanded — answer to spec Q3 = A.
  Rationale: users routinely paste shell snippets with `$VAR` into chat
  and would not expect expansion inside a code fence.
- **ADR-0023 (bundled scripts) remains a placeholder.** Sprint 7 ships
  the markdown-only loader; auto-execution of `scripts/` in a skill is
  deferred until a concrete consumer use case demands it.
- Claude Code skills documentation: https://code.claude.com/docs/en/skills

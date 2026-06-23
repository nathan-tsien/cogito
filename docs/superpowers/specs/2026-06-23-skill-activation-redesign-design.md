# Skill activation redesign — tool-call primary channel + forcing instruction

Date: 2026-06-23
Status: Draft for review
Scope: Make skill activation reliable. Add an `activate_skill` tool as the
primary activation channel, strengthen the injected skill index with a
mandatory forcing instruction, and add lean index ordering/capping — while
keeping the existing sigil and slash channels as portability fallbacks and
leaving Brain (H01–H11) untouched.

Related: ADR-0020 (skill loader — this supersedes its §1 activation
mechanism), ADR-0029 (bundled-file path exposure), ADR-0033 (no custom
skill frontmatter descriptor — honored: no schema change), ADR-0004 (layer
map), ADR-0007 (additive events). New ADR to be filed: **ADR-0042**.

---

## 1. Problem

Today activation is **K5 (ADR-0020)**: the model emits a `$SkillName` sigil
in its prose, H06 detects it by regex, and `SkillInjector` injects the full
`SKILL.md` body on the next turn; users can also type `/skill <name>`. There
is **no tool channel** and the injected index (`## Available Skills`,
`crates/cogito-context/src/injector/skill.rs:194`) is a passive bullet list
with no instruction telling the model to use it.

This does not trigger reliably:

1. **Sigil-in-prose is out-of-distribution.** Models are RL-trained to call
   tools, not to emit magic tokens in free text. The natural phrasing
   ("I'll use the brainstorming skill") contains no `$`, so activation is
   silently missed.
2. **No forcing function.** The index has no imperative; nothing tells the
   model to scan it and activate a relevant skill.
3. **Sigil guardrails create false negatives.** Code-fence skipping, exact
   registered-name matching, and `disable-model-invocation` exclusion each
   narrow the activation window.
4. **No index hygiene.** `list()` dumps every skill unordered and uncapped,
   degrading signal-to-noise as the skill count grows.

Reference surveyed: NousResearch `hermes-agent` (`prompt_builder.py` +
`skill_*.py`). Its reliability comes from (a) tool-call activation
(`skill_view(name)` returns the skill body), (b) a `## Skills (mandatory)`
index with an explicit "you MUST load it" instruction, and (c) conditional
filtering / compact categories to keep the index relevant.

## 2. Decision summary

| Lever | Decision |
|---|---|
| Primary channel | New builtin tool **`activate_skill(name)`** that returns the skill body as its `ToolResult` (Hermes-style, in-turn). |
| Fallback channels | **sigil + slash kept, always on** — multi-model portability (ADR-0020's original driver) is preserved. Three channels run in parallel. |
| Forcing instruction | Injected index becomes `## Skills (mandatory)` with an imperative "you MUST call `activate_skill`" block (sigil mentioned as the no-tools fallback). |
| Index hygiene | Order by scope precedence, optional scope grouping, total count/char cap with a non-silent `log` on truncation. **Existing metadata only — no frontmatter change** (honors ADR-0033). |
| Body delivery | Tool returns body directly; persisted natively by `ToolResultRecorded`. Sigil/slash keep the next-turn `SkillInjector` path. |
| Event sourcing | Tool channel adds **no new event** — `ToolUseRecorded` + `ToolResultRecorded` are the activation record. `SkillActivated` stays sigil/slash-only. No `SkillActivationChannel` variant added. |
| Cross-channel dedup | `SkillInjector` also treats prior `activate_skill` tool calls as "already activated", so a skill loaded via tool is never re-injected via the sigil path. |
| Version / ADR | New **ADR-0042** supersedes ADR-0020 §1; implementation lands in **v0.3**. |

The Brain delta is **zero**. Every change is an additive Hands impl
(`activate_skill` builtin), an additive `cogito-protocol` helper (shared
body renderer), or a presentation change inside the context injector.

## 3. Components

### 3.1 `activate_skill` builtin tool (Hands)

A stateful `BuiltinTool` in `cogito-tools` (`builtins/activate_skill.rs`)
holding `Arc<dyn SkillProvider>`, registered via
`BuiltinToolProvider::builder().with_tool(Arc::new(ActivateSkill::new(provider)))`.
`BuiltinTool` is already registered as `Arc<dyn BuiltinTool>`
(`provider.rs:60`), so a stateful instance is the established pattern — no
new mechanism needed. `SkillProvider` is a `cogito-protocol` trait;
`cogito-tools` already depends on `cogito-protocol` and already reasons
about skills via `ExecCtx.skill_roots`, so this crosses no layer boundary.
The tool is sync (`BuiltinTool::invoke -> ToolResult`), which fits — it only
reads from the in-memory registry.

Descriptor:

```
name: "activate_skill"
description: "Load a skill's full instructions into the conversation.
  Call this before acting whenever a listed skill is relevant to the task.
  Returns the skill's complete SKILL.md body."
schema: { "type": "object",
          "properties": { "name": { "type": "string",
            "description": "The skill name exactly as listed in the Skills section." } },
          "required": ["name"], "additionalProperties": false }
execution_class: AlwaysSync
```

`invoke` behavior:
- Parse `{ name }`; bad args → `ToolResult::Error { kind: InvalidArgs, retryable: false }`.
- `provider.get_metadata(name)` miss → `ToolResult::Error { kind:
  InvocationFailed, retryable: false, message: "unknown skill '<name>';
  available: <comma-list>" }`. (Listing available names lets the model
  self-correct.)
- `disable_model_invocation == true` → `ToolResult::Error { kind:
  InvocationFailed, retryable: false, message: "skill '<name>' is
  user-invocable only; ask the user to run /skill <name>" }`. Mirrors the
  sigil channel honoring the same flag.
- Otherwise `provider.get(name)` → `ToolResult::text(render_skill_block(&content))`
  using the shared renderer (§3.3), so the bytes match the sigil/slash path.
- MUST NOT panic (trait contract + inviolable rule 5).

### 3.2 Forcing instruction + index hygiene (context injector)

`build_registry_block` (`crates/cogito-context/src/injector/skill.rs:194`)
changes from a passive list to:

```
## Skills (mandatory)
Before responding, scan the skills below. If any skill is relevant to the
user's task — even partially — you MUST load it first by calling the
`activate_skill` tool with its name. (If you cannot call tools, write
`$<name>` instead.) Loading injects the skill's full instructions.

<scope-grouped, precedence-ordered, capped list of `- <name>: <description>`>
```

Index hygiene (existing metadata only):
- **Order** by scope precedence Repo > User > Plugin > System (most-local
  first). Within a scope, preserve discovery order.
- **Grouping** (optional, behind a small bool, default on): a one-line scope
  sub-header (`### From this repository` / `### User` / `### Plugins` /
  `### Built-in`) above each non-empty group. Scope substitutes for the
  category metadata cogito does not have.
- **Caps**: keep the existing per-description 1024-char cap; add a max
  listed-skills count and a max total-block-char budget (exact numbers in
  §6). On truncation, emit `log` naming how many skills were dropped — never
  a silent cap.
- All hygiene lives in the injector (presentation). `SkillProvider::list()`
  contract is unchanged (additive principle).

### 3.3 Shared body renderer (`cogito-protocol`)

Extract the per-skill body rendering currently inlined in
`build_body_blocks` into a free function in `cogito-protocol::skill`:

```rust
/// Render a skill's body block (ADR-0029 `<skill name=… source=… root=…>`
/// wrapper + root-resolution hint) for delivery to the model. Shared by the
/// context injector (sigil/slash channel) and the `activate_skill` tool so
/// all channels deliver byte-identical content.
pub fn render_skill_block(content: &SkillContent) -> String
```

`SkillContent` and `SkillSource` already live in `cogito-protocol`, so the
function has no external dependency. `build_body_blocks` becomes a thin loop
over `render_skill_block`. Additive protocol change; no signature breaks.

Note: the existing ADR-0029 `TODO` about unescaped `root="…"` interpolation
carries over verbatim into `render_skill_block` (still operator-trusted in
v0.3; tenant-controlled escaping deferred to the SaaS Phase 3).

### 3.4 Cross-channel dedup (context injector)

`collect_prior_activations` (`skill.rs:184`) currently scans `SkillActivated`
events. Extend it to also scan `ToolUseRecorded` where `tool_name ==
"activate_skill"`, take `args.name`, and confirm success via the correlated
`ToolResultRecorded` (same `call_id`, `result` is `Output` not `Error`),
adding that name to the "already activated" set. Effect: if the model both
calls `activate_skill(x)` and writes `$x`,
the sigil path does **not** re-inject `x`'s body on the next turn. The tool
channel never routes through `SkillInjector` body injection — the body is
already in the tool result.

## 4. Data flow

Tool channel (primary):
```
model emits tool_use activate_skill{name} (H02 records ToolUseRecorded)
  → H05/H07/H08 dispatch → ActivateSkill.invoke
  → ToolResult::text(render_skill_block(body))
  → H02 records ToolResultRecorded (body persisted)
  → model sees body in the same agent loop, proceeds
  → (next assembly) SkillInjector sees the activate_skill tool_use, marks
     `name` activated, does NOT re-inject
```

Sigil/slash channel (fallback, unchanged):
```
$name in prose (H06) | /skill name (TurnStarted.activate_skills)
  → SkillInjector records SkillActivated + injects render_skill_block(body)
     as system-prompt suffix on the next turn
```

Resume: the tool channel rebuilds for free — the body lives in the persisted
`ToolResultRecorded`. The sigil/slash channel keeps its existing
event-sourced rebuild.

## 5. Layer / invariant check (ADR-0004)

- `activate_skill` is Hands; implements the `ToolProvider`/`BuiltinTool`
  contract; wired by the Runtime layer into `CompositeToolProvider`. Brain
  holds only `dyn ToolProvider`.
- Only `cogito-protocol` addition is the additive `render_skill_block` fn.
- No Brain component edited; H01 stays the only coordinator; H02–H11 never
  call each other.
- No `SCHEMA_VERSION` bump: no new event variant, no new enum variant.
- State rebuildable from the log: yes (tool result persisted; sigil/slash
  unchanged).

## 6. Constants (proposed, lockable in plan)

- Max listed skills in the index: **50** (then truncate + log).
- Max total index-block chars: **8 KiB** (then truncate + log).
- Per-description cap: **1024** (unchanged).
- Scope grouping default: **on**.

## 7. Testing

- Unit (`cogito-tools`): `activate_skill` returns rendered body for a known
  skill; `InvalidArgs` on bad args; `InvocationFailed` (with available-list)
  on unknown name; `InvocationFailed` on `disable-model-invocation` skill;
  never panics.
- Injector unit: index contains the `## Skills (mandatory)` instruction;
  scope-precedence ordering; grouping headers; truncation past the cap emits
  a `log` and drops the documented count.
- Equivalence contract test: `render_skill_block` output delivered via the
  tool == body injected via the sigil path, byte-for-byte, for the same
  `SkillContent` (incl. ADR-0029 root header).
- Cross-channel dedup: model calls `activate_skill(x)` and also writes `$x`
  → exactly one body copy reaches the model; no second injection next turn.
- Multi-model: mock model variant A emits a tool call, variant B emits a
  sigil — both deliver the body.
- Resume-chaos: new scenario `tool_activate_skill_then_use` —
  crash-injection around the `activate_skill` tool boundary; resume rebuilds
  the body from `ToolResultRecorded` and the subsequent tool use proceeds.

## 8. Out of scope (YAGNI)

- No new frontmatter fields; no tool/platform-based filtering (ADR-0033).
- No `SkillActivated` event for the tool channel; no telemetry-uniformity
  layer over the two record shapes.
- No category metadata — scope is the only grouping dimension.
- No change to bundled-script execution (ADR-0023) or the `Workspace`/
  artifact pillars (separate tracks).

## 9. ADR-0042 outline

Title: "Skill activation — tool-call primary channel". Supersedes ADR-0020
§1 (K5). Records: the reliability problem with sigil-in-prose, the reversal
of ADR-0020's "no `load_skill` tool" decision (its premise — "no platform
uses tool-call activation; a round trip is pure overhead" — is now false:
Claude Code ships a `Skill` tool and Hermes uses `skill_view`, and the
round-trip cost is dominated by the cost of a missed activation), the
three-channel parallel model, the no-new-event choice, and the
existing-metadata-only filtering. ADR-0020 gets a status note pointing here.

# H10 · Strategy Selector

## What is a strategy

A strategy is a named, declarative "agent mode." It bundles *which
model, which persona, which tools, which context policy* for one
kind of work. Consumers ship N strategies — `coder`, `planner`,
`reviewer`, `critic` — and `--strategy <name>` selects the mode.
Same Brain, same Boundary, different *behavior contract*.

Strategies are not configuration of cogito. `cogito.toml` is
"where is the model and how do I reach it"; strategies are "what do
I tell the model to do." Strategies reference providers from
`cogito.toml` by name; they never embed credentials.

See [`ADR-0026`](../adr/0026-strategy-registry.md) for the full
rationale.

> **Status**: Implemented · `HarnessStrategy::default_with_model(model_id)` factory + field set
> (name / system_prompt / allowed_tools / tool_order / model_params / max_turns) in Sprint 2;
> markdown+frontmatter `StrategyRegistry` via the `cogito-strategy` crate in Sprint 9a
> (ADR-0026, which supersedes the originally-planned YAML loader). `crates/cogito-core/src/harness/strategy.rs`

## Role in Harness

Given the model identity and the turn's task context, produce a
`HarnessStrategy` value that the rest of Brain reads. **Produces a value;
calls no other component.** H10 is the only fully-passive component in
Brain.

H10 runs **first** in the prompt-build phase (`Init → ContextManaged`), at
the very entry of every turn. Its output is consumed downstream by:

- **H11 Context Manage** — reads `length_budget`, summarization preference
  (per ADR-0008, pending), and tool-filter starting point.
- **H04 Prompt Composer** — reads `system_prompt`, `length_budget`,
  `model_params`.
- **H05 Tool Surface Builder** — reads `allowed_tools`, `tool_order`.
- **H09 Hook Pipeline** — reads `hooks` to know which hooks to fire.

H10 itself is unaware of H11/H04/H05/H09 — the dependency arrow is
one-way (H01 calls H10, then hands the value around). See
`docs/components/H01-turn-driver.md` §"Init → ContextManaged → PromptBuilt
sequence" for the canonical walkthrough.

## Interface (design level)

- Sprint 2: `HarnessStrategy::default_with_model(model_id: impl Into<String>) -> HarnessStrategy` — a free constructor; no registry, no per-task selection.
- Sprint 6: `select(model_id: &str, task: &TaskContext, registry: &StrategyRegistry) -> HarnessStrategy` — the full selector backed by a YAML registry.
- `HarnessStrategy` is a plain value type in `cogito-protocol::strategy`. **v0.1 Mid field set** (locked Sprint 2):
  - `name: String` — strategy identifier, written into `TurnStarted` event
  - `system_prompt: String`
  - `allowed_tools: ToolFilter` — `All` or `Allow(Vec<String>)`
  - `tool_order: Option<Vec<String>>` — explicit tool ordering for prompt-cache stability; `None` falls back to alphabetical
  - `model_params: ModelParams` — model + temperature + max_tokens + top_p + stop_sequences
  - `max_turns: u32` — agent-loop safety budget; default 16

  Reserved for later versions (intentionally **not** in v0.1):
  - `length_budget: usize` — Sprint 8 / H11 ADR-0008
  - `hooks: Vec<HookConfig>` — Sprint 7
  - `allow_async_tools: bool` — Sprint 5 (with JobManager)
  - `parallel_dispatch: bool` — 0.x option, off in v0.1

The Sprint 2 factory and the Sprint 6 selector are both **pure**: same inputs → same strategy.

## Dependencies

**Calls (out)**: None.

**Called by**: H01 Turn Driver, on entry (at the start of the `Init` state, before `Init → ContextManaged` transition). The returned value is **cached for
the duration of the turn** and consumed (read-only) by H11, H04, H05, H09.

## Critical invariants

1. **No side effects.** No I/O, no clock, no random. Configuration is loaded once at Runtime startup (`StrategyRegistry::load_from_dir`).
2. **Never calls any other Brain component.** The dependency arrow goes one way: H10 is called by H01, and its output is read by H11/H04/H05/H09. H10 does not know they exist.
3. **Strategy is immutable for the duration of a turn.** Even if a hook returns `Modify(strategy_override)`, the modification is recorded as an event and applied on the *next* re-entry (e.g., for the next iteration of the prompt-model-tool loop within the same turn).
4. **Selection is deterministic.** No "if the model has been slow lately, pick a different one" logic; that's the consumer's deployment concern.

## v0.1 Sprint 2 scope

- `HarnessStrategy::default_with_model(model_id)` is the **only** way to obtain a strategy. CLI surfaces it via `--model <id> [--system "<prompt>"]`. No YAML, no per-task selection, no registry — that machinery is intentionally deferred.
- Defaults baked into the factory:
  - `name = "default"`
  - `system_prompt = "You are a helpful assistant."` (overridable by CLI `--system`)
  - `allowed_tools = ToolFilter::All`
  - `tool_order = None`
  - `model_params = { model: <model_id>, max_tokens: 4096, temperature: None, top_p: None, stop_sequences: [] }` — `temperature: None` defers to the provider default; reasoning models (Kimi K2, o-series, DeepSeek-R1) reject any other value
  - `max_turns = 16`

## v0.x Sprint 6 scope (designed, not implemented)

- Strategies are loaded from YAML files at runtime startup (`strategies/*.yaml`)
- Selection key: `model_id` → strategy file (one-to-one mapping in 0.x)
- `task` (e.g., "code-review" vs "chat" vs "tool-heavy") is reserved in the API but ignored in 0.x (defaults to a single per-model strategy)
- Strategy registry is in-memory; no hot reload (process restart to pick up YAML edits)

> **2026-05-27 update (ADR-0026 / Sprint 9a):** Strategy files are
> markdown with YAML frontmatter (Skills convention). The `name`
> frontmatter field is REQUIRED and MUST match the filename
> basename. The 2026-05-21 note saying `name:` would be dropped is
> superseded — we keep `name:` and validate it. Scope precedence
> (Repo > User) replaces the single `runtime.strategies_dir` model;
> `strategies_dir`, when set, overrides the Repo root only.

## Example strategy file (Sprint 9a+)

Strategies live as markdown files with YAML frontmatter under
`.cogito/strategies/<name>.md`. The filename basename must match the
`name:` frontmatter field. The body of the markdown is the system
prompt.

```markdown
---
name: coder                          # required, must match filename basename
description: >                       # optional, human-only; surfaced by `--list`
  Coding tasks. Read first, write second. Low temperature for precision.

# Optional provider/model binding. If absent, CLI --model and
# cogito.toml [default_provider] resolve them. CLI --model wins.
provider: anthropic-default          # references cogito.toml [providers.anthropic-default]
model: claude-opus-4-7

# Tool filter. null/omit = ToolFilter::All (every tool the provider lists).
allowed_tools:
  - read_file
  - run_tests

# Optional explicit ordering for prompt-cache stability.
tool_order:
  - read_file
  - run_tests

# Safety budget; default 16 if omitted.
max_turns: 50

# Sampling knobs. Overlay on top of provider-level model_params.
# Strategy keys win on conflict.
model_params:
  temperature: 0.3
  max_tokens: 4096

# Context-management pipeline. Deserializes directly into
# cogito_context::ContextConfig. Default = all-no-op.
context:
  compactor: { kind: truncate, max_tokens: 100000 }
---

You are a precise software engineer.
Always read before writing. Run tests after every change.
...
```

## Open design questions

- Per-task selection: do we want `(model_id, task)` → strategy, or just `model_id` → strategy with task-conditioned hooks within? Initial v0.1: `model_id` only; `task` field reserved for 0.x.
- Strategy inheritance / composition: do strategies need a `extends: <base>` field? Initial answer: no — keep YAML flat; if duplication appears, refactor then.
- Hot reload: nice-to-have, but only if a real operational need surfaces. Default: process restart.

## Testing strategy

- **Unit**: YAML parsing for valid and invalid strategies; selection by model_id; missing strategy file → clear error.
- **Snapshot** (insta): canonical YAML → canonical `HarnessStrategy` value.

## References

- ARCHITECTURE.md §"The 11-component Brain" (H10 row)
- (no dedicated ADR — strategy file format is implementation detail, not architectural)

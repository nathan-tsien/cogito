# H10 · Strategy Selector

> **Status**: 🚧 Sprint 2 lands the `HarnessStrategy::default_with_model(model_id)` factory + Mid field set
> (name / system_prompt / allowed_tools / tool_order / model_params / max_turns).
> YAML loader + multi-strategy registry + per-task selection remain Sprint 5.

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
- Sprint 5: `select(model_id: &str, task: &TaskContext, registry: &StrategyRegistry) -> HarnessStrategy` — the full selector backed by a YAML registry.
- `HarnessStrategy` is a plain value type in `cogito-protocol::strategy`. **v0.1 Mid field set** (locked Sprint 2):
  - `name: String` — strategy identifier, written into `TurnStarted` event
  - `system_prompt: String`
  - `allowed_tools: ToolFilter` — `All` or `Allow(Vec<String>)`
  - `tool_order: Option<Vec<String>>` — explicit tool ordering for prompt-cache stability; `None` falls back to alphabetical
  - `model_params: ModelParams` — model + temperature + max_tokens + top_p + stop_sequences
  - `max_turns: u32` — agent-loop safety budget; default 16

  Reserved for later versions (intentionally **not** in v0.1):
  - `length_budget: usize` — Sprint 7 / H11 ADR-0008
  - `hooks: Vec<HookConfig>` — Sprint 6
  - `allow_async_tools: bool` — Sprint 4 (with JobManager)
  - `parallel_dispatch: bool` — 0.x option, off in v0.1

The Sprint 2 factory and the Sprint 5 selector are both **pure**: same inputs → same strategy.

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
  - `model_params = { model: <model_id>, max_tokens: 4096, temperature: Some(0.7), top_p: None, stop_sequences: [] }`
  - `max_turns = 16`

## v0.x Sprint 5 scope (designed, not implemented)

- Strategies are loaded from YAML files at runtime startup (`strategies/*.yaml`)
- Selection key: `model_id` → strategy file (one-to-one mapping in 0.x)
- `task` (e.g., "code-review" vs "chat" vs "tool-heavy") is reserved in the API but ignored in 0.x (defaults to a single per-model strategy)
- Strategy registry is in-memory; no hot reload (process restart to pick up YAML edits)

> **2026-05-21 update (ADR-0017 §9):** Strategy file basename
> (without `.yaml`) is the canonical strategy name; the YAML body
> drops `name:` and `applicable_models:` fields. The two existing
> draft files (`strategies/claude-opus.yaml`, `strategies/gpt-4.yaml`)
> will be rewritten when Sprint 5 lands the loader.

## Example strategy YAML (Sprint 5+)

```yaml
# strategies/claude-sonnet-default.yaml
model_id: claude-sonnet-4-6
system_prompt: |
  You are a helpful assistant for software engineering tasks.
allowed_tools: ["read_file", "grep", "shell"]
tool_order: ["read_file", "grep", "shell"]
model_params:
  temperature: 0.7
  max_tokens: 4096
max_turns: 32
# Sprint 6+ fields (when H09 hooks land):
# hooks:
#   - name: "sensitive_content_filter"
#     config: { severity: "high" }
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

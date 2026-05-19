# H10 · Strategy Selector

> **Status**: 🚧 Not implemented · Sprint 5

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

- `select(model_id: &str, task: &TaskContext, registry: &StrategyRegistry) -> HarnessStrategy`
- `HarnessStrategy` is a plain value type containing:
  - `system_prompt: String`
  - `allowed_tools: ToolFilter` (`All` or `Names(Vec<String>)`)
  - `tool_order: Option<Vec<String>>` — for prompt cache stability
  - `length_budget: usize` — max prompt tokens (consumed by H04)
  - `model_params: ModelParams` — temperature, max_tokens, top_p, etc.
  - `hooks: Vec<HookConfig>` — which hooks run, with what config (consumed by H09)
  - `parallel_dispatch: bool` (0.x — false in v0.1)

The function is **pure**: same `(model_id, task, registry)` → same strategy.

## Dependencies

**Calls (out)**: None.

**Called by**: H01 Turn Driver, on entry (at the start of the `Init` state, before `Init → ContextManaged` transition). The returned value is **cached for
the duration of the turn** and consumed (read-only) by H04, H05, H09.

## Critical invariants

1. **No side effects.** No I/O, no clock, no random. Configuration is loaded once at Runtime startup (`StrategyRegistry::load_from_dir`).
2. **Never calls any other Brain component.** The dependency arrow goes one way: H10 is called by H01, and its output is read by H11/H04/H05/H09. H10 does not know they exist.
3. **Strategy is immutable for the duration of a turn.** Even if a hook returns `Modify(strategy_override)`, the modification is recorded as an event and applied on the *next* re-entry (e.g., for the next iteration of the prompt-model-tool loop within the same turn).
4. **Selection is deterministic.** No "if the model has been slow lately, pick a different one" logic; that's the consumer's deployment concern.

## v0.1 scope

- Strategies are loaded from YAML files at runtime startup (`strategies/*.yaml`)
- Selection key: `model_id` → strategy file (one-to-one mapping in v0.1)
- `task` (e.g., "code-review" vs "chat" vs "tool-heavy") is reserved in the API but ignored in v0.1 (defaults to a single per-model strategy)
- Strategy registry is in-memory; no hot reload (process restart to pick up YAML edits)

## Example strategy YAML

```yaml
# strategies/claude-sonnet-default.yaml
model_id: claude-sonnet-4-6
system_prompt: |
  You are a helpful assistant for software engineering tasks.
allowed_tools: ["read_file", "grep", "shell"]
tool_order: ["read_file", "grep", "shell"]
length_budget: 100000
model_params:
  temperature: 0.7
  max_tokens: 4096
hooks:
  - name: "sensitive_content_filter"
    config: { severity: "high" }
parallel_dispatch: false
```

## Open design questions

- Per-task selection: do we want `(model_id, task)` → strategy, or just `model_id` → strategy with task-conditioned hooks within? Initial v0.1: `model_id` only; `task` field reserved for 0.x.
- Strategy inheritance / composition: do strategies need a `extends: <base>` field? Initial answer: no — keep YAML flat; if duplication appears, refactor then.
- Hot reload: nice-to-have, but only if a real operational need surfaces. Default: process restart.

## Testing strategy

- **Unit**: YAML parsing for valid and invalid strategies; selection by model_id; missing strategy file → clear error.
- **Snapshot** (insta): canonical YAML → canonical `HarnessStrategy` value.

## References

- ARCHITECTURE.md §"The 10-component Brain" (H10 row)
- (no dedicated ADR — strategy file format is implementation detail, not architectural)

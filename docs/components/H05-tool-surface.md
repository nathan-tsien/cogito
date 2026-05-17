# H05 · Tool Surface Builder

> **Status**: 🚧 Not implemented · Sprint 2

## Role in Harness

Decide which tools the LLM sees **this turn**. The decision is Brain-side
(it's reasoning about strategy + state); the execution stays in Hands. H05
queries the injected `ToolProvider` for the full catalog and filters per
the active `HarnessStrategy`.

## Interface (design level)

- `surface(strategy: &HarnessStrategy, provider: &dyn ToolProvider) -> Vec<ToolDescriptor>`
- Output: the filtered subset of `ToolDescriptor`s that will appear in this
  turn's `ModelInput.tools` (consumed by H04).

The function is **synchronous** (no I/O — `provider.list()` is documented
to be a fast in-memory operation) and **deterministic**.

## Dependencies

**Calls (out)**:
- `ToolProvider::list()` — read-only, idempotent, expected to be fast.

**Called by**: H01 Turn Driver, at `Init → PromptBuilt`.

## Critical invariants

1. **Deterministic.** Same strategy + same provider list → same surface (same set, same order).
2. **No side effects.** `surface()` is a pure decision; it does not warm caches, ping providers, or persist anything.
3. **Stable order.** Tools in the output are sorted by name (or by the strategy's `tool_order` if given). Affects prompt cache hit rate; should not drift across calls.
4. **No invocation.** H05 does not call `ToolProvider::invoke`. That is H08's job alone.

## v0.1 scope

- Strategy declares an allow-list (`tools: ["read_file", "grep"]`) or a wildcard (`tools: "*"`)
- H05 returns `provider.list()` filtered by the allow-list, sorted by name
- No dynamic per-turn filtering (e.g., "drop write tools if this is a read-only session"). Such logic, if needed, lives in a hook that runs `pre_prompt` and rejects the turn, not in H05.
- No tool synthesis (we don't make up new tools per turn).

## Composite providers

The consumer is expected to assemble a single `Arc<dyn ToolProvider>` to
hand to Runtime. When the consumer wants builtin tools + MCP tools +
custom tools, they use `CompositeToolProvider` (utility in `cogito-tools`):

```text
CompositeToolProvider {
    providers: [BuiltinToolProvider, McpToolProvider, ConsumerCustomProvider],
    naming: Strict | Prefixed("provider_alias"),
}
```

H05 sees only the composite — it doesn't know or care that the catalog
came from multiple sources.

## Open design questions

- Should H05 expose its filter logic to a hook (`pre_prompt`) so policies can inject per-turn modifications? Initial answer: no — keep H05 pure; if a hook wants to modify the surface, it goes through `HookDecision::Modify(strategy)` and the surface is rebuilt deterministically. Strategy is the only knob.
- Caching: if `provider.list()` is expensive (e.g., MCP server roundtrip), should H05 cache? Initial answer: caching is the provider's responsibility (MCP provider should cache server-side tool lists itself); H05 stays stateless.

## Testing strategy

- **Unit**: empty allow-list, wildcard, specific names, names that don't exist in the catalog (silently dropped).
- **Property**: result order is stable across multiple calls; result set is exactly the intersection of catalog and allow-list.
- **Integration**: composite provider with overlapping tool names under both `Strict` and `Prefixed` naming policies.

## References

- ARCHITECTURE.md §"Hands layer internal structure"
- §"Tool execution classes" (H05 doesn't care about class — it's per-tool metadata, surfaced via `ToolDescriptor`)

# H05 · Tool Surface Builder

> **Status**: Implemented · Sprint 2 (strategy-filtered list); honors per-turn `ToolFilterOverridden` (Sprint 6). `crates/cogito-core/src/harness/tool_surface.rs`

## Role in Harness

Decide which tools the LLM sees **this turn**. The decision is Brain-side
(it's reasoning about strategy + state); the execution stays in Hands. H05
queries the injected `ToolProvider` for the full catalog and filters per
the active `HarnessStrategy`.

H05 is the **strategy-static** half of tool selection. The **dynamic** half
— "drop the `write_file` tool because we're in a read-only review subtask",
"add a context-derived tool just for this turn" — belongs to **H11 Context
Manage**, which runs before H05 in the `Init → ContextManaged → PromptBuilt`
sequence. H11 may write a `ToolFilterOverridden` event; H05 then intersects
its strategy filter with that override (override only *narrows*; H05 never
expands beyond what the strategy authorizes). See
`docs/components/H01-turn-driver.md` §"Init → ContextManaged → PromptBuilt
sequence" for the canonical walkthrough.

## Interface (design level)

- `surface(strategy: &HarnessStrategy, provider: &dyn ToolProvider) -> Vec<ToolDescriptor>`
- Output: the filtered subset of `ToolDescriptor`s that will appear in this
  turn's `ModelInput.tools` (consumed by H04).

The function is **synchronous** (no I/O — `provider.list()` is documented
to be a fast in-memory operation) and **deterministic**.

## Dependencies

**Calls (out)**:
- `ToolProvider::list()` — read-only, idempotent, expected to be fast.

**Called by**: H01 Turn Driver, at `ContextManaged → PromptBuilt`. (Prior to the ADR-0006 amendment of 2026-05-19, this fired from `Init → PromptBuilt`; the new `ContextManaged` state runs H11 first so H05 can read any `ToolFilterOverridden` event H11 wrote.)

## Critical invariants

1. **Deterministic.** Same strategy + same provider list → same surface (same set, same order).
2. **No side effects.** `surface()` is a pure decision; it does not warm caches, ping providers, or persist anything.
3. **Stable order.** Tools in the output are sorted by name (or by the strategy's `tool_order` if given). Affects prompt cache hit rate; should not drift across calls.
4. **No invocation.** H05 does not call `ToolProvider::invoke`. That is H08's job alone.

## v0.1 scope

- Strategy declares `allowed_tools: ToolFilter` — `ToolFilter::All` (wildcard) or `ToolFilter::Allow(Vec<String>)` (explicit allow-list)
- Strategy may set `tool_order: Option<Vec<String>>` — Sprint 2 honors this; tools named in `tool_order` come first in the order given, remaining tools follow alphabetically. `None` falls back to all-alphabetical (prompt-cache stable)
- H05 returns `provider.list()` filtered by the allow-list and sorted per the rule above
- No dynamic per-turn filtering by H05 itself. Dynamic narrowing is H11's responsibility (writes a `ToolFilterOverridden` event; H05 intersects with its strategy filter). A `pre_prompt` hook may also `Reject` an entire turn whose tool surface is unsafe.
- No tool synthesis (we don't make up new tools per turn).
- Dynamic narrowing is live since Sprint 6 (ADR-0008 Accepted): H11's `ToolFilterOverrider` writes a `ToolFilterOverridden` event during `ContextManaged`, and H05 honors it (see "Sprint 6: ToolFilterOverridden integration" below). When H11 uses the default `NoneOverrider`, the event carries `Inherit` (a no-op), so H05's surface matches strategy alone.

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

## Sprint 6: ToolFilterOverridden integration

H05 reads the latest `ToolFilterOverridden` event for the current turn from the event log (written by H11's `ToolFilterOverrider` during the preceding `ContextManaged` state). It applies the mode on top of `strategy.allowed_tools`:

- `Inherit` — use `strategy.allowed_tools` unchanged (no-op; what `NoneOverrider` writes every turn).
- `Intersect { tools }` — keep only tools that appear in both `strategy.allowed_tools` and `tools`.
- `Replace { tools }` — use `tools` as the full surface, ignoring `strategy.allowed_tools`.

Override only **narrows or replaces**; H05 never expands beyond what the strategy authorizes unless `Replace` is used explicitly by a plugin or subagent. If no `ToolFilterOverridden` event exists for the current turn, H05 falls back to strategy alone (backward-compatible with pre-Sprint-6 sessions).

See ADR-0008 §"Event surface" for the `ToolFilterOverridden` event shape and field semantics.

### Observability fields (Sprint 4)

Each surface build emits `tracing::info!` on target `h05.tool_surface`
with structured fields: `mcp.tool_count`, `mcp.tool_desc_total_bytes`,
`builtin.tool_count`. See [ADR-0018 §7](../adr/0018-mcp-integration.md).

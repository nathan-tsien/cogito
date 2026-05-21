# H07 · Tool Call Resolver

> **Status**: 🚧 In progress · Sprint 2

## Role in Harness

Parse each `ToolUseCompleted` event (emitted by H06 from the model
stream) into a structured `ToolInvocation`, validate the args against
the tool's JSON Schema, and surface failures as **structured results
that the LLM can act on** — never as panics or propagated `Err`s.

## Interface (design level)

- `resolve(use_event: &ToolUseEmitted, surface: &[ToolDescriptor]) -> ResolvedCall`
- `ToolInvocation` and `ResolvedCall` are **harness-internal** value
  types (not in `cogito-protocol`): they never persist, never cross a
  language boundary, and are consumed exclusively by H08. They live in
  `cogito-core::harness::tool_resolver`.
- `ResolvedCall::Ok(ToolInvocation { call_id, name, args })`
- `ResolvedCall::Error(ToolResult)` — wraps a `ToolResult::Error` value
  using the existing protocol types (`ToolErrorKind::InvalidArgs` for
  schema mismatch or malformed JSON; a fresh local kind for unknown tool
  is mapped onto `ToolErrorKind::InvocationFailed` with a descriptive
  message)

## Dependencies

**Calls (out)**: None directly. Reads `ToolDescriptor.input_schema` from the surface passed by H01.

**Called by**: H01 Turn Driver, at `ModelCompleted` (once per emitted tool call).

## Critical invariants

1. **No panics.** Every parse / validation failure returns `ResolvedCall::Error` with a structured kind. Brain never sees `Err` from this path.
2. **No side effects.** Resolution is pure validation. No store writes, no provider calls.
3. **`UnknownTool` and `SchemaMismatch` are not Brain-fatal.** They turn into a `ToolResult::Error` event that is fed back to the LLM, which usually retries with corrected input. The turn does **not** transition to `Failed`.
4. **Schema validation is strict.** v0.1 uses `jsonschema` (per workspace deps) in *strict mode* (no extra properties unless the schema explicitly allows). Tool authors must opt in to extra-properties tolerance.
5. **`call_id` is preserved.** The model assigns a `call_id` per tool_use block; H07 carries it through so H08 results can be correlated back.

## v0.1 scope

- JSON Schema validation via `jsonschema` crate
- No type coercion (no "the schema says number but the model sent a numeric string, so we'll parse it"). Strict.
- No retry / repair logic (the LLM is responsible for fixing its own args based on the error message)

## Error message shape

The `message` field of `ToolResult::Error` is designed for an LLM reader, not
a human. Recommended template:

- `UnknownTool`: `"tool `<name>` is not available this turn. available: [a, b, c]."`
- `SchemaMismatch`: `"args for `<name>` failed validation: <jsonschema error path>: <expected>, got: <actual>"`
- `MalformedJson`: `"args for `<name>` were not valid JSON: <serde error>"`

These messages get round-tripped to the LLM via the tool result message;
clarity here directly improves recovery rate.

## Open design questions

- Should H07 *also* validate tool *output* against an output schema? Initial answer: no for v0.1 (output schemas are optional in MCP and unspecified in Anthropic/OpenAI tool definitions). 0.2 may add opt-in output validation for tool authors who want it.
- Should H07 reject duplicate `call_id` within one model response? Initial answer: yes — return `MalformedJson` with a clear message; this would be a model bug.

## Testing strategy

- **Unit**: valid args, args missing required field, args with unexpected field, args with wrong type, malformed JSON, unknown tool name, unknown tool when wildcard surface is used (should still be valid since wildcard matches).
- **Property** (proptest): arbitrary JSON values + a schema produce either `Ok` or a structured `Error` — never panic.
- **Snapshot**: standard error messages for each kind are stable strings (LLMs are sensitive to message phrasing).

## References

- ARCHITECTURE.md §"Turn state machine" (ModelCompleted)
- ARCHITECTURE.md §"Hands layer internal structure" (`ToolDescriptor.input_schema` source)
- AGENTS.md §"Inviolable design principles" #5 (Tool failures are structured errors, not panics)

### MCP-sourced tool schemas (Sprint 4)

Tool schemas from MCP servers (`mcp__<server>__<tool>` tools) are
forwarded verbatim from `rmcp::model::Tool::input_schema` into the
`ToolDescriptor::schema`. H07 applies its standard JSON Schema
validation; no MCP-specific path. See
[ADR-0018 §6](../adr/0018-mcp-integration.md).

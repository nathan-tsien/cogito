# H04 · Prompt Composer

> **Status**: 🚧 Not implemented · Sprint 2

## Role in Harness

Assemble the next `ModelInput` that gets handed to the `ModelGateway`. Reads
events from the session log, the current `HarnessStrategy` (from H10), and
the per-turn tool surface (from H05), and produces a fully-formed input.

## Interface (design level)

- `compose(history: &[ConversationEvent], strategy: &HarnessStrategy, surface: &[ToolDescriptor]) -> ModelInput`
- `ModelInput` is a value type in `cogito-protocol`. It carries:
  - `system: String` (the system message, from strategy)
  - `messages: Vec<Message>` (the dialogue history in model-shaped form: user / assistant / tool_result turns)
  - `tools: Vec<ToolSchema>` (the surfaced tools' name + JSON Schema, in stable order)
  - `params: ModelParams` (temperature, max_tokens, etc., from strategy)

The function is **synchronous** and **deterministic**: same inputs always
produce the same `ModelInput`.

## Dependencies

**Calls (out)**: None. Reads passive inputs only.

**Called by**: H01 Turn Driver, at `Init → PromptBuilt` (and re-called on re-resume from `PromptBuilt`).

## Critical invariants

1. **Deterministic.** Same `history` + `strategy` + `surface` → byte-identical `ModelInput`.
2. **No I/O.** No file reads, no network, no `Instant::now()`.
3. **Length-budgeted.** Respects `strategy.length_budget` (max tokens of prompt + history). v0.1 truncates oldest-first when over budget; summarization is a 0.x option behind a strategy flag.
4. **Stable tool ordering.** Tools appear in `ModelInput.tools` sorted by name (or by an explicit `strategy.tool_order` if provided). Order matters for prompt-cache hit rates.
5. **History reconstruction is event-driven.** The dialogue history is rebuilt by iterating the event log and projecting events into `Message` shapes — never read from a separate "messages" table.

## v0.1 scope

- Single prompt template per strategy: `{system}` + flat conversation history + tool schemas verbatim
- Truncation policy: oldest-first message drop until under `strategy.length_budget`
- No summarization, no semantic compression
- No "context pin" mechanism (any pins are normal user / tool_result messages in the log)

## History projection

The function walks events in seq order and projects to `Message`s:

| Event | Projects to |
|---|---|
| `UserMessageAdded { text }` | `Message::User(text)` |
| `ModelCallCompleted { text, tool_calls }` | `Message::Assistant { text, tool_calls }` |
| `ToolResultRecorded { call_id, result }` | `Message::ToolResult { call_id, result }` |
| `HookModified { ... }` | (no projection — informational only) |
| State transitions (`TurnStarted`, `PromptComposed`, ...) | (no projection) |

Events that aren't part of the model-visible dialogue (control events,
hook events, redaction markers) are skipped during projection.

## Open design questions

- Truncation strategy at 0.2: keep most recent N, drop middle? Summarize middle? Initial v0.1: drop oldest, simple.
- Whether to include partial `TextDelta` events from a crashed turn in projection: initial answer: no — only sealed `ModelCallCompleted` events project to history.

## Testing strategy

- **Unit**: empty history, single-turn history, multi-turn history, history with tool results, history over length budget.
- **Snapshot** (insta): canonical event log → canonical `ModelInput` JSON, locked.
- **Property**: composition is idempotent under reordering of independent events (e.g., reordering `HookModified` events doesn't change the resulting `ModelInput` — they don't project).

## References

- ARCHITECTURE.md §"Turn state machine" (Init → PromptBuilt)
- (no dedicated ADR yet — prompt composition policy decisions go in a future ADR if they become contentious)

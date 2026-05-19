# H04 · Prompt Composer

> **Status**: 🚧 In progress · Sprint 2

## Role in Harness

Assemble the next `ModelInput` that gets handed to the `ModelGateway`. Reads
events from the session log, the current `HarnessStrategy` (from H10), and
the per-turn tool surface (from H05), and produces a fully-formed input.

H04 is **passive composition**: it never *decides* what context to include —
that responsibility belongs to **H11 Context Manage**, which runs at the
`Init → ContextManaged` transition before H04. H04 honors H11's decisions
by reading the event log: a `ContextCompacted` event in the log tells H04
to skip a range of literal events and emit the compaction's `replacement`
instead. This keeps H04's pure-function invariant (#2 below) intact while
still supporting summarization-based compaction.

See `docs/components/H01-turn-driver.md` §"Init → ContextManaged → PromptBuilt
sequence" for the canonical walkthrough of how H10, H11, H04, H05, H09
collaborate.

## Interface (design level)

- `compose(history: &[ConversationEvent], strategy: &HarnessStrategy, surface: &[ToolDescriptor]) -> ModelInput`
- `ModelInput` is a value type in `cogito-protocol::gateway`. It carries:
  - `system: String` (the system message, from strategy)
  - `messages: Vec<Message>` (the dialogue history; each `Message::User { content: Vec<ContentBlock> }` or `Message::Assistant { content: Vec<ContentBlock> }`)
  - `tools: Vec<ToolDescriptor>` (the surfaced tools, in stable order — adapter serializes to provider-specific tool schema at the wire level)
  - `params: ModelParams` (temperature, max_tokens, etc., from strategy)

`Message` is a tagged union of two roles. There is **no third
`ToolResult` role**: per Anthropic Messages API semantics, a tool result
is a `ContentBlock::ToolResult { call_id, content, is_error }` carried
*inside* a `Message::User`. The OpenAI Chat Completions adapter splits
these into separate `{role: "tool", tool_call_id, content}` messages at
serialization time; the cogito-internal shape stays user/assistant only.

The function is **synchronous** and **deterministic**: same inputs always
produce the same `ModelInput`.

## Dependencies

**Calls (out)**: None. Reads passive inputs only.

**Called by**: H01 Turn Driver, at `ContextManaged → PromptBuilt` (and re-called on re-resume from `PromptBuilt`). Note: as of the ADR-0006 amendment (2026-05-19, PR #6), H04 no longer fires from `Init` — it fires from `ContextManaged`, after H11 has finalized context decisions.

## Critical invariants

1. **Deterministic.** Same `history` + `strategy` + `surface` → byte-identical `ModelInput`.
2. **No I/O.** No file reads, no network, no `Instant::now()`.
3. **Length-budgeted.** Respects `strategy.length_budget` (max tokens of prompt + history). v0.1 truncates oldest-first when over budget. **Summarization is NOT H04's job** — that belongs to H11 Context Manage (runs before H04). When summarization is needed, H11 writes a `ContextCompacted` event; H04 then projects history through that event's `replacement` per invariant #5.
4. **Stable tool ordering.** Tools appear in `ModelInput.tools` sorted by name (or by an explicit `strategy.tool_order` if provided). Order matters for prompt-cache hit rates.
5. **History reconstruction is event-driven.** The dialogue history is rebuilt by iterating the event log and projecting events into `Message` shapes — never read from a separate "messages" table.

## v0.1 scope

- Single prompt template per strategy: `{system}` + flat conversation history + tool schemas verbatim
- Truncation policy: oldest-first message drop until under `strategy.length_budget`
- No summarization, no semantic compression (deferred to H11 Context Manage + ADR-0008)
- No "context pin" mechanism (any pins are normal user / tool_result messages in the log)
- H11 is implemented as a pass-through in Sprint 2; this means H04's behavior in v0.1
  is identical to the pre-amendment design. The architectural slot is reserved without
  changing v0.1 prompt-build behavior.

## History projection

The function walks events in seq order and projects to `Message`s:

| Event | Projects to |
|---|---|
| `UserMessageAdded { content: Vec<ContentBlock> }` | `Message::User { content }` |
| `AssistantMessageAppended { content: Vec<ContentBlock> }` (one per text block sealed by H02) | merged into the in-progress `Message::Assistant { content: [accumulated] }` for the same turn (multiple `AssistantMessageAppended`s within one turn append blocks to the same assistant message) |
| `ToolUseEmitted { call_id, name, args }` | appended as `ContentBlock::ToolUse { call_id, name, args }` to the current assistant message |
| `ToolResultRecorded { call_id, result }` | emitted as a fresh `Message::User { content: [ContentBlock::ToolResult { call_id, content, is_error }] }` after the assistant message that requested it |
| `HookModified { ... }` | (no projection — informational only) |
| `ContextCompacted { replaced_seq_range, replacement, ... }` (pending ADR-0008) | skip events with seq in `replaced_seq_range`; project `replacement` blocks into messages |
| `SystemPromptInjected { suffix }` (pending ADR-0008) | append `suffix` to the composed `system` field |
| State transitions (`TurnStarted`, `PromptComposed`, ...) | (no projection) |

Events that aren't part of the model-visible dialogue (control events,
hook events, redaction markers) are skipped during projection.

**Projection through compaction** (locked by this PR; details pending ADR-0008):
when the walker encounters a `ContextCompacted` event with
`replaced_seq_range = (lo, hi)`, it (1) skips all events whose seq is in
`[lo, hi]`, (2) emits the events from `replacement` in their place, and
(3) continues the walk from `seq = hi + 1`. The compacted events are
**not deleted** from the log — the append-only invariant of ADR-0007
holds — they are only skipped by H04's projection.

## Open design questions

- Truncation strategy at 0.2: keep most recent N, drop middle? Summarize middle? Initial v0.1: drop oldest, simple. **Summarization-as-truncation is now H11's concern**, not H04's — ADR-0008 will define the policy.
- Whether to include partial `TextDelta` events from a crashed turn in projection: initial answer: no — only sealed `ModelCallCompleted` events project to history.

## Testing strategy

- **Unit**: empty history, single-turn history, multi-turn history, history with tool results, history over length budget.
- **Snapshot** (insta): canonical event log → canonical `ModelInput` JSON, locked.
- **Property**: composition is idempotent under reordering of independent events (e.g., reordering `HookModified` events doesn't change the resulting `ModelInput` — they don't project).

## References

- ARCHITECTURE.md §"Turn state machine" (Init → PromptBuilt)
- (no dedicated ADR yet — prompt composition policy decisions go in a future ADR if they become contentious)

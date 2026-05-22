# ADR-0019: Reasoning content modeling and event scope

## Status

Accepted (2026-05-22)

## Context

Model providers cogito targets surface chain-of-thought in three
distinct ways:

- **Anthropic Messages API** emits `thinking` content blocks inline
  with `text` / `tool_use`, carrying a `signature` opaque blob (or a
  `redacted_thinking { data }` variant when safety-filtered).
  Streaming deltas arrive as `thinking_delta` and `signature_delta`
  events. Multi-step tool use with extended thinking **requires
  verbatim round-trip of the signature** or the API rejects the next
  call.
- **OpenAI Responses API** emits `reasoning` items at the **top level
  of `response.output`** — siblings of `message` and `function_call`
  — with a `summary` array and an `encrypted_content` blob (or
  `previous_response_id` for stateful continuation). Same round-trip
  requirement.
- **OpenAI-compat / Chat Completions backends** (vLLM, sglang,
  llama.cpp, DeepSeek-R1/V3, QwQ, Qwen3-Thinking, GLM-4.5-Thinking,
  etc.) — the wire format has no first-class reasoning field. Two
  sub-conventions exist in the wild:
  - **Inline `<think>...</think>` tags** inside the regular
    `delta.content` text stream (most open-source serving stacks,
    raw model output).
  - **Separate `delta.reasoning_content` field** parallel to
    `delta.content` (DeepSeek's official API, some vLLM configs
    with `--enable-reasoning`).
  No signature, no encrypted blob, **no round-trip requirement for
  correctness** — open-source reasoning models generally don't
  validate prior thinking on follow-up calls, and the DeepSeek-R1 /
  QwQ training conventions explicitly **drop prior thinking** on
  multi-turn calls.

The current cogito build (Sprint 3, schema_version 1) handles only
the third regime, and only by accident: `cogito-model::openai_compat`
passes `delta.content` straight through, so when a model emits
`<think>...</think>`, the entire tagged span ends up inside one
`AssistantMessageAppended.text` event. A representative log line:

```json
{"schema_version":1,"event_id":"…","session_id":"…","turn_id":"…",
 "seq":31,"type":"assistant_message_appended",
 "data":{"text":"<think>Both attempts ga...\n</think>\n\n\n"}}
```

This is the only reasoning shape currently representable — it carries
no structural distinction between reasoning and final text, breaks
UI rendering (the model's private thought is shown as if it were
the assistant's reply), and would not satisfy the Anthropic /
Responses round-trip constraints if those providers were enabled
today.

cogito's current protocol has no slot for it:

- `ContentBlock` (Sprint 1) covers `Text` / `ToolUse` / `ToolResult` only.
- `EventPayload` (Sprint 1 + Sprint 3 `ModelCallCompleted`) has
  `AssistantMessageAppended { text }` — pure string, no provider opaque.
- `ModelEvent` (Sprint 2) has `TextDelta` / `TextBlockCompleted` /
  `ToolUseStarted` / `ToolUseCompleted` / `MessageCompleted` — no
  reasoning analog.
- `StreamEvent` (broadcast for UI) has `TextDelta` but no thinking
  delta.

We need to decide, before either provider's reasoning mode lands in a
sprint:

1. **Modeling level** — does reasoning live inside the assistant
   message (as a `ContentBlock` variant) or as a sibling top-level
   event (Codex-style)?
2. **Provider-opaque carrier** — where do `signature` /
   `encrypted_content` ride such that resume + multi-step tool use
   keep working unchanged?
3. **Streaming surface** — what does H06 emit, and what does the
   broadcast channel surface for live UI (folded "Thinking…" panels)?
4. **Persistence semantics** — does this need a new `EventPayload`
   variant + `SCHEMA_VERSION` bump, or is the `non_exhaustive`
   `ContentBlock` enough?

## Decision

### 1. `ContentBlock::Thinking` — single inline variant

Add to `cogito-protocol::content::ContentBlock`:

```rust
ContentBlock::Thinking {
    /// Human-readable reasoning text (Anthropic: full thinking;
    /// OpenAI: concatenated summary). May be empty if the provider
    /// only returned an opaque encrypted payload.
    text: String,
    /// Provider-opaque blob that MUST round-trip verbatim on the next
    /// model call for multi-step tool use to validate. Anthropic
    /// stores `signature` (and the `redacted_thinking` data when
    /// applicable); OpenAI stores `encrypted_content` and the
    /// reasoning item `id`. Schema is provider-defined and opaque to
    /// cogito.
    provider_opaque: Option<serde_json::Value>,
},
```

This places reasoning **inside the assistant message's `content` vector**,
alongside `Text` and `ToolUse`. H04 Prompt Composer rebuilds the
prior assistant turn's `Message::Assistant.content` from the event
log in `seq` order — exactly as it does today for interleaved text
and tool_use — so signatures ride along without any provider-specific
reassembly step. The full reflow contract lives in §4.

`#[non_exhaustive]` on `ContentBlock` (ADR-0007 §"Additive variants")
means this is an **additive change with no `SCHEMA_VERSION` bump**.
External readers tolerating unknown `type` values keep working;
schema-aware readers regenerate from `docs/schemas/conversation-event-v1.json`.

### 2. Persistence via a new additive `EventPayload` variant

Add to `cogito-protocol::event::EventPayload`:

```rust
/// One thinking/reasoning content block has been sealed by the
/// provider. Sibling to `AssistantMessageAppended` (one event per
/// completed block, in arrival order). `turn_id` is on the envelope.
ThinkingBlockRecorded {
    /// Full reasoning text for the sealed block. May be empty when
    /// the provider exposes only an encrypted summary.
    text: String,
    /// Provider-opaque blob (signature / encrypted_content / item
    /// id) that must round-trip verbatim on the next call.
    provider_opaque: Option<serde_json::Value>,
},
```

`AssistantMessageAppended { text: String }` stays unchanged.
`SCHEMA_VERSION` stays at 1. This follows ADR-0007 §"Additive variant
precedent" verbatim: `EventPayload` is `#[non_exhaustive]`, Rust
consumers extend their `_ =>` arms, cross-language consumers tolerate
the unknown `type`.

Block ordering across `ThinkingBlockRecorded`,
`AssistantMessageAppended`, and `ToolUseRecorded` is reconstructed by
**event `seq`** — the same mechanism that already orders interleaved
text and tool_use events today. Anthropic's "thinking precedes
text/tool_use within one assistant message" constraint is satisfied
naturally because the provider emits thinking blocks first in time,
so H02 assigns them a lower `seq` than subsequent text/tool_use
events within the same turn.

### 3. Streaming surface: `ModelEvent` + `StreamEvent` additions

`cogito-protocol::gateway::ModelEvent` gains:

```rust
ThinkingDelta { block_index: u32, chunk: String },
ThinkingBlockCompleted {
    block_index: u32,
    text: String,
    provider_opaque: Option<serde_json::Value>,
},
```

Symmetric with `TextDelta` / `TextBlockCompleted`. H06 Stream
Demultiplexer treats them with the same pre-aggregation rule: deltas
pass through, the `Completed` event seals the block and feeds H02.

`cogito-protocol::stream::StreamEvent` gains:

```rust
ThinkingDelta { chunk: String },
```

Symmetric with `TextDelta`. UI surfaces (REPL, future TUI) render this
into a folded "Thinking…" section. The H02 batching rule for
`AssistantMessageAppended` (≤200ms or ≤500 chars before flush, per
CLAUDE.md §"Inviolable design rules" #2) **extends to thinking
deltas** — same flush trigger, single batched append per block.

### 4. H04 reflow contract

H04 Prompt Composer rebuilds `Message::Assistant.content` by walking
events for the assistant turn in `seq` order and mapping each event
to its `ContentBlock`:

| Event variant | Mapped block |
|---|---|
| `ThinkingBlockRecorded { text, provider_opaque }` | `ContentBlock::Thinking { text, provider_opaque }` |
| `AssistantMessageAppended { text }` | `ContentBlock::Text { text }` |
| `ToolUseRecorded { call_id, tool_name, args }` | `ContentBlock::ToolUse { call_id, tool_name, args }` |

This is the **same walk** H04 already performs for `text` + `tool_use`
interleaving today — `ThinkingBlockRecorded` slots in transparently.
The rule that elevates correctness to inviolable:

> H04 MUST include all `ThinkingBlockRecorded` events of an assistant
> turn when reflowing that turn's `Message::Assistant.content`, in
> their original `seq` position, with `provider_opaque` carried
> verbatim. Stripping or reordering them invalidates the next-turn
> signature check on Anthropic and the reasoning-item continuity on
> OpenAI.

H11 Context Manage (ADR-0008, pending) gets a complementary rule: any
summarization / compaction step MUST either keep all
`ThinkingBlockRecorded` events of an assistant turn alongside the
turn's other events, OR drop the entire assistant turn from history
(cannot keep `AssistantMessageAppended` + `ToolUseRecorded` but drop
`ThinkingBlockRecorded`).

### 5. End-to-end walkthrough across all three provider regimes

This section grounds the abstract decisions in concrete event flow.
Each subsection traces one model turn ("model thinks → speaks →
calls a tool") through:

```
provider wire  →  cogito-model adapter  →  ModelEvent  →  H06 demux  →
       ↓
   H02 writes ConversationEvent  →  cogito-store-jsonl (append-only)
       ↓
   next turn: H04 walks events by seq → rebuilds Message::Assistant.content
```

#### 5.1 Anthropic regime (signature-bound)

Provider wire (Anthropic Messages SSE):

```
event: content_block_start  {index:0, type:"thinking"}
event: content_block_delta  {index:0, delta:{type:"thinking_delta", thinking:"I should grep "}}
event: content_block_delta  {index:0, delta:{type:"thinking_delta", thinking:"for the symbol."}}
event: content_block_delta  {index:0, delta:{type:"signature_delta", signature:"abc123…"}}
event: content_block_stop   {index:0}
event: content_block_start  {index:1, type:"text"}
event: content_block_delta  {index:1, delta:{type:"text_delta", text:"Looking now."}}
event: content_block_stop   {index:1}
event: content_block_start  {index:2, type:"tool_use", id:"toolu_01", name:"grep"}
event: content_block_delta  {index:2, delta:{type:"input_json_delta", partial_json:"{\"pattern\":\"foo\"}"}}
event: content_block_stop   {index:2}
event: message_delta        {delta:{stop_reason:"tool_use"}, usage:{...}}
```

`cogito-model::anthropic::decode` aggregates `thinking_delta` + the
trailing `signature_delta` into a single sealed block. `ModelEvent`s
emitted to H06:

```
ThinkingDelta { block_index:0, chunk:"I should grep " }
ThinkingDelta { block_index:0, chunk:"for the symbol." }
ThinkingBlockCompleted {
    block_index: 0,
    text: "I should grep for the symbol.",
    provider_opaque: Some({"signature":"abc123…"}),
}
TextDelta { block_index:1, chunk:"Looking now." }
TextBlockCompleted { block_index:1, text:"Looking now." }
ToolUseStarted { block_index:2, call_id:"toolu_01", tool_name:"grep" }
ToolUseCompleted { block_index:2, call_id:"toolu_01", tool_name:"grep", args:{"pattern":"foo"} }
MessageCompleted { stop_reason: ToolUse, usage: {...} }
```

H06 forwards deltas to the broadcast channel (as `StreamEvent::ThinkingDelta`
/ `StreamEvent::TextDelta`) and feeds `*Completed` events to H02. H02
appends:

```
seq:5  ThinkingBlockRecorded { text:"I should grep for the symbol.",
                               provider_opaque: Some({"signature":"abc123…"}) }
seq:6  AssistantMessageAppended { text:"Looking now." }
seq:7  ToolUseRecorded { call_id:"toolu_01", tool_name:"grep",
                         args:{"pattern":"foo"} }
seq:8  ModelCallCompleted { stop_reason: ToolUse, usage: {...} }
```

Next turn, H04 walks seq 5–7 and produces:

```rust
Message::Assistant { content: vec![
    ContentBlock::Thinking { text:"I should grep for the symbol.",
                             provider_opaque: Some({"signature":"abc123…"}) },
    ContentBlock::Text     { text:"Looking now." },
    ContentBlock::ToolUse  { call_id:"toolu_01", tool_name:"grep", args:{"pattern":"foo"} },
]}
```

`cogito-model::anthropic::encode` unpacks `provider_opaque.signature`
back into the wire-format `signature` field. The chain validates.

#### 5.2 OpenAI Responses regime (encrypted_content-bound)

Provider wire (Responses streaming):

```
event: response.output_item.added       {item:{type:"reasoning", id:"rs_01"}}
event: response.reasoning_summary_text.delta  {item_id:"rs_01", delta:"I'll grep for it."}
event: response.output_item.done        {item:{type:"reasoning", id:"rs_01",
                                              encrypted_content:"enc_xyz…",
                                              summary:[{type:"summary_text", text:"I'll grep for it."}]}}
event: response.output_item.added       {item:{type:"message", id:"msg_01"}}
event: response.output_text.delta       {delta:"Looking now."}
event: response.output_item.done        {item:{type:"message", id:"msg_01",
                                              content:[{type:"output_text", text:"Looking now."}]}}
event: response.output_item.added       {item:{type:"function_call", call_id:"call_01", name:"grep"}}
event: response.output_item.done        {item:{type:"function_call", call_id:"call_01",
                                              name:"grep", arguments:"{\"pattern\":\"foo\"}"}}
event: response.completed               {response:{...usage...}}
```

`cogito-model::openai_responses` (v0.2 — adapter does not exist
today; landed alongside this ADR) maps `reasoning` items into
`ThinkingDelta` / `ThinkingBlockCompleted` exactly like Anthropic:

```
ThinkingBlockCompleted {
    block_index: 0,
    text: "I'll grep for it.",
    provider_opaque: Some({"item_id":"rs_01", "encrypted_content":"enc_xyz…"}),
}
```

The `item_id` ride-along is what makes `previous_response_id`
continuation optional — the next call can either pass
`previous_response_id` (cheap, stateful) or pass the encrypted
reasoning items verbatim in `input` (stateless, portable). H04
generates the latter form by default: it puts a
`ContentBlock::Thinking` into the assistant message, and the
adapter on the way out emits a `{type:"reasoning", id, encrypted_content}`
item into `input.items`. The persisted event log is identical
in shape to §5.1, only `provider_opaque` differs.

#### 5.3 OpenAI-compat regime (`<think>` tag / `reasoning_content`)

This is the regime that currently produces the log line in the
context section. Two sub-flavors must be handled by
`cogito-model::openai_compat::decode`:

##### 5.3.a Inline `<think>...</think>` tags

Provider wire (Chat Completions SSE, raw tag inlining):

```
data: {"choices":[{"delta":{"content":"<thi"}}]}
data: {"choices":[{"delta":{"content":"nk>Both attempts ga"}}]}
data: {"choices":[{"delta":{"content":"ve the same hash.</thi"}}]}
data: {"choices":[{"delta":{"content":"nk>\n\nLet me try grep."}}]}
data: {"choices":[{"delta":{"tool_calls":[{"id":"call_01","function":{"name":"grep","arguments":"{\"pattern\":\"foo\"}"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls"}]}
```

The adapter runs a **two-state SSE parser** ("outside think" /
"inside think") that buffers across chunk boundaries — `<think>`
and `</think>` may straddle any number of `data:` frames.
Pseudocode:

```
state = Outside
for chunk in delta.content stream:
    buf += chunk
    loop:
        if state == Outside:
            if buf contains "<think>":
                emit TextDelta for text before "<think>"
                consume up through "<think>"
                state = Inside, block_index = next
            else:
                emit TextDelta for safe prefix (keep last 7 chars in case "<think>" is split)
                break
        else:  # Inside
            if buf contains "</think>":
                emit ThinkingDelta for text before "</think>"
                consume up through "</think>"
                emit ThinkingBlockCompleted { text: accumulated, provider_opaque: None }
                state = Outside
            else:
                emit ThinkingDelta for safe prefix (keep last 8 chars)
                break
on stream end while Inside:
    emit ThinkingBlockCompleted with accumulated text (treat unclosed as best-effort)
```

`provider_opaque` is **always `None`** for this regime. `ModelEvent`s
emitted to H06 are then **identical in shape to §5.1**, and the rest
of the pipeline is unchanged.

##### 5.3.b Separate `delta.reasoning_content` field

Provider wire (DeepSeek official API / vLLM `--enable-reasoning`):

```
data: {"choices":[{"delta":{"reasoning_content":"Both attempts gave "}}]}
data: {"choices":[{"delta":{"reasoning_content":"the same hash."}}]}
data: {"choices":[{"delta":{"content":"Let me try grep."}}]}
data: {"choices":[{"delta":{"tool_calls":[...]}}]}
```

This is the cleaner case: `delta.reasoning_content` directly maps
to `ThinkingDelta`, `delta.content` to `TextDelta`. The adapter
emits a synthetic `ThinkingBlockCompleted` when it first sees a
non-empty `delta.content` (i.e., the reasoning block has ended)
or at stream end if reasoning never gave way to content.

##### 5.3 round-trip on the way back out

OpenAI-compat reasoning models do **not** validate prior thinking
on follow-up turns — DeepSeek-R1's paper and QwQ's model card
both recommend **dropping** the prior turn's thinking content
before the next call (the model is trained to think fresh each
turn). H04's default behavior for this regime:

```rust
match block {
    ContentBlock::Thinking { .. } if provider_id == "openai_compat" => {
        // Drop. Configurable via provider config:
        //   [provider.foo] include_prior_thinking = true
    }
    ContentBlock::Thinking { text, .. } if include_prior_thinking => {
        // Re-wrap as <think>…</think> in the outgoing user/system text,
        // OR set delta.reasoning_content if the backend honors it.
    }
    ...
}
```

The persisted `ThinkingBlockRecorded` event keeps the reasoning
for replay, audit, and UI; H04 decides per-provider how (or
whether) to re-feed it.

##### 5.3 backward compatibility with old log lines

**Inviolable rule: persisted JSONL is append-only and never
rewritten by cogito.** Sessions recorded before this ADR lands —
including ones with `<think>…</think>` baked into
`AssistantMessageAppended.text`, like the example in the Context
section — stay literally as they are on disk, byte-for-byte. This
applies regardless of which provider regime wrote them.

Concretely:

- No backfill job, no migration tool, no normalization pass. The
  adapter starts emitting cleanly-separated `ThinkingBlockRecorded`
  events from the moment the new code lands; everything before
  that boundary is frozen.
- Replay of an old session must succeed without any rewriting
  step. H04 walking events for an old assistant turn produces a
  `ContentBlock::Text` whose `text` happens to contain
  `<think>…</think>` literally — which is exactly what was sent
  to the model at write time, so the replay is faithful.
- Renderers (REPL, future TUI, external consumers) that want to
  retroactively prettify old sessions may apply a client-side
  parser to the rendered text, but **must not** persist the
  result back into the event log.
- New sessions recorded after the change get the clean separation
  (separate `ThinkingBlockRecorded` events, no tags in
  `AssistantMessageAppended.text`). Old shape and new shape
  therefore coexist in storage; readers handle both.

The same rule applies if a future ADR introduces yet another
reasoning representation: cogito appends-forward, never
rewrites-backward.

## Alternatives considered

### Alt-A: top-level `EventPayload::ReasoningAppended` (Codex style)

Mirror OpenAI Responses: a sibling event next to
`AssistantMessageAppended`, with reasoning living **outside** the
assistant message.

**Rejected** because:

- H04 has to interleave reasoning back into the API's `content` array
  anyway (both providers require it), so the "events are siblings"
  framing leaks complexity into prompt composition.
- Anthropic's signature contract is **per content array, not per
  event** — splitting reasoning out of the assistant message would
  require a synthetic reassembly step inside `cogito-model::anthropic`
  on every call.
- "Is reasoning part of the assistant turn?" becomes ambiguous: H10
  Strategy Selector and H11 Context Manage both need to answer
  consistently, and inlining gives a single answer ("yes, it's a
  ContentBlock") that survives provider differences.

The one benefit Codex's split offers — cleanly excluding reasoning
from UI history rendering — is recovered by the StreamEvent
`ThinkingDelta` channel + a `ContentBlock::Thinking` filter in the
renderer. UI clarity does not require event-shape segregation.

### Alt-B: rename `AssistantMessageAppended.text` to `content: Vec<ContentBlock>` + bump `SCHEMA_VERSION` to 2

Collapse all assistant-side blocks into one event shape, modelling
the assistant message as a single sealed `Vec<ContentBlock>` instead
of one event per block.

**Rejected** because:

- It is a breaking field rename — the first `SCHEMA_VERSION` bump
  cogito would ship. The migration story in ADR-0005 §4 #2 and
  ADR-0007 §"Storage-level contracts" is documented but not yet
  exercised; spending that capital on a change ADR-0007's additive
  precedent can already absorb is poor sequencing.
- It also changes H02's emission discipline: today H02 writes one
  event per completed block (`AssistantMessageAppended` per text
  block, `ToolUseRecorded` per tool_use). Collapsing to one per
  message would either delay persistence until `MessageCompleted`
  (worse for crash recovery) or require a "draft" event that gets
  overwritten (violates ADR-0002 append-only semantics).
- Event-seq ordering already carries block order today for
  text + tool_use interleaving. Adding `ThinkingBlockRecorded`
  reuses that mechanism without inventing a new one.

### Alt-C: defer the whole question

Skip reasoning support until a sprint asks for it.

**Rejected** because v0.1 already ships Anthropic + OpenAI providers,
and the moment a user enables `claude-opus-4-7` with extended thinking
or `gpt-5` reasoning mode, multi-step tool use silently drops the
chain. Silent correctness loss is worse than scheduled work.

## Consequences

- **Easier**: ADR-0007's additive-variant precedent absorbs the
  change. `SCHEMA_VERSION` stays at 1; no migration tooling, no
  fixture rewrite, only an additive fixture row.
- **Easier**: H02 keeps its "one event per sealed block" discipline
  unchanged; `ThinkingBlockRecorded` is just a third per-block event
  shape next to `AssistantMessageAppended` and `ToolUseRecorded`.
- **Easier**: H04 reuses today's seq-ordered walk; `cogito-model::anthropic`
  and `cogito-model::openai_*` adapters converge on one internal
  representation with `provider_opaque` as a single `Option<Value>`
  slot.
- **Easier**: UI gets a clean broadcast channel for live "Thinking…"
  rendering without coupling to persistence shape.
- **Harder**: `provider_opaque` becomes a persisted public blob.
  Whatever shape Anthropic / OpenAI use today is locked into our
  JSONL spec; if a provider changes their signature format, we
  carry the old shape forward in stored sessions even after we
  upgrade the adapter.
- **Harder**: cross-language readers (Go / Python / Node services
  per ADR-0007) must learn the new `ContentBlock` variant **and**
  the new `EventPayload::ThinkingBlockRecorded` event type. The
  `non_exhaustive` discipline keeps them parseable, but UI
  rendering of historical sessions outside Rust requires awareness.
- **Harder**: assistant-message reconstruction now requires walking
  three event types in seq order instead of two — easy to get wrong
  in a hand-rolled external reader. Mitigated by extending the
  canonical fixture and the JSONL spec doc.
- **Deliberately not done**: no backfill, no migration, no
  server-side rewriting of already-persisted JSONL — old sessions
  with `<think>…</think>` embedded in `AssistantMessageAppended.text`
  stay byte-for-byte as-is on disk. Old and new shapes coexist
  forever; readers handle both. See §5.3 for the inviolable rule.
- **Open question (out of scope here)**: deployment-time redaction.
  Some tenants may not want reasoning persisted at all (PII,
  regulatory). Suggest a future ADR introduce a
  `RuntimeConfig.privacy.persist_reasoning: bool` knob; when
  false, H06 drops `ThinkingBlockCompleted` before reaching H02,
  and H04 must tolerate history without thinking blocks (which
  forces a non-streaming next-call mode on Anthropic).

## Follow-on work

- **Sprint N** (TBD, before any reasoning-capable model is exposed via
  `cogito.toml`):
  1. Land `ContentBlock::Thinking` variant + JSON schema regen +
     fixture update (no `SCHEMA_VERSION` bump).
  2. Land `EventPayload::ThinkingBlockRecorded` additive variant +
     fixture update (no bump; covered by ADR-0007 precedent).
  3. Land `ModelEvent::ThinkingDelta` / `ThinkingBlockCompleted` +
     `StreamEvent::ThinkingDelta`.
  4. Update H04 reflow + H06 demux + H02 batching tests; extend the
     resume-chaos suite to cover a turn that contains a sealed
     `ThinkingBlockRecorded` followed by `AssistantMessageAppended`
     and `ToolUseRecorded` in mixed seq order.
- **Adapter work** (same sprint):
  - `cogito-model::anthropic`: map `thinking_delta` /
    `signature_delta` / `content_block_stop(type=thinking)` and
    `redacted_thinking` to the new events; pack signature into
    `provider_opaque`.
  - `cogito-model::openai_responses` (new sub-module if the
    Responses API ships before the ADR adapter sprint, else
    deferred): map `reasoning` output items; pack
    `encrypted_content` + reasoning item `id` into `provider_opaque`.
  - `cogito-model::openai_compat`: implement the §5.3 two-state
    SSE parser for `<think>...</think>` tags, **and** read
    `delta.reasoning_content` when the field is present. Both
    paths emit `ThinkingDelta` / `ThinkingBlockCompleted` with
    `provider_opaque = None`. Add a `provider.<name>.include_prior_thinking:
    bool` config (default false) for the on-the-way-out behavior
    described in §5.3.
- **Documentation**:
  - Add `ContentBlock::Thinking` and
    `EventPayload::ThinkingBlockRecorded` to
    `docs/data-model/jsonl-v1.md` as additive entries — **no
    version filename bump**; the file stays `jsonl-v1.md`.
  - Add ordering rule ("Thinking precedes Text / ToolUse within one
    assistant message") to `AGENTS.md` §"Inviolable design principles".
  - Add the §5.3 inviolable rule ("persisted JSONL is append-only
    and never rewritten by cogito") to `AGENTS.md` as well.
  - Per CLAUDE.md, propagate the locked decision to
    `docs/components/H02-step-recorder.md`,
    `H04-prompt-composer.md`, `H06-stream-demux.md` once Accepted.
- **Explicitly out of scope** (do not do):
  - Backfill / migration / normalization of existing JSONL files,
    whether or not they contain `<think>…</think>` text. See §5.3.
- **Deferred**: privacy/redaction ADR (see "Open question" above).

## References

- ADR-0002 (event sourcing)
- ADR-0005 §4 #2 (schema_version + migration policy)
- ADR-0006 (Runtime + H01 execution model)
- ADR-0007 (event log cross-language contract; `non_exhaustive`
  precedent for additive variants)
- Anthropic Messages API — extended thinking + interleaved tool use
  (provider docs)
- OpenAI Responses API — reasoning items + `previous_response_id`
  (provider docs)
- `crates/cogito-protocol/src/content.rs` (current `ContentBlock`)
- `crates/cogito-protocol/src/event.rs` (current `EventPayload`)
- `crates/cogito-protocol/src/gateway.rs` (current `ModelEvent`)
- `crates/cogito-protocol/src/stream.rs` (current `StreamEvent`)

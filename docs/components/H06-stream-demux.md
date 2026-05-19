# H06 · Stream Demultiplexer

> **Status**: 🚧 Not implemented · Sprint 2

## Role in Harness

Consume the streaming response from `ModelGateway::stream(...)` and split
it into typed cogito events. Drives the streaming side of H02 (text deltas)
and feeds the buffer that H07 reads at `ModelCompleted`.

## Interface (design level)

- `demux<S: Stream<Item = Result<ModelEvent, ModelError>>>(stream: S, ctx: ExecCtx, recorder: &dyn StepRecorderHandle, broadcast: &broadcast::Sender<StreamEvent>) -> Result<ModelOutput, ModelError>`
- `ModelOutput` contains the sealed assistant message: `{ content: Vec<ContentBlock>, stop_reason, usage }`. It is what H07 reads.
- Side effects during streaming: each event produces zero or more `record(...)` / `on_text_delta(...)` / `on_text_block_complete(...)` calls on the recorder, plus broadcast sends on the live channel.

## Dependencies

**Calls (out)**:
- H02 Step Recorder (`on_text_delta` for live text fanout; `on_text_block_complete` to seal a text block; `record` for the eventual `ModelCallCompleted` event emitted by H01).

**Called by**: H01 Turn Driver, during the `ModelCalling → ModelCompleted` transition.

## What gets emitted (provider-agnostic)

Different LLM providers stream different event shapes. The `ModelGateway`
adapter **pre-aggregates** them into a small, stable set of cogito-internal
`ModelEvent` variants (see `cogito-protocol::gateway::ModelEvent`). H06
consumes this normalized stream:

| Provider input (examples) | adapter emits `ModelEvent::` |
|---|---|
| Anthropic `content_block_delta { delta: text_delta }` | `TextDelta { block_index, chunk }` |
| Anthropic `content_block_stop` (text block) | `TextBlockCompleted { block_index, text }` (gateway accumulates per-block text) |
| Anthropic `content_block_start { tool_use }` | `ToolUseStarted { block_index, call_id, name }` |
| Anthropic `content_block_delta { delta: input_json_delta }` | (buffered inside adapter; not emitted) |
| Anthropic `content_block_stop` (tool_use block) | `ToolUseCompleted { block_index, call_id, name, args }` |
| OpenAI `choices[].delta.content` | `TextDelta { block_index: 0, chunk }` |
| OpenAI `choices[].delta.tool_calls[i]` | buffered per call_id inside adapter |
| OpenAI `finish_reason: tool_calls` | one `ToolUseCompleted` per buffered call + `MessageCompleted` |
| `stop_reason: end_turn / tool_use / max_tokens / stop_sequence` | last event: `MessageCompleted { stop_reason, usage }` |

The **pre-aggregation responsibility lives in each `ModelGateway` adapter**
(see `cogito-model::anthropic::decode` / `cogito-model::openai_compat::decode`).
H06 never sees partial JSON or per-block running text — it only sees sealed
`*Completed` events. This keeps H06 stateless w.r.t. block accumulation.

## Critical invariants

1. **Streaming.** H06 does not buffer the entire response. `TextDelta`
   events flow through to the live broadcast channel and to the recorder's
   `on_text_delta` accumulator as they arrive.
2. **Persistence is at content_block boundary, not chunk-by-chunk.**
   H06 calls `recorder.on_text_block_complete()` exactly when
   `ModelEvent::TextBlockCompleted` arrives. The recorder writes one
   `AssistantMessageAppended` event per text block (no 200 ms timer, no
   character threshold — see AGENTS.md §2 and H02 §"Text block lifecycle").
3. **`TextDelta` is live-only.** Each `TextDelta` is broadcast to subscribers
   via `StreamEvent::TextDelta` but **not** persisted directly; persistence
   happens at `TextBlockCompleted`.
4. **`ToolUseCompleted` is recorded immediately.** When H06 receives a
   `ToolUseCompleted`, it calls `recorder.record(ToolUseEmitted { ... })`
   immediately — no batching applies to non-text events.
5. **`ModelCallCompleted` is recorded by H01**, not by H06. H06 returns
   `ModelOutput`; H01 calls `record(ModelCallCompleted { ... })` as part of
   the transition to `ModelCompleted`.
6. **No interpretation.** H06 does not parse tool args against schemas,
   doesn't validate names — that's H07. It only consumes the normalized
   `ModelEvent` stream and writes the appropriate H02 events.

## v0.1 scope

- Anthropic + OpenAI provider event shapes covered
- No streaming tool-arg interpretation
- No multi-message streaming (one model call = one assistant message)

## Open design questions

- Backpressure: if H02's recorder is slow (store under load), should H06 buffer model chunks in memory or apply backpressure upstream to the gateway? Initial answer: backpressure — let the gateway / HTTP client slow down, don't accumulate unbounded memory.
- Cancellation mid-stream: when `ctx.cancel` fires, the in-flight stream is dropped; whatever text was already in the recorder buffer gets flushed; turn transitions to `Failed { kind: Cancelled }`. H06's role is to detect cancellation and stop reading; the gateway's role is to close the HTTP connection.

## Testing strategy

- **Unit**: synthetic `ModelEvent` streams with interleavings of text + tool_use + stop_reason; verify recorded event sequence.
- **Property**: arbitrary valid streams produce no duplicate events, no dropped events, no out-of-order tool_use sequences.
- **Integration**: `cogito-mock-model` produces scripted streams; end-to-end test verifies event log matches expectation.

## References

- ARCHITECTURE.md §"Turn state machine" (ModelCalling → ModelCompleted)
- H02 (the recorder, which H06 calls heavily)
- `cogito-model::ModelGateway` (the input source; the trait is provider-agnostic)

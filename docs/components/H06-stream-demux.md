# H06 · Stream Demultiplexer

> **Status**: 🚧 Not implemented · Sprint 2

## Role in Harness

Consume the streaming response from `ModelGateway::stream(...)` and split
it into typed cogito events. Drives the streaming side of H02 (text deltas)
and feeds the buffer that H07 reads at `ModelCompleted`.

## Interface (design level)

- `demux<S: Stream<Item = ModelEvent>>(stream: S, ctx: ExecCtx, recorder: &dyn StepRecorderHandle) -> impl Future<Output = ModelOutput>`
- `ModelOutput` contains the sealed assistant message: `{ text, tool_uses, stop_reason, usage }`. It is what H07 reads.
- Side effects during streaming: each chunk produces zero or more `record(...)` / `record_text_delta(...)` calls.

## Dependencies

**Calls (out)**:
- H02 Step Recorder (`record_text_delta` for assistant text; `record` for tool_use start / arg deltas / end events).

**Called by**: H01 Turn Driver, during the `ModelCalling → ModelCompleted` transition.

## What gets emitted (provider-agnostic)

Different LLM providers stream different event shapes. H06 normalizes them
into a small, stable set of cogito-internal events:

| Provider input (examples) | cogito event emitted |
|---|---|
| Anthropic `content_block_delta { delta: text_delta }` | `TextDelta { turn_id, content }` (via batched recorder) |
| Anthropic `content_block_start { tool_use }` | `ToolUseStarted { turn_id, call_id, name }` |
| Anthropic `content_block_delta { delta: input_json_delta }` | (buffered internally; not recorded yet) |
| Anthropic `content_block_stop` (for tool_use) | `ToolUseEmitted { turn_id, call_id, name, args }` |
| OpenAI `chat.completion.chunk.choices[].delta.content` | `TextDelta` |
| OpenAI `chat.completion.chunk.choices[].delta.tool_calls[]` | combined into `ToolUseEmitted` at message end |
| `stop_reason: end_turn / tool_use / max_tokens / stop_sequence` | (returned in `ModelOutput.stop_reason`; turn transition recorded by H01) |

The provider mapping lives in each `ModelGateway` impl; H06 receives the
already-normalized `ModelEvent` stream from the gateway. The gateway is
responsible for translating provider quirks; H06 is responsible for *what
events get recorded as the stream progresses*.

## Critical invariants

1. **Streaming.** H06 does not buffer the entire response. Text deltas flow through to H02's batched path as they arrive.
2. **Tool-use is buffered until `content_block_stop`.** Mid-stream partial JSON args are not recorded — only the fully-emitted `ToolUseEmitted` event is. (Partial JSON is rarely useful and is fragile to re-stream after resume.)
3. **`text_delta` events use the batched recorder path** (200 ms / 500 char window); all other events go through immediate `record()`.
4. **`ModelCallCompleted` is recorded by H01**, not by H06. H06 returns `ModelOutput`; H01 calls `record(ModelCallCompleted { ... })` as part of the transition to `ModelCompleted`.
5. **No interpretation.** H06 does not parse tool args against schemas, doesn't validate names — that's H07. It just normalizes shape.

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

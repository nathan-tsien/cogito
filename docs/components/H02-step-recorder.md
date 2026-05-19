# H02 · Step Recorder

> **Status**: ✅ Implemented · Sprint 1 (text-block lifecycle, JSONL backend) · `crates/cogito-core/src/harness/step_recorder.rs`

## Role in Harness

Persist every meaningful step as a `ConversationEvent` by calling the
`ConversationStore` trait. Every Brain component, including H01, writes
events through H02 — H02 is the **only** path into the event log from Brain.

## Interface (design level)

- `record_*` family (e.g. `record_session_started`, `record_turn_started`,
  `record_tool_use`, `record_tool_result`, `record_turn_completed`,
  `record_turn_failed`) — immediate, single-event append. Each returns
  only after `ConversationStore::append` has durably accepted the event.
- `on_text_delta(turn_id, chunk)` — **does not persist**. Accumulates
  chunks into the in-flight `TextBlockBuf` AND broadcasts each chunk on
  the live `StreamEvent::TextDelta` channel for UI subscribers.
- `on_text_block_complete()` — drains the in-flight text buffer and
  writes a single `AssistantMessageAppended` event. Triggered by H06
  when the model emits `content_block_stop` for a text block.

H02 batches assistant text by **wire-protocol content_block boundary**
(matches Codex / Claude Code) — there is no timer and no character
threshold. See AGENTS.md §2 for the inviolable rule and "Text block
lifecycle" below for the full state diagram.

## Dependencies

**Calls (out)**:
- `ConversationStore::append(session_id, ConversationEvent) -> EventSeq` (the single dependency)

**Called by**: H01 (state transitions), H06 (text deltas, tool_use events from the stream), H07 (parse outcome), H08 (dispatch outcome, job submission), H09 (hook decisions), H10 (strategy selection — once per turn). Effectively all of Brain.

## Critical invariants

1. **Non-text-delta records write immediately.** No buffering, no batching, no coalescing. `record(event).await` returns only after the store has durably accepted the event.
2. **Text content is persisted at content_block boundary** — see "Text block lifecycle" below. Text deltas accumulate in an in-memory buffer; the buffer is drained to a single `AssistantMessageAppended` event when the demultiplexer signals `text_block_complete`. **No timer-based or size-based batching exists** (AGENTS.md §2). The live `StreamEvent::TextDelta` broadcast is independent of persistence.
3. **Append-only.** Records are never updated, never deleted. Compaction / archival is out of scope.
4. **Sequence is monotonic per session.** `EventSeq` is assigned by the store; H02 surfaces it back to the caller for ordering reference but does not generate it.
5. **Failure surfaces upward as `RecordError`.** Store-level errors do not become panics or silent drops. H01 treats most `RecordError`s as fatal for the current turn (transition to `Failed`).

## Behavior under crash

- After a successful `record_*().await`, the event is in the OS page
  cache via userspace flush per event; no `sync_data`/`fsync` in v0.1
  JSONL (dev/debug only — production durability lives in
  `cogito-store-postgres` at v0.4). Process crash is recoverable; power
  loss may lose recent events. The next Brain instance reads back
  whatever the kernel managed to flush.
- If `record_*().await` panics mid-write, the JSONL file may have a
  partial line. H03 + the store must tolerate this by ignoring the
  trailing partial line on read.
- The in-flight text-block buffer is **non-durable**. An unfinished
  block is lost on crash. This is acceptable because the model can
  re-stream the block on resume; the event log only commits a text
  block at `content_block_stop`, so the "before" state is well-defined
  (no half-written `AssistantMessageAppended`).

## v0.1 scope

- Sole `ConversationStore` impl: `cogito-store-jsonl` (per-session file,
  userspace flush per event; no fsync — see ADR-0007 dev/debug scope).
- No compaction, no archival, no rotation.
- Text-block buffer is per-session, in-process; one `TextBlockBuf` slot
  per recorder, drained on `on_text_block_complete()`.

## Open design questions

- Should `record()` accept a "batch" of events (e.g., several tool_use events resolved together)? Initial answer: no — keep the API one-event-at-a-time; if perf bites, add a `record_batch` later.
- Backpressure on the store: what does H02 do if `append` is slow? Initial answer: it blocks the calling component (H01 cannot transition; the FSM stalls). This is correct semantics — better to stall than to drop events.

## Testing strategy

- **Unit**: text-block lifecycle — `on_text_delta` accumulates into the buffer + broadcasts each chunk; `on_text_block_complete` drains and writes one `AssistantMessageAppended`. Verify with synthetic delta streams that multiple `text_block_complete` boundaries produce N separate events (not one combined event).
- **Contract**: any future `ConversationStore` impl must pass the same contract test as `cogito-store-jsonl`. Shared test in `cogito-protocol::tests::store_contract` (consumed by each store crate).
- **Property** (proptest): given an arbitrary sequence of `record`, `on_text_delta`, and `on_text_block_complete` calls, the resulting event log replays to a structurally equivalent state.
- **Performance**: experiment E01 (10K events) targets P99 write < 5 ms and is the published budget.

## References

- ARCHITECTURE.md §"State storage planes" P1
- ADR-0002 (event-sourced conversation log)
- AGENTS.md §"Inviolable design principles" #2

## Text block lifecycle

Per ADR-0007 + spec §6.1, H02 batches text content by **wire-protocol
content_block boundary**, not by timer or character threshold. The
lifecycle is:

1. H06 emits `text_delta` chunks for the current content_block.
   `StepRecorder::on_text_delta` accumulates them into
   `current_text_block.text` AND broadcasts each chunk as
   `StreamEvent::TextDelta` for live subscribers. **Nothing is
   persisted yet.**
2. H06 emits `text_block_complete` when the model signals
   `content_block_stop` for a text block.
   `StepRecorder::on_text_block_complete` writes one
   `AssistantMessageAppended` carrying the full accumulated text and
   clears the buffer.

On crash mid-block: the recorder dies with the SessionActor (no
cross-turn state per ADR-0006 §3). The accumulated text is lost.
Resume restarts the turn from `ModelCalling`, the model re-streams,
and no partial assistant message ends up in the persisted log.

This matches Codex's `should_persist_event_msg` (filters out
`AgentMessageDelta`, persists only `AgentMessage`) and Claude Code's
behavior.

## Implementation note (v0.1)

H02 has no standalone object in v0.1. Logically it splits into:

- **Producer side**: every call site (TurnDriver state transitions,
  actor main loop, hooks) sends `PersistCommand::Append { event, ack }`
  on a `persist_tx: mpsc::Sender<PersistCommand>` (capacity 256), then
  awaits the `ack` oneshot before transitioning.

- **Consumer side**: a `store_writer` tokio subtask owns the
  `ConversationStore` handle (`crates/cogito-core/src/runtime/store_writer.rs`).
  It writes each `PersistCommand::Append` to the store as a single event
  (no batching), calls `store.append` with per-event userspace flush (via
  `spawn_blocking`), and signals the `ack` oneshot. Text-block accumulation
  happens upstream in the recorder's in-memory `TextBlockBuf`; the writer
  sees only sealed `AssistantMessageAppended` events.

The producer/consumer split is what makes the inviolable rule "every
state transition writes an event before transitioning" cheap: the
producer awaits one mpsc round-trip + one ack — the actor's mailbox
stays polled the whole time because the producer is the TurnDriver
task, not the actor itself.

See `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
§8 for fsync strategy, batching rules, and the Sprint 1 SLO benchmark plan.

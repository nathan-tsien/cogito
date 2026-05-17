# H02 · Step Recorder

> **Status**: 🚧 Not implemented · Sprint 1

## Role in Harness

Persist every meaningful step as a `ConversationEvent` by calling the
`ConversationStore` trait. Every Brain component, including H01, writes
events through H02 — H02 is the **only** path into the event log from Brain.

## Interface (design level)

- `record(event: ConversationEvent) -> Result<EventSeq, RecordError>` — immediate append; returns the assigned sequence number on success.
- `record_text_delta(session_id, turn_id, delta: &str)` — batched path for streaming text; flushed when one of:
  - 200 ms has elapsed since the first un-flushed delta, **or**
  - 500 characters have accumulated, **or**
  - `flush_text_buffer(session_id)` is called explicitly (e.g., at `ModelCompleted` transition).
- `flush_text_buffer(session_id)` — force the streaming buffer out before reading state.

H02 is internally a thin async wrapper around `ConversationStore` plus a
small per-session text-delta buffer.

## Dependencies

**Calls (out)**:
- `ConversationStore::append(session_id, ConversationEvent) -> EventSeq` (the single dependency)

**Called by**: H01 (state transitions), H06 (text deltas, tool_use events from the stream), H07 (parse outcome), H08 (dispatch outcome, job submission), H09 (hook decisions), H10 (strategy selection — once per turn). Effectively all of Brain.

## Critical invariants

1. **Non-text-delta records write immediately.** No buffering, no batching, no coalescing. `record(event).await` returns only after the store has durably accepted the event.
2. **Text-delta batching is the *only* exception**, bounded by 200 ms / 500 chars / explicit flush.
3. **Append-only.** Records are never updated, never deleted. Compaction / archival is out of scope.
4. **Sequence is monotonic per session.** `EventSeq` is assigned by the store; H02 surfaces it back to the caller for ordering reference but does not generate it.
5. **Failure surfaces upward as `RecordError`.** Store-level errors do not become panics or silent drops. H01 treats most `RecordError`s as fatal for the current turn (transition to `Failed`).

## Behavior under crash

- After a successful `record().await`, the event is durably on disk in v0.1's JSONL backend (`fsync` after each write). The system can crash at any point after; the next Brain instance reads the event back.
- If `record().await` panics mid-write, the JSONL file may have a partial line. H03 + the store must tolerate this by ignoring the trailing partial line on read.
- The text-delta buffer is **non-durable**. Unflushed deltas are lost on crash. This is acceptable because the model can re-stream them on retry; the event log only commits text deltas after batching, so the "before" state is well-defined.

## v0.1 scope

- Sole `ConversationStore` impl: `cogito-store-jsonl` (per-session file, `fsync` per event).
- No compaction, no archival, no rotation.
- Text-delta buffer is per-session, in-process, with a single timer per session.

## Open design questions

- Should `record()` accept a "batch" of events (e.g., several tool_use events resolved together)? Initial answer: no — keep the API one-event-at-a-time; if perf bites, add a `record_batch` later.
- Backpressure on the store: what does H02 do if `append` is slow? Initial answer: it blocks the calling component (H01 cannot transition; the FSM stalls). This is correct semantics — better to stall than to drop events.

## Testing strategy

- **Unit**: text-delta batching behavior under all three flush triggers (timer, char count, explicit flush). Synthetic delta streams.
- **Contract**: any future `ConversationStore` impl must pass the same contract test as `cogito-store-jsonl`. Shared test in `cogito-protocol::tests::store_contract` (consumed by each store crate).
- **Property** (proptest): given an arbitrary sequence of `record` and `record_text_delta` calls, the resulting event log replays to a structurally equivalent state.
- **Performance**: experiment E01 (10K events) targets P99 write < 5 ms and is the published budget.

## References

- ARCHITECTURE.md §"State storage planes" P1
- ADR-0002 (event-sourced conversation log)
- AGENTS.md §"Inviolable design principles" #2

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

All `record_*` methods currently return `Result<(), StoreError>`. Sprint 3
P2.5 will unify all `record_*` method signatures to return
`Result<EventId, StoreError>`. Until then, only `record_turn_failed`
effectively surfaces the event identity indirectly via `TurnOutcome::Failed
{ recorded_event_id }` (Sprint 3 P2.5 will generalize this so any caller
that needs to reference a recorded event can do so, replacing the Sprint 2
`"unknown"` stub).

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

## Events written

Each `record_*` call appends exactly one `ConversationEvent` variant to the store. Events are listed in the order they appear during a normal turn.

- **`SessionStarted { meta }`** — Recorded by H01 when a new session is created. Session-level event; `turn_id` is `None` on the envelope.
- **`TurnStarted { user_input }`** — Recorded by H01 when a new user turn begins. `user_input` is `Vec<ContentBlock>`.
- **`ContextManageEntered {}`** — Recorded at the start of the `Init → ContextManaged` transition. v0.1 H11 is a pass-through; ADR-0008 will replace the body with real context decisions.
- **`ContextManageCompleted {}`** — Recorded at the end of the `ContextManaged → PromptBuilt` transition. v0.1 pass-through carries no decision body.
- **`PromptComposed { model, surface_size }`** — Recorded after H04 composes the prompt and H05 builds the tool surface. Carries metadata only — the full prompt is NOT persisted (event log is a state-recovery source, not a prompt cache; see ADR-0007).
- **`ModelCallStarted { model }`** — Recorded by H01 at the `PromptBuilt → ModelCalling` transition boundary, right before the gateway stream opens.
- **`AssistantMessageAppended { text }`** — Written by H02 when `on_text_block_complete` is called (text-block boundary, not per-delta).
- **`ToolUseRecorded { call_id, tool_name, args }`** — Recorded by H06 when the model emits a complete `tool_use` block.
- **`ModelCallCompleted { stop_reason, usage }`** *(Sprint 3 P2.2 — not yet shipped)* — Recorded by H06 Stream Demultiplexer when the model response stream emits `ModelEvent::MessageCompleted` (Anthropic `message_delta` with stop_reason / OpenAI `finish_reason`). Sealing event for one model call. Enables H03 to distinguish "model call done" from "model call in flight" without re-issuing the gateway request. See spec §4 Q1.
- **`ToolResultRecorded { call_id, result }`** — Recorded by H08 after a tool call completes (success or structured error).
- **`TurnCompleted { outcome }`** — Recorded by H01 on successful turn completion.
- **`TurnFailed { reason }`** — Recorded by H01 when the turn FSM transitions to `Failed`. `reason` is a `TurnFailureReason`; `turn_id` is on the event envelope, not in the payload. The Sprint 2 `TurnOutcome::Failed { recorded_event_id: "unknown" }` stub will be unified in Sprint 3 P2.5 (recorders to return `Result<EventId, StoreError>`).
- **`TurnPaused { job_id }`** — Recorded by H01 when the turn pauses on an async job. Precedes `JobCompletedRecorded` on resume.
- **`JobCompletedRecorded { job_id, outcome }`** — Recorded by H01 when a previously-awaited async job has finished. Triggers the resume path.

## Recorder API

All `record_*` methods currently return `Result<(), StoreError>`. Sprint 3 P2.5 will unify all `record_*` method signatures to return `Result<EventId, StoreError>`. Until then, only `record_turn_failed` returns a result that callers need to thread into `TurnOutcome::Failed { recorded_event_id }` (replacing the Sprint 2 `"unknown"` stub). Sprint 3 P2.5 generalizes this so any caller that needs to reference a recorded event can do so.

| Method | Parameters | Return type | Called by |
|---|---|---|---|
| `record_session_started` | `meta: SessionMeta` | `Result<(), StoreError>` | H01 on session creation. |
| `record_turn_started` | `turn_id, user_input: Vec<ContentBlock>` | `Result<(), StoreError>` | H01 on new user turn. |
| `record_context_manage_entered` | `turn_id` | `Result<(), StoreError>` | H01 at `Init → ContextManaged` transition start. |
| `record_context_manage_completed` | `turn_id` | `Result<(), StoreError>` | H01 at `ContextManaged → PromptBuilt` transition end. |
| `record_prompt_composed` | `turn_id, model: String, surface_size: u32` | `Result<(), StoreError>` | H01 after H04/H05 produce the `ModelInput`. |
| `record_model_call_started` | `turn_id, model: String` | `Result<(), StoreError>` | H01 at `PromptBuilt → ModelCalling` transition boundary. |
| `on_text_delta` | `turn_id, chunk: String` | `()` | H06 per text-delta chunk; accumulates buffer + broadcasts. Does not write to store. |
| `on_text_block_complete` | — | `Result<(), StoreError>` | H06 on `content_block_stop` for a text block; drains buffer to `AssistantMessageAppended`. |
| `record_tool_use` | `turn_id, call_id: String, tool_name: String, args: serde_json::Value` | `Result<(), StoreError>` | H06 when model emits a complete `tool_use` block. |
| `record_model_call_completed` *(P2.2 — not yet shipped)* | `turn_id, stop_reason, usage` | `Result<EventId, StoreError>` | H06 demux loop when `MessageCompleted` model event observed. |
| `record_tool_result` | `turn_id, call_id: String, result: ToolResult` | `Result<(), StoreError>` | H08 after tool dispatch completes. |
| `record_turn_paused` | `turn_id, job_id: JobId` | `Result<(), StoreError>` | H01 when turn pauses on async job. |
| `record_job_completed` | `turn_id, job_id: JobId, outcome: JobOutcome` | `Result<(), StoreError>` | H01 when previously-awaited job finishes. |
| `record_turn_completed` | `turn_id, outcome: TurnOutcome` | `Result<(), StoreError>` | H01 on successful turn completion. |
| `record_turn_failed` | `turn_id, reason: TurnFailureReason` | `Result<(), StoreError>` | H01 on FSM transition to `Failed`. |

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
- ADR-0006 §7 (dual-stream rationale: persisted vs live broadcast)
- ADR-0007 (JSONL dev/debug scope; prompt-not-persisted rationale)
- ADR-0008 (future context management decisions)
- AGENTS.md §"Inviolable design principles" #2
- Sprint 3 spec: `2026-05-20-sprint-3-resume-coordinator-design.md` §4 Q1 + §5.4

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

On crash mid-block: the recorder dies with the per-session loop task (no
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

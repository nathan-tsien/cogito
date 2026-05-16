# H03 · Resume Coordinator

> **Status**: 🚧 Not implemented · Sprint 3

## Role in Harness

Decide where to resume a turn given the persisted event log. **Pure
function**: same input → same output, no I/O, no clock, no random.

A new Brain instance picks up an existing session by reading the event log
and asking H03 "what state should I start in, and where do I read up to?".
H03 is the load-bearing piece of cogito's resumability — it's the function
that makes ADR-0002's event sourcing actually replayable.

## Interface (design level)

- `resume_decision(events: &[ConversationEvent]) -> ResumeDecision`
- `ResumeDecision { state: TurnState, last_event_seq: u64, partial_text: Option<String> }`
  - `state`: the FSM state H01 should enter
  - `last_event_seq`: highest sequence number consumed; subsequent appends should be `> last_event_seq`
  - `partial_text`: any unfinished assistant text from `TextDelta` events that hasn't been wrapped in a `TextCompleted`

The function is **synchronous and pure**. No `async`. The caller is
responsible for loading the events (asynchronously) before calling H03.

## Resume decision table

Given the **last fully-recorded event** in the session's log:

| Last event | Resume state | Reasoning |
|---|---|---|
| (none — empty log) | `Init` | First turn |
| `TurnStarted` | `Init` | Re-derive prompt from scratch |
| `StrategySelected` | `Init` | Strategy is known but prompt not built yet |
| `PromptComposed` | `PromptBuilt` | Prompt is durable; OK to re-call model |
| `ModelCallStarted` | `PromptBuilt` | Model call may have partially happened — retry from scratch (idempotent at the model level via cache or just re-billing) |
| `TextDelta` (no following `ModelCallCompleted`) | `PromptBuilt` | Re-stream; previous deltas were durable but partial; drop them on resume (`partial_text` returned so H01 can decide to discard) |
| `ToolUseEmitted` (no following `ModelCallCompleted`) | `PromptBuilt` | Same — model output not fully sealed |
| `ModelCallCompleted` | `ModelCompleted` | Model output fully sealed |
| `ToolCallResolved` for some calls | `ModelCompleted` | Re-resolve (cheap, deterministic on input schema) |
| `ToolDispatched` (no result yet) | `ToolDispatching`, with the dispatched call(s) re-checked against `JobManager` to see if they completed off-Brain | A crashed sync tool call must be re-run; an async one's job state is the truth |
| `ToolResultRecorded` for **all** calls in the turn | `ToolDispatching` (back to model with results) | Loop back to `PromptBuilt` next |
| `JobSubmitted` (turn-level pause) | `Paused` | Wait for `JobCompleted` event before continuing |
| `JobCompleted` | `ToolDispatching` | Inject result and continue |
| `TurnCompleted` / `TurnFailed` | (no turn to resume; caller starts a new turn) | Terminal |

> The full machine-readable table lives in `crates/cogito-core/src/harness/resume.rs` once implemented; this doc is the source of truth for *what* it decides, not *how* it's coded.

## Dependencies

**Calls (out)**: None. Pure function.

**Called by**: H01 Turn Driver, once on entry.

## Critical invariants

1. **Pure**: `resume_decision(events_a) == resume_decision(events_b)` whenever `events_a` and `events_b` are byte-identical.
2. **No I/O / no clock / no random.** The function is testable as a unit, in any environment, including under `proptest`.
3. **Idempotent**: H03 may be called multiple times with the same input; behavior is identical.
4. **Last-fully-completed-state wins.** If an event log ends mid-transition (a partial event was being written and the file truncates), the partial event is ignored by the store on read; H03 sees only the last complete event.
5. **Resume is *semantic*, not *byte-exact*.** A resumed turn may produce a different token sequence from the model than the original would have (the model is non-deterministic). The guarantee is that the end-state (`Completed` / `Failed` / `Paused`) is *semantically equivalent* — same tool calls succeeded / failed, same final assistant message intent.

## Open design questions

- Resume after `ModelCallStarted` re-bills tokens. For 0.2 we may add an opt-in "resume from text-delta watermark" if the gateway supports continuation; for v0.1, simplest correct behavior is to retry the model call.
- Async tool re-run vs JobManager state check: when crashed mid-`ToolDispatched`, do we always re-check `JobManager` first, or just re-run? Initial answer: re-check `JobManager` if the tool's `InvokeOutcome::Async` was already recorded; otherwise re-run.

## Testing strategy

- **Unit**: every row of the decision table, exercised with synthetic event sequences.
- **Property** (proptest): arbitrary event sequences produce a decision that satisfies *no event would arrive in the resumed state that contradicts an event already in the log*.
- **Chaos** (`tests/resume_chaos.rs`): the headline test of cogito.
  - Generate a "golden run" with no crashes.
  - Replay the same input, injecting a crash between every adjacent event-pair.
  - Resume from log; finish the turn.
  - Verify the resumed end-state is semantically equivalent to the golden run.
  - This test runs in `--release` and is the gate for declaring resumability "works."

## References

- ARCHITECTURE.md §"Turn state machine"
- ADR-0002 (event sourcing)
- ADR-0003 (state-machine Turn Driver)
- AGENTS.md §"Inviolable design principles" #3, #4

# H03 · Resume Coordinator

> **Status**: 🟡 Designed · Sprint 3 (implementation in P3)

## Role in Harness

Decide where to resume a turn given the persisted event log. **Pure
function**: same input → same output, no I/O, no clock, no random.

A new Brain instance picks up an existing session by reading the event log
and asking H03 "what state should I start in, and where do I read up to?".
H03 is the load-bearing piece of cogito's resumability — it's the function
that makes ADR-0002's event sourcing actually replayable.

## Interface

Entry point: `harness::resume::replay(events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError>`

```rust
// crates/cogito-core/src/harness/resume.rs

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeDecision {
    pub point: ResumePoint,
    /// `seq` of the last event in the log when this decision was computed.
    /// `None` iff `point == FreshTurn` AND the log is empty.
    /// Actor uses this to initialize the per-session event seq generator.
    pub last_event_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ResumePoint {
    /// Empty log or last turn already terminal (TurnCompleted/Failed).
    /// Actor stays idle; the next Input triggers spawning TurnDriver.
    FreshTurn,

    /// In-flight turn but no completed model call. FSM enters Init; H04
    /// rebuilds the prompt from the log. One model call will be re-billed.
    RestartCurrentTurn { turn_id: TurnId },

    /// Most recent model call has a ModelCallCompleted with stop_reason =
    /// end_turn and no tool calls; actor crashed before writing TurnCompleted.
    /// FSM enters ModelCompleted using rebuilt_output — no model re-call.
    ResumeFromModelCompleted {
        turn_id: TurnId,
        rebuilt_output: ModelOutput,
    },

    /// Tool dispatch round partially in progress; zero or more tools done.
    /// FSM enters ToolDispatching; surface is reconstructed by enter_turn
    /// via H10 + H05.
    ResumeFromToolDispatching {
        turn_id: TurnId,
        /// Unpaired ToolUseRecorded entries after latest_mcc; log order
        /// preserved. H07 re-validates schema before dispatch.
        pending: Vec<ResumePendingCall>,
        /// Already-paired (call_id, ToolResult) entries.
        completed: Vec<(String, ToolResult)>,
    },

    /// Turn paused on an async job. TurnPaused is the latest event and has no
    /// following JobCompletedRecorded. Actor re-registers on_complete.
    ResumePausedJob { turn_id: TurnId, job_id: JobId },

    /// Async job completed but Brain crashed before consuming
    /// JobCompletedRecorded. FSM enters ToolDispatching with the result
    /// injected. call_id is resolved by walk-back (Sprint 3 invariant:
    /// ≤1 async dispatch per turn; Sprint 4 may revise).
    ResumeAfterJobCompletion {
        turn_id: TurnId,
        job_id: JobId,
        outcome: JobOutcome,
        call_id: String,
        completed_before_pause: Vec<(String, ToolResult)>,
        pending_after_pause: Vec<ResumePendingCall>,
    },
}

/// Raw triple recovered from a ToolUseRecorded event.
/// enter_turn passes this through H07 re-validation before dispatch.
#[derive(Debug, Clone)]
pub struct ResumePendingCall {
    pub call_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    #[error("malformed event log: {0}")]
    Malformed(String),
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    #[error("tool `{tool_name}` (call_id `{call_id}`) no longer registered")]
    ToolUnavailable { call_id: String, tool_name: String },
    #[error("tool `{tool_name}` schema rejects persisted args: {reason}")]
    ToolSchemaDrift { tool_name: String, reason: String },
}
```

The function is **synchronous and pure**. No `async`. The caller (actor startup)
loads events asynchronously before calling H03; H03 itself does no I/O.

## Resume decision table

The algorithm runs in O(N) with a single linear scan over the event log.
It proceeds in three phases.

### Phase 1 — Find the nearest turn boundary

Scan backward from the end of the log for the first event in
`{TurnStarted, TurnCompleted, TurnFailed, TurnPaused}`.

| Boundary event | Condition | ResumePoint |
|---|---|---|
| None (only SessionStarted or empty log) | — | `ResumePoint::FreshTurn` |
| `TurnCompleted` / `TurnFailed` | — | `ResumePoint::FreshTurn` |
| `TurnPaused { job_id }` | No matching `JobCompletedRecorded { job_id }` follows | `ResumePoint::ResumePausedJob` |
| `TurnPaused { job_id }` | A matching `JobCompletedRecorded { job_id, outcome }` follows | `ResumePoint::ResumeAfterJobCompletion` (call_id resolved in Phase 3) |
| `TurnStarted { turn_id }` | — | Proceed to Phase 2 |

### Phase 2 — Classify the in-flight turn

Scan forward from `TurnStarted` and locate:

- `latest_mcs` = index of the most recent `ModelCallStarted`
- `latest_mcc` = index of the most recent `ModelCallCompleted`

| `latest_mcs` | `latest_mcc` | Relation | ResumePoint |
|---|---|---|---|
| `None` | — | — | `ResumePoint::RestartCurrentTurn` |
| `Some(s)` | `None` | model in flight | `ResumePoint::RestartCurrentTurn` |
| `Some(s)` | `Some(c)` | `s > c` | `ResumePoint::RestartCurrentTurn` (new model call in flight) |
| `Some(s)` | `Some(c)` | `c ≥ s` | Examine tool events after `c` → Phase 2b |

**Phase 2b** — pair `ToolUseRecorded` with `ToolResultRecorded` after `latest_mcc`:

| ToolUseRecorded count | Paired (k) | Unpaired (u) | ResumePoint |
|---|---|---|---|
| 0 | — | — | `ResumePoint::ResumeFromModelCompleted` (rebuild output; stop_reason must be end_turn) |
| ≥1 | k | u | `ResumePoint::ResumeFromToolDispatching { pending: u, completed: k }` |

### Phase 3 — Construct ResumeDecision

- `last_event_seq = events.last().map(|e| e.seq)`.
- **`ResumeFromModelCompleted.rebuilt_output`**: scan events between
  `latest_mcs` and `latest_mcc`; assemble `AssistantMessageAppended →
  ContentBlock::Text` and `ToolUseRecorded → ContentBlock::ToolUse` in seq
  order into `Vec<ContentBlock>`; attach `stop_reason` and `usage` from
  `latest_mcc`.
- **`ResumeAfterJobCompletion.call_id`**: walk back before `TurnPaused` and
  find the most recent unmatched `ToolUseRecorded` call_id. Sprint 3
  invariant: ≤1 async dispatch per turn. Sprint 4 may add `call_id` directly
  to the `TurnPaused` payload when multi-async-dispatch is introduced.

### Error cases

- `JobCompletedRecorded` with no matching `TurnPaused` → `ResumeError::Malformed`.
- Nested turns (`TurnStarted` inside an un-terminated `TurnStarted`) →
  `ResumeError::Malformed` (v0.1 invariant: single turn-in-flight per session).
- `schema_version > SCHEMA_VERSION` → `ResumeError::UnsupportedSchema`.
- `schema_version < SCHEMA_VERSION` → currently SCHEMA_VERSION is 1; no
  action needed; revisit post-Sprint 7.

## Algorithm sketch

The three phases above map directly to the implementation:

**Step 1 — Backward scan for turn boundary.** Walk from `events.last()` toward
the front, stopping at the first `TurnStarted`, `TurnCompleted`, `TurnFailed`,
or `TurnPaused`. A `TurnCompleted`/`TurnFailed` boundary means the prior turn
is closed; return `FreshTurn`. A `TurnPaused` boundary triggers a forward scan
for the matching `JobCompletedRecorded`.

**Step 2 — Forward scan from latest TurnStarted.** Collect `ModelCallStarted`
and `ModelCallCompleted` indices. If no `ModelCallCompleted` is found (or the
last `ModelCallStarted` is later than the last `ModelCallCompleted`), the
model call is incomplete — return `RestartCurrentTurn`.

**Step 3 — Match unpaired tool calls.** Walk events after `latest_mcc`.
Build two collections: `completed` (each `ToolUseRecorded` that has a
matching `ToolResultRecorded`) and `pending` (those that do not). If both
collections are empty and `stop_reason == end_turn`, reconstruct
`ModelOutput` inline and return `ResumeFromModelCompleted`.

**Output construction.** All reconstruction of `ModelOutput` (Phase 3
`rebuilt_output`) happens here. This is the only place in H03 that performs
an "events → high-level value" transformation. `enter_turn` receives the
result without needing to re-scan.

`SessionActor::actor_main` calls `replay` at step ⑥ of its startup sequence
(after schema validation, before initializing the seq counter):

```rust
// ② H03 computes the decision
let decision = match harness::resume::replay(&initial_events) {
    Ok(d) => d,
    Err(e) => return ShutdownOutcome::ResumeFailed(e.to_string()),
};

// ③ seq generator initialized (must precede any write)
state.event_seq.store(
    decision.last_event_seq.map_or(0, |s| s + 1),
    Ordering::SeqCst,
);

// ⑤ branch on ResumePoint
apply_resume_point(&mut state, decision.point).await?;
```

Step ordering is fixed: schema check → `replay` → seq init → branch. Swapping
② and ③ would allow a write with `seq < last_event_seq`, violating
ADR-0002 immutability.

## Critical invariants

1. **Pure**: `replay(events_a) == replay(events_b)` whenever `events_a` and
   `events_b` are byte-identical.
2. **No I/O / no clock / no random.** The function is testable as a unit, in
   any environment, including under `proptest`.
3. **Idempotent**: H03 may be called multiple times with the same input;
   behavior is identical.
4. **Last-fully-completed-state wins.** If an event log ends mid-transition (a
   partial event was being written and the file truncates), the partial event
   is ignored by the store on read; H03 sees only the last complete event.
5. **Resume is *semantic*, not *byte-exact*.** A resumed turn may produce a
   different token sequence from the model than the original would have (the
   model is non-deterministic). The guarantee is that the end-state
   (`Completed` / `Failed` / `Paused`) is *semantically equivalent* — same
   tool calls succeeded / failed, same final assistant message intent.
6. **`ResumeDecision` is never persisted.**
   `ResumeDecision` is a **derived projection** from the event log, not durable
   state. The actor recomputes it from scratch on every startup. Persisting it
   would violate Inviolable Rule #3 (state lives in the event log / Conversation
   Service, not in Harness memory) — creating a second source of truth that
   drifts under schema evolution and duplicates information already authoritative
   in `ConversationStore`. Even if every nested type happens to be `Serialize`
   today, the rule forbids the storage. See spec §6 落盘语义 and ADR-0002.

## Dependencies

**Calls (out)**: None. Pure function.

**Called by**: H01 Turn Driver, once on entry. Specifically, invoked from
`SessionActor::actor_main` at step ⑥ — after schema validation and before
the per-session seq counter is initialized (see Algorithm sketch above).

## Open design questions

These items are tracked from spec §9 risks 1, 4, and 6 (items that do not
block Sprint 3 but remain visible):

- **Mock model determinism** (risk 1, blocking): `cogito-mock-model` must
  return byte-identical `ModelEvent` streams for the same input across
  repeated calls. Oracles ③ (tool mapping) and ④ (final text) of the chaos
  test are meaningless if the mock is non-deterministic. Verify and, if
  needed, add `ScriptedMockModel` before any chaos test code runs.
- **Performance at 10k+ events** (risk 4): H03 currently does a full linear
  scan of all events. The algorithm only needs events from the most recent
  `TurnStarted` onward, but Sprint 3 does not optimize this. Incremental
  reads are deferred to a v0.6 hardening ADR.
- **`TurnPaused` missing `call_id` payload** (risk 6): Sprint 3's walk-back
  algorithm for resolving `call_id` in `ResumeAfterJobCompletion` depends on
  the Sprint 3 invariant of ≤1 async dispatch per turn. When Sprint 4
  introduces multi-async-dispatch, `TurnPaused` will need an explicit
  `call_id` field; the walk-back approach will no longer be correct.

## Testing strategy

The chaos test is the headline test of cogito's resumability. It is part of
the default CI gate (same level as fmt / clippy / unit tests).

### Four oracle assertions

Each oracle corresponds to a distinct failure mode:

```rust
// ① Prefix immutable — verifies ADR-0002: events written before crash
//    are unchanged in the resumed run (modulo event_id / ts).
fn assert_prefix_immutable(golden: &[Event], resumed: &[Event], crash_n: u64);

// ② Terminal equivalent — TurnCompleted / TurnFailed / TurnPaused variant
//    matches; Failed compares TurnFailureReason variant.
fn assert_terminal_equivalent(g_term: &EventPayload, r_term: &EventPayload);

// ③ Tool mapping equivalent — {call_id → (tool_name, args, ToolResult)}
//    set equality; independent of mock determinism.
fn assert_tool_mapping_equivalent(golden: &[Event], resumed: &[Event]);

// ④ Final assistant text equivalent — byte equality; relies on
//    ScriptedMockModel determinism.
fn assert_final_text_equivalent(golden: &[Event], resumed: &[Event]);
```

Oracle ① directly validates the ADR-0002 immutability promise. Oracles ③ and
④ together verify that H07/H08 and H06/H04 reconstruct the correct state on
resume.

### Z mechanism — crash injection

**Y path (primary)**: after writing the N-th event, the test notifies the
test harness via a `oneshot` channel (clean shutdown via
`SessionHandle::shutdown` + normal actor drain). Every event boundary in every
scenario is covered this way.

**X path (depth)**: at curated semantic points (e.g., after `TurnStarted`,
after `ModelCallCompleted`, after `ToolResultRecorded`), a real `panic!()` is
injected, exercising the ADR-0006 panic-catch boundary. A new `Runtime`
instance resumes from the store, simulating a real process restart.

Both paths share the same `FaultInjectingStore` wrapper:

```rust
pub struct FaultInjectingStore<S> {
    inner: S,
    written_count: AtomicU64,
    trigger: Mutex<FaultTrigger>,
}

pub enum FaultTrigger {
    None,
    /// Panic after the N-th event is written (X path). Event is durably
    /// written before the panic fires.
    PanicAt { event_no: u64, message: &'static str },
    /// Notify the test after the N-th event is written (Y path).
    NotifyAt { event_no: u64, signal: oneshot::Sender<()> },
}
```

Production code has zero awareness of fault injection — no `cfg` flags, no
feature gates. The wrapper lives entirely in `cogito-test-fixtures`.

### Four scenarios

| Scenario | Flow | ~Events | ResumePoints covered |
|---|---|---|---|
| `single_tool_happy_path` | user → model+tool_use → tool → model end_turn | ~12 | FreshTurn / RestartCurrentTurn / ResumeFromModelCompleted / ResumeFromToolDispatching |
| `no_tool_short_turn` | user → model end_turn | ~7 | FreshTurn / RestartCurrentTurn / ResumeFromModelCompleted |
| `tool_returns_error` | user → model+tool_use → ToolResult::Error → model handles | ~14 | + tool error path |
| `paused_async_job` | user → model+async_tool → TurnPaused → MockJob.complete → model end_turn | ~10 | ResumePausedJob / ResumeAfterJobCompletion |

### CI budget

| Path | Per run | Scenarios × crash points | Total |
|---|---|---|---|
| Y | ~50 ms (tmpfs + mock model) | 4 × ~12 = ~48 | ~2.5 s |
| X | ~150 ms (panic + new Runtime) | 4 × 8 = 32 | ~5 s |
| **Total** | | | **< 10 s** |

The chaos test suite is part of `just ci`. The `just chaos` recipe is reserved
for future v0.6 fuzz / property tests.

## References

- ARCHITECTURE.md §"Turn state machine" · §"Actor model — why and how" ·
  §"Resume entry path" (under "Turn state machine")
- ADR-0002 (event sourcing)
- ADR-0003 (state-machine Turn Driver)
- ADR-0006 §1 (actor execution model; panic-catch boundary)
- AGENTS.md §"Inviolable design principles" #3, #4
- `docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md`
  (decision rationale for §4 types, §5 actor recovery path, §6 persistence
  semantics, §8 chaos test design, §9 risks)

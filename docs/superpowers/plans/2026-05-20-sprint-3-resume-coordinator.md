# Sprint 3 · Resume Coordinator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Sprint 3 of v0.1 per spec `docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md` — land the `ModelCallCompleted` event variant, the full H03 Resume Coordinator decision table, actor recovery wiring (`SessionMode::Resume` end-to-end), the `FaultInjectingStore` test wrapper + `MockJobManager`, and the chaos test with 4 oracle helpers + Z (Y+X) fault injection. End state: a process can crash at any persisted-event boundary and a new process can resume the session to a semantically equivalent terminal state, verified by CI.

**Architecture:** Single new event variant (`ModelCallCompleted` with `stop_reason` + `usage`) under ADR-0007 `#[non_exhaustive]` b-档兼容（no SCHEMA_VERSION bump). H03 is a pure function (`replay(&[ConversationEvent]) -> Result<ResumeDecision, ResumeError>`) producing `{ point: ResumePoint, last_event_seq: Option<u64> }` with 6 `ResumePoint` variants. Actor startup sequence becomes `schema check → replay → seq init → apply_resume_point → mailbox loop`. Chaos test uses zero-production-code-modification approach: a `FaultInjectingStore` `ConversationStore` wrapper in `cogito-test-fixtures` triggers `panic!` (X-path) or oneshot notification (Y-path) after writing the N-th event.

**Tech Stack:** Rust 2024 (MSRV 1.85), tokio 1.40, async-trait 0.1, futures 0.3, thiserror 1, serde 1, serde_json 1, schemars 0.8, tracing 0.1, tempfile 3.10, nextest, just.

**Conventions (read before starting any task):**

- All Rust comments (`//`, `///`, `//!`) in English (per CLAUDE.md §Coding standards). Chinese stays in spec / ADR / commit / chat.
- Errors via `thiserror` (libraries) or `anyhow` (binaries / tools). No `unwrap` / `expect` / `panic` in non-test code.
- `unsafe_code = "forbid"`. `missing_docs = "warn"` — every public item has a doc comment.
- Workspace deps go through `[workspace.dependencies]`; members declare `{ workspace = true }`.
- Commits: imperative, capitalized first word, no trailing period. Match recent style (`Sprint 3 P1: ...`, `Sprint 3 P2: ...`).
- Each task ends with one commit (unless task is purely a branch / preparatory step).
- Each phase (`P1`...`P5`) is **one PR**. Phase starts with creating its impl branch off latest `main`. Phase ends with opening a PR `nathan-tsien:impl/sprint-3-<phase> -> main`, base = main.
- **Spec is the source of truth.** When this plan and the spec disagree, fix the plan. When the spec is wrong, stop and escalate.
- Layer rule (ADR-0004) is enforced by `just ci`: `cogito-core::harness` may only import `cogito-protocol`. `MockJobManager` / `FaultInjectingStore` live in `cogito-test-fixtures` (testing layer, off-tree from Brain).
- "Write event before transition" (ADR-0003) — every transition's recorder call must complete (with EventId returned) before state moves on.
- After each task: scoped `just fix` + `just test`. After each phase: full `just ci` + `gh pr create`.

**Test snippet adaptation (workspace lints are strict):**

`unwrap_used = "deny"` + `expect_used = "deny"` + `panic = "deny"`. Test snippets below use `.unwrap()` for readability; adapt to:

```rust
#[test]
fn name() -> Result<(), Box<dyn std::error::Error>> {
    // propagate with ?
    Ok(())
}
```

or for `#[tokio::test]` async tests, same pattern with `Result<(), …>` return.

---

## Phase 1 (P1) · Doc propagation — design-only PR

> Per spec §10.1. 9 commits (commit 1 = spec, already at `dc7b6d6`). Branch:
> `impl/sprint-3-p1-doc-propagation`. One PR landing all 9 doc edits.

### Task P1.1: Create branch

- [ ] **Step 1: Branch off main**

```bash
git checkout main
git pull --ff-only
git checkout -b impl/sprint-3-p1-doc-propagation
```

### Task P1.2: ARCHITECTURE.md — add "Actor model — why and how" + "Resume entry path"

**Files:**

- Modify: `ARCHITECTURE.md` (insert new top-level section after `§"Brain / Hands / Session boundaries"`; add subsection to `§"Turn state machine"` end)

- [ ] **Step 1: Read spec §7 (lines 732-842) for content source**

```bash
sed -n '732,842p' docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md
```

- [ ] **Step 2: Read current ARCHITECTURE.md to find insertion points**

```bash
grep -n "^## " ARCHITECTURE.md
```

Expected output includes `## Brain / Hands / Session boundaries` and `## Turn state machine`.

- [ ] **Step 3: Insert new top-level section after `§"Brain / Hands / Session boundaries"` end**

Add a `## Actor model — why and how` section with 5 subsections matching spec §7.1–§7.5:

- §"Why an actor model?" — embedded library serving ≥1000 concurrent sessions; rejects `Arc<Session> + Mutex<ActiveTurn>` (Codex style); 5 constraints listed (failure isolation / external tokio handle / cooperative cancel / dual event streams / async job wake-up).
- §"Four core invariants" — private state / message-driven / single mutable owner / cooperative termination.
- §"Topology" — copy the ASCII diagram from spec §7.3 (Caller → Runtime → SessionActor with mailbox + broadcast + persist + job channels + store_writer subtask).
- §"Advantages in cogito's context" — failure isolation at scheduler / backpressure first-class / cancellation safety / scaling unit clarity / resume locality.
- §"Trade-offs" — per-session memory / mailbox FIFO vs cancel / boilerplate / structured tracing requirement.

End the new section with: `> Cross-refs: ADR-0006 §1 (decision), §3 (cancellation), §4 (channels), §5 (job wake-up); spec 2026-05-20-sprint-3-resume-coordinator-design.md §7.`

- [ ] **Step 4: Append "Resume entry path" subsection to end of `§"Turn state machine"`**

Add an `### Resume entry path` h3 under `## Turn state machine`. Content: copy spec §3 Resume 完整时序 ASCII diagram + 4 关键不变量 (steps ②/⑦/⑨ ordering + PausedOnJob no-spawn rule + ResumeAfterJobCompletion distinctness).

End with: `> Full algorithm: docs/components/H03-resume-coordinator.md. Decision rationale: spec 2026-05-20-sprint-3-resume-coordinator-design.md §3 + §4.`

- [ ] **Step 5: Verify markdown renders cleanly**

```bash
# Use any markdown previewer or just visually scan
head -200 ARCHITECTURE.md | head -100
grep -c "^## Actor model" ARCHITECTURE.md  # expect 1
grep -c "^### Resume entry path" ARCHITECTURE.md  # expect 1
```

- [ ] **Step 6: Commit**

```bash
git add ARCHITECTURE.md
git commit -m "$(cat <<'EOF'
Sprint 3 P1: ARCHITECTURE add Actor model section + Resume entry path

New top-level section "Actor model — why and how" sinks the four
invariants (private state / message-driven / single mutable owner /
cooperative termination) plus the topology diagram and trade-offs
into the durable architecture doc. New "Resume entry path" subsection
under "Turn state machine" carries the end-to-end recovery flow.

Per Sprint 3 spec §10.1 commit 2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task P1.3: ADR-0006 §1 amendment — actor model invariants

**Files:**

- Modify: `docs/adr/0006-runtime-h01-execution-model.md` (expand §1 Decision body; add §"Amendments" 2026-05-20 entry)

- [ ] **Step 1: Read current ADR-0006 §1**

```bash
sed -n '1,80p' docs/adr/0006-runtime-h01-execution-model.md
```

- [ ] **Step 2: Expand §1 "Per-session actor task model" Decision body**

After the existing paragraph ("Each session runs in a dedicated long-lived tokio task..."), insert before the **Rejected** paragraph:

```markdown
The actor model rests on four invariants that are mandatory for correctness, not stylistic preferences:

1. **Private state**: each session's runtime state (in_flight cursor, seq generator, PausedOnJob marker) is owned by **one** task. No cross-actor `Arc<Mutex<_>>`.
2. **Message-driven**: all interaction with an actor goes through channels — mailbox (commands), broadcast (live events), persist (durable events), job sink (async wake-up). Function calls reaching into an actor's internal state = design bug.
3. **Single mutable owner**: the actor task is the only mutator of its private state. Subtasks (TurnDriver, store_writer) receive value copies or explicit handles via channels — never mutable references.
4. **Cooperative termination**: cancellation flows through `CancellationToken` + `select!`, never `task.abort()`. Every await point gets a chance to drop RAII guards and flush pending events.

For the rationale of why these four are load-bearing in cogito's context (≥1000 concurrent sessions, embedded library posture, dual event streams), see **ARCHITECTURE.md §"Actor model — why and how"**.
```

- [ ] **Step 3: Append §"Amendments" entry**

Append at the end of the file (if §"Amendments" doesn't exist, create it):

```markdown
## Amendments

- **2026-05-19 (PR #6)**: FSM extended with `ContextManaged` state between `Init` and `PromptBuilt` to host H11 Context Manage.
- **2026-05-20 (Sprint 3)**: Decision body §1 expanded with the four actor-model invariants. Cross-refs added to ARCHITECTURE.md §"Actor model — why and how" and spec `2026-05-20-sprint-3-resume-coordinator-design.md` §5 + §7.
```

- [ ] **Step 4: Verify**

```bash
grep -c "Private state" docs/adr/0006-runtime-h01-execution-model.md  # expect ≥1
grep -c "^## Amendments" docs/adr/0006-runtime-h01-execution-model.md  # expect 1
```

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0006-runtime-h01-execution-model.md
git commit -m "Sprint 3 P1: ADR-0006 expand §1 with four actor-model invariants" -m "Sinks the durable principles (private state / message-driven / single mutable owner / cooperative termination) into the ADR Decision body. Cross-refs ARCHITECTURE actor model section landed in the previous commit. Per Sprint 3 spec §10.1 commit 3." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.4: H03 doc — full rewrite per Sprint 3 decisions

**Files:**

- Modify: `docs/components/H03-resume-coordinator.md` (full rewrite)

- [ ] **Step 1: Read current H03 doc and spec §4 + §5 + §6**

```bash
cat docs/components/H03-resume-coordinator.md
sed -n '451,694p' docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md
```

- [ ] **Step 2: Replace doc body with the canonical Sprint 3 content**

Status header: `🟢 Implemented · Sprint 3`.

Required sections in this order:

1. **Role in Harness** — keep current phrasing ("Decide where to resume a turn given the persisted event log. Pure function: same input → same output, no I/O, no clock, no random.").
2. **Interface** — replace existing variant list with new types from spec §4 §"类型定义最终形态": `ResumeDecision { point: ResumePoint, last_event_seq: Option<u64> }`; `ResumePoint` enum 6 variants (`FreshTurn`, `RestartCurrentTurn`, `ResumeFromModelCompleted`, `ResumeFromToolDispatching`, `ResumePausedJob`, `ResumeAfterJobCompletion`); `ResumePendingCall` struct; `ResumeError` enum 4 variants.
3. **Resume decision table** — replace existing table with spec §5 阶段一/二/三 tables (turn boundary classification + in-flight turn classification + tool-pair matching).
4. **Algorithm sketch** — copy spec §5.1–§5.3 algorithmic description (backward scan for boundary → forward scan from latest TurnStarted → matching unpaired tool calls → output construction).
5. **Critical invariants** — keep 1–5 from current doc; **add #6: "ResumeDecision is a derived projection from the event log. It is never persisted; the actor recomputes it on every startup. Cross-process serialization would conflict with non-serializable `Arc<dyn ToolHandler>` handles and would diverge across schema evolution. See spec §6 落盘语义."**
6. **Dependencies** — keep current ("Calls (out): None. Pure function. Called by: H01 Turn Driver, once on entry."); refine "Called by" to specify `SessionActor::actor_main` step ⑥ (spec §5.2).
7. **Open design questions** — replace with spec §9 风险条目 1, 4, 6 (mock model determinism / 10k+ events perf / TurnPaused call_id eventual addition).
8. **Testing strategy** — replace with spec §8 chaos test designed structure: 4 oracles (prefix immutable / terminal equivalent / tool mapping equivalent / final text equivalent); Z mechanism (Y path = clean shutdown via NotifyAt; X path = real panic via PanicAt); 4 scenarios (single_tool_happy_path / no_tool_short_turn / tool_returns_error / paused_async_job); CI < 10s budget.
9. **References** — keep current bullet list; add: spec `2026-05-20-sprint-3-resume-coordinator-design.md`; ARCHITECTURE.md §"Actor model" + §"Resume entry path".

- [ ] **Step 3: Verify section structure**

```bash
grep -c "^## " docs/components/H03-resume-coordinator.md  # expect 9
grep -c "ResumePoint" docs/components/H03-resume-coordinator.md  # expect ≥10 (mentioned in types + decision table)
grep -c "FaultInjectingStore" docs/components/H03-resume-coordinator.md  # expect ≥1 (in Testing strategy)
```

- [ ] **Step 4: Commit**

```bash
git add docs/components/H03-resume-coordinator.md
git commit -m "Sprint 3 P1: H03 doc full rewrite per Sprint 3 decisions" -m "Interface / decision table / algorithm / invariants / testing strategy all replaced with Sprint 3 spec content. Adds critical invariant #6 (ResumeDecision is never persisted). Testing strategy reflects 4-oracle / Z-mechanism / 4-scenario chaos test design. Per Sprint 3 spec §10.1 commit 4." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.5: H02 doc — add ModelCallCompleted to event clearcut

**Files:**

- Modify: `docs/components/H02-step-recorder.md`

- [ ] **Step 1: Read current event list and recorder method table**

```bash
grep -n "^##" docs/components/H02-step-recorder.md
grep -n "ModelCallStarted\|record_model_call_started" docs/components/H02-step-recorder.md
```

- [ ] **Step 2: Add ModelCallCompleted to event clearcut**

Locate the event list (probably under `## Events written`). After the `ModelCallStarted` entry, insert:

```markdown
- **`ModelCallCompleted { stop_reason, usage }`** — Recorded by H06 Stream Demultiplexer when the model response stream emits `ModelEvent::MessageCompleted` (Anthropic `message_delta` with stop_reason / OpenAI `finish_reason`). Sealing event for one model call. Enables H03 to distinguish "model call done" from "model call in flight" without re-issuing the gateway request. Added in Sprint 3 (see spec §4 Q1).
```

- [ ] **Step 3: Add `record_model_call_completed` to method table**

Locate the recorder method table (probably under `## Recorder API`). Add row:

```markdown
| `record_model_call_completed` | `turn_id, stop_reason, usage` | `Result<EventId, StoreError>` | Called by H06 demux loop when `MessageCompleted` model event observed. |
```

- [ ] **Step 4: Update method signature note**

Add or update the "all recorder methods return EventId" note. If the doc previously documented `Result<(), StoreError>`, replace with:

```markdown
All `record_*` methods return `Result<EventId, StoreError>`. The returned `EventId` is plumbed back to callers that need to reference the event later — most notably `record_turn_failed` for `TurnOutcome::Failed { recorded_event_id }` (which replaced the Sprint 2 `"unknown"` stub). Sprint 3 unified this signature across all methods.
```

- [ ] **Step 5: Add cross-ref footer**

At end of doc, add: `→ Sprint 3 decision: spec 2026-05-20-sprint-3-resume-coordinator-design.md §4 Q1 + §5.4.`

- [ ] **Step 6: Commit**

```bash
git add docs/components/H02-step-recorder.md
git commit -m "Sprint 3 P1: H02 doc add ModelCallCompleted event + unified EventId return" -m "Event list adds ModelCallCompleted (sealing event for one model call). Recorder method table adds record_model_call_completed. All record_* methods documented as returning Result<EventId, StoreError> (replaces Sprint 2 stub). Per Sprint 3 spec §10.1 commit 5." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.6: H06 doc — add "调 step_recorder 时机" subsection

**Files:**

- Modify: `docs/components/H06-stream-demux.md`

- [ ] **Step 1: Read current H06 doc end**

```bash
tail -50 docs/components/H06-stream-demux.md
```

- [ ] **Step 2: Append new subsection**

Append before any final References section:

```markdown
## Recorder invocation timing

H06's `demux` loop owns the side-effect contract between the gateway stream and `StepRecorder`:

| Model stream event | Recorder call | Persisted event |
|---|---|---|
| `TextDelta { chunk }` | `on_text_delta` (buffer) | (none; flushed at block boundary) |
| `TextBlockCompleted` | `on_text_block_complete` (flush) | `AssistantMessageAppended` |
| `ToolUseCompleted` | `record_tool_use` | `ToolUseRecorded` |
| **`MessageCompleted { stop_reason, usage }`** | **`record_model_call_completed` (added Sprint 3)** | **`ModelCallCompleted`** |

The `MessageCompleted` → `record_model_call_completed` call **must complete before `demux` returns the `ModelOutput`**. This guarantees H03 sees the sealing event before any subsequent transition (H07/H08) writes tool dispatch events — preserving the invariant "events written in causal order".

→ Sprint 3 decision: spec `2026-05-20-sprint-3-resume-coordinator-design.md` §4 Q1 落库时机 + §5.4 EventId 串回.
```

- [ ] **Step 3: Commit**

```bash
git add docs/components/H06-stream-demux.md
git commit -m "Sprint 3 P1: H06 doc add recorder invocation timing table" -m "Documents the MessageCompleted → record_model_call_completed call as a load-bearing step in H06's contract with H02. Locks the causal ordering invariant: sealing event before any tool dispatch events. Per Sprint 3 spec §10.1 commit 6." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.7: H01 doc — add "Resume entry path" subsection

**Files:**

- Modify: `docs/components/H01-turn-driver.md`

- [ ] **Step 1: Find insertion point**

```bash
grep -n "^##" docs/components/H01-turn-driver.md
```

- [ ] **Step 2: Insert "Resume entry path" subsection**

Insert after `## State machine` (or equivalent section showing the FSM):

```markdown
## Resume entry path

When `SessionActor::actor_main` resolves a non-`FreshTurn` resume decision, it calls `enter_turn(turn_entry, ctx, deps)` where `turn_entry` is an internal `TurnEntry` enum translating a `ResumePoint` (which carries actor-level concerns like `turn_id`) into the FSM-level shape the FSM consumes:

```rust
pub(crate) enum TurnEntry {
    /// FSM enters Init. H04 rebuilds prompt from event log; H10 re-selects strategy.
    /// Maps from ResumePoint::RestartCurrentTurn.
    FreshLikeInit,
    /// FSM enters ModelCompleted with rebuilt output; fast-paths to Completed
    /// (no model re-call). Maps from ResumePoint::ResumeFromModelCompleted.
    FromModelCompleted { output: ModelOutput },
    /// FSM enters ToolDispatching with pending/completed pre-populated. H07
    /// re-validates pending against current schemas; H10+H05 rebuild surface.
    /// Maps from ResumePoint::ResumeFromToolDispatching AND
    /// ResumePoint::ResumeAfterJobCompletion (latter injects job outcome as
    /// the final completed entry before entering).
    FromToolDispatching {
        pending: Vec<ResumePendingCall>,
        completed: Vec<(String, ToolResult)>,
    },
}
```

`ResumePoint::FreshTurn` does not produce a `TurnEntry` — the actor idles in `mailbox` until the next `Input` triggers a fresh turn. `ResumePoint::ResumePausedJob` likewise does not spawn a TurnDriver; the actor enters `InFlight::PausedOnJob` directly and re-registers the `on_complete` callback.

`TurnEntry` lives inside `cogito-core::harness::turn_driver` and is **harness-internal** — it never crosses the protocol boundary. The protocol-visible recovery interface is `ResumePoint` (in `cogito-core::harness::resume`).

→ Sprint 3 decision: spec `2026-05-20-sprint-3-resume-coordinator-design.md` §5.3.
```

- [ ] **Step 3: Commit**

```bash
git add docs/components/H01-turn-driver.md
git commit -m "Sprint 3 P1: H01 doc add Resume entry path subsection" -m "Documents the internal TurnEntry enum (3 variants) and how ResumePoint (6 variants) is translated to TurnEntry by SessionActor::apply_resume_point. Clarifies that FreshTurn and ResumePausedJob do not produce a TurnEntry — the actor handles those without spawning a TurnDriver task. Per Sprint 3 spec §10.1 commit 7." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.8: ADR-0007 — add "Additive variant precedent" note

**Files:**

- Modify: `docs/adr/0007-event-log-as-cross-language-contract.md`

- [ ] **Step 1: Read current ADR-0007 end**

```bash
tail -50 docs/adr/0007-event-log-as-cross-language-contract.md
```

- [ ] **Step 2: Append precedent note**

Add a new section before the final References (if any):

```markdown
## Additive variant precedent

Sprint 3 (2026-05-20) added `EventPayload::ModelCallCompleted { stop_reason, usage }` without bumping `SCHEMA_VERSION` (still 1). This sets the precedent for how additive variant changes work under this ADR:

- **Rust consumers**: `#[non_exhaustive]` on `EventPayload` forces match arms to use `_ => { … }` fallbacks. Adding a new variant compiles cleanly without changes at every consumer site.
- **Cross-language consumers** (Go / Python / Node reading JSONL directly): an unknown `type` field on a `ConversationEvent` JSON object SHOULD be tolerated by readers. Older readers see the new event as "unknown type, skip" rather than failing parse. This is consistent with the b-档 (additive backward) compatibility window defined in ADR-0005 §"Compatibility commitments".
- **JSON schema artifact** (`docs/schemas/conversation-event-v1.json`): regenerated by `cogito-gen-schema` after the variant lands. CI drift gate triggers on regeneration; this is expected and the regenerated artifact is part of the same commit.
- **Fixtures** (`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`): updated to include the new event in a representative position.
- **No SCHEMA_VERSION bump**: a bump is reserved for **breaking** changes (removing variants, changing field types, renaming required fields). Adding fields to existing variants likewise does not bump if the field is optional or has a `serde` default.

This precedent applies to all future additive variants and additive fields. See Sprint 3 spec §4 Q1 for the discussion that led to `ModelCallCompleted`.
```

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0007-event-log-as-cross-language-contract.md
git commit -m "Sprint 3 P1: ADR-0007 add Additive variant precedent note" -m "Sprint 3's ModelCallCompleted addition is the first additive EventPayload variant under this ADR. Document the precedent: #[non_exhaustive] for Rust, unknown-type tolerance for cross-language, schema artifact regen, fixture update, no SCHEMA_VERSION bump for additive changes. Per Sprint 3 spec §10.1 commit 8." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.9: jsonl-v1.md — document ModelCallCompleted

**Files:**

- Modify: `docs/data-model/jsonl-v1.md`

- [ ] **Step 1: Read current event documentation**

```bash
grep -n "model_call_started\|ModelCallStarted" docs/data-model/jsonl-v1.md
```

- [ ] **Step 2: Add `model_call_completed` section after `model_call_started`**

```markdown
### `model_call_completed`

Sealing event for one model call. Written by H06 Stream Demultiplexer when the gateway stream emits `MessageCompleted` (Anthropic `message_delta` with stop_reason; OpenAI `finish_reason`). Always follows a preceding `model_call_started` event in the same turn; `seq` is strictly larger.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `stop_reason` | string enum | `"end_turn"` / `"tool_use"` / `"max_tokens"` / `"stop_sequence"` (matches `StopReason` enum in `cogito-protocol::gateway`) |
| `usage` | object | `{ "input_tokens": u32, "output_tokens": u32 }` (further fields may be added under `#[non_exhaustive]`) |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":7,"ts":"2026-05-20T10:00:00Z","type":"model_call_completed","data":{"stop_reason":"tool_use","usage":{"input_tokens":120,"output_tokens":45}}}
```

**H03 use**: distinguishes "model call done" from "model call in flight" without re-issuing the gateway request. Without this event, H03 cannot tell whether to fast-path to `Completed` (when `stop_reason = end_turn` and no tool blocks present) or to dispatch tools — re-calling the model would re-bill tokens. See Sprint 3 spec §4 Q1.

Added: Sprint 3 (2026-05-20). No `SCHEMA_VERSION` bump (additive, b-档 compatible per ADR-0007 §"Additive variant precedent").
```

- [ ] **Step 3: Commit**

```bash
git add docs/data-model/jsonl-v1.md
git commit -m "Sprint 3 P1: jsonl-v1.md add model_call_completed event section" -m "Documents payload shape, example JSON, and H03 use-case for the new sealing event. Cross-refs ADR-0007 Additive variant precedent note. Per Sprint 3 spec §10.1 commit 9." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.10: ROADMAP.md — Sprint 3 checklist precision + Sprint 4 deliverable

**Files:**

- Modify: `ROADMAP.md`

- [ ] **Step 1: Find Sprint 3 + Sprint 4 sections**

```bash
grep -n "^#### Sprint 3\|^#### Sprint 4" ROADMAP.md
```

- [ ] **Step 2: Refine Sprint 3 checklist**

The current Sprint 3 checklist has 4 items. Replace with:

```markdown
#### Sprint 3 · Resume Coordinator (2 days)

- [ ] `EventPayload::ModelCallCompleted { stop_reason, usage }` variant added; schema artifact regenerated; fixture updated (per spec §4 Q1)
- [ ] H03 Resume Coordinator decision table fully implemented (`harness::resume::replay()` covers all 9 decision-table rows from spec §5)
- [ ] `ResumeDecision` shape: `{ point: ResumePoint, last_event_seq: Option<u64> }`; 6 `ResumePoint` variants (per spec §4 §Q2 后续修正)
- [ ] `Runtime::open_session(SessionMode::Resume)` walks the full recovery path (read store → replay → seq init → apply_resume_point)
- [ ] EventId串回完成：Sprint 2 留下的 `recorded_event_id: "unknown"` stub 清理；所有 `record_*` 方法返 `Result<EventId, StoreError>`
- [ ] Chaos test (`crates/cogito-core/tests/resume_chaos.rs`) injects crashes at every event boundary (Y path) + 8 curated panic points (X path) for 4 scenarios
- [ ] All 4 oracles (prefix immutable / terminal equivalent / tool mapping equivalent / final text equivalent) pass for all crash points
- [ ] Resume-from-paused-job scenario validated via `MockJobManager` (real `cogito-jobs` lands Sprint 4)
- [ ] Chaos test total CI time < 10s (verified by `just ci` timing)
```

- [ ] **Step 3: Add Sprint 4 deliverable**

In `#### Sprint 4 · Async Jobs` checklist, after the existing `cogito-jobs implements JobManager` item, add:

```markdown
- [ ] `cogito-jobs` provides cross-process job state persistence (mirrors event log structure; required by Sprint 3 ResumePausedJob path — see Sprint 3 spec §5.6)
```

- [ ] **Step 4: Commit**

```bash
git add ROADMAP.md
git commit -m "Sprint 3 P1: ROADMAP precision + Sprint 4 cross-process job persistence deliverable" -m "Sprint 3 checklist expanded from 4 items to 9, reflecting locked Sprint 3 spec decisions (event variant addition, ResumePoint shape, actor recovery path, EventId threading, chaos test structure + budget). Sprint 4 gets explicit deliverable for cross-process JobManager state persistence — Sprint 3's ResumePausedJob path depends on this contract. Per Sprint 3 spec §10.1 commit 10." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P1.11: Open PR for P1

- [ ] **Step 1: Run full CI to verify all doc edits**

```bash
just ci
```

Expected: PASS. (Doc-only edits shouldn't touch any clippy/test gates.)

- [ ] **Step 2: Push and create PR**

```bash
git push -u origin impl/sprint-3-p1-doc-propagation
gh pr create --title "Sprint 3 P1: doc propagation — actor model section + H03/H02/H06/H01 updates + ADR-0006/0007 amendments" --body "$(cat <<'EOF'
## Summary

- New ARCHITECTURE.md §"Actor model — why and how" + §"Resume entry path" subsection
- ADR-0006 §1 Decision body expanded with four actor-model invariants
- H03 doc full rewrite per Sprint 3 spec §4–§8
- H02 / H06 / H01 docs updated with `ModelCallCompleted` and recovery wiring
- ADR-0007 gains "Additive variant precedent" note (no `SCHEMA_VERSION` bump for new variants)
- `docs/data-model/jsonl-v1.md` documents the new event
- ROADMAP Sprint 3 checklist refined; Sprint 4 deliverable added for cross-process job persistence

Per spec `docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md` §10.1 commits 2–10.

This is a **design-only PR**. No protocol / code changes — implementation lands in P2 onwards.

## Test plan

- [ ] `just ci` green
- [ ] Visually verify ARCHITECTURE.md actor model section renders correctly
- [ ] Confirm H03 doc Interface section matches `crates/cogito-core/src/harness/resume.rs` types (will be enforced by P3 commits)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Surface PR URL to user**

PR URL goes back to the user for review. **Phase P1 ends here.** Wait for PR merge before starting P2.

---

## Phase 2 (P2) · Protocol + Recorder — implementation start

> Adds `ModelCallCompleted` to `EventPayload`, regenerates schema artifact, updates fixture, then updates `step_recorder` (new method + unified `EventId` return) and wires H06 demux to call the new method. Branch: `impl/sprint-3-p2-protocol-recorder`.

### Task P2.1: Create branch

- [ ] **Step 1: Branch off main after P1 merges**

```bash
git checkout main
git pull --ff-only
git checkout -b impl/sprint-3-p2-protocol-recorder
```

### Task P2.2: Add ModelCallCompleted variant to EventPayload (TDD)

**Files:**

- Modify: `crates/cogito-protocol/src/event.rs`
- Test: `crates/cogito-protocol/src/event.rs` (existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write failing test**

Add to `crates/cogito-protocol/src/event.rs` test module (replace `all_nine_variants_roundtrip` test to include 10 variants):

```rust
#[test]
fn all_ten_variants_roundtrip() -> serde_json::Result<()> {
    let variants = vec![
        EventPayload::SessionStarted {
            meta: SessionMeta {
                cogito_version: "0.1.0".into(),
                ..Default::default()
            },
        },
        EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: "go".into() }],
        },
        EventPayload::AssistantMessageAppended { text: "ok".into() },
        EventPayload::ToolUseRecorded {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({"p": 1}),
        },
        EventPayload::ToolResultRecorded {
            call_id: "c1".into(),
            result: ToolResult::text("out"),
        },
        EventPayload::TurnPaused {
            job_id: JobId::default(),
        },
        EventPayload::JobCompletedRecorded {
            job_id: JobId::default(),
            outcome: JobOutcome::Cancelled,
        },
        EventPayload::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
        EventPayload::TurnFailed {
            reason: TurnFailureReason::TurnTimedOut,
        },
        EventPayload::ModelCallCompleted {
            stop_reason: crate::gateway::StopReason::EndTurn,
            usage: crate::gateway::Usage {
                input_tokens: 100,
                output_tokens: 50,
            },
        },
    ];
    for v in variants {
        let event = sample_envelope(v.clone());
        let json = serde_json::to_string(&event)?;
        let back: ConversationEvent = serde_json::from_str(&json)?;
        assert_eq!(event, back, "variant {v:?} did not roundtrip");
    }
    Ok(())
}
```

(Delete the old `all_nine_variants_roundtrip` after adding this.)

- [ ] **Step 2: Run test to verify failure**

```bash
just test -p cogito-protocol all_ten_variants_roundtrip
```

Expected: compilation error (`no variant or associated item named ModelCallCompleted`).

- [ ] **Step 3: Add the variant**

In `crates/cogito-protocol/src/event.rs` `EventPayload` enum, add (placement: after `ModelCallStarted`, before closing brace):

```rust
    /// Recorded by H06 Stream Demultiplexer when the model response
    /// stream emits `MessageCompleted` (Anthropic `message_delta` with
    /// stop_reason / OpenAI `finish_reason`). Sealing event for one
    /// model call. `turn_id` is on the envelope.
    ///
    /// Added in Sprint 3 (2026-05-20) as the first additive variant under
    /// ADR-0007's b-档 compatibility window. No SCHEMA_VERSION bump.
    ModelCallCompleted {
        /// Stop reason as reported by the provider.
        stop_reason: crate::gateway::StopReason,
        /// Token usage for this call.
        usage: crate::gateway::Usage,
    },
```

- [ ] **Step 4: Run test to verify pass**

```bash
just test -p cogito-protocol all_ten_variants_roundtrip
```

Expected: PASS.

- [ ] **Step 5: Run full protocol crate tests**

```bash
just test -p cogito-protocol
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/event.rs
git commit -m "Sprint 3 P2: add ModelCallCompleted event variant" -m "First additive EventPayload variant under ADR-0007 §Additive variant precedent. No SCHEMA_VERSION bump (#[non_exhaustive] + b-档 compatible). Required by H03 to distinguish 'model call done' from 'model call in flight' without re-issuing the gateway request. Per Sprint 3 spec §4 Q1." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P2.3: Regenerate JSON schema artifact

**Files:**

- Modify: `docs/schemas/conversation-event-v1.json` (regenerated)

- [ ] **Step 1: Find the schema generator bin**

```bash
ls crates/cogito-gen-schema/src/ 2>/dev/null || find . -name "gen-schema*" -o -name "*gen_schema*" 2>/dev/null | head -10
```

If `cogito-gen-schema` doesn't exist, check the actual generator path (likely `crates/cogito-protocol/examples/gen_schema.rs` or similar).

- [ ] **Step 2: Run the generator**

```bash
cargo run --bin cogito-gen-schema -- --output docs/schemas/conversation-event-v1.json
# OR (if it's an example):
cargo run --example gen_schema --package cogito-protocol -- docs/schemas/conversation-event-v1.json
```

Expected: file regenerated; `git diff docs/schemas/conversation-event-v1.json` shows new `ModelCallCompleted` block in the `oneOf` / `anyOf` schema.

- [ ] **Step 3: Verify schema validates the new fixture line (anticipatory check, fixture lands next task)**

```bash
cat docs/schemas/conversation-event-v1.json | jq '.definitions.EventPayload // .properties // .' | head -30
grep -c "model_call_completed" docs/schemas/conversation-event-v1.json
```

Expected: `≥1`.

- [ ] **Step 4: Run CI schema drift gate**

```bash
just ci 2>&1 | grep -i "schema"
```

Expected: schema check passes (artifact matches generator output now).

- [ ] **Step 5: Commit**

```bash
git add docs/schemas/conversation-event-v1.json
git commit -m "Sprint 3 P2: regenerate JSON schema artifact for ModelCallCompleted" -m "Auto-regenerated by cogito-gen-schema after the additive variant addition. Drift gate now matches generator output. Per Sprint 3 spec §4 Q1 + ADR-0007 Additive variant precedent." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P2.4: Update canonical fixture

**Files:**

- Modify: `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`

- [ ] **Step 1: Read current fixture**

```bash
cat crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl
```

- [ ] **Step 2: Add a ModelCallCompleted line**

Find the line containing `"type":"model_call_started"` and add a new line immediately after with:

```jsonl
{"schema_version":1,"event_id":"01HZZZ00000000000000000007","session_id":"01HZZZ00000000000000000001","turn_id":"01HZZZ00000000000000000002","seq":6,"ts":"2026-05-20T10:00:06Z","type":"model_call_completed","data":{"stop_reason":"tool_use","usage":{"input_tokens":120,"output_tokens":45}}}
```

(Adjust `event_id` / `session_id` / `turn_id` / `seq` / `ts` to match the existing fixture's progression — use the same session/turn IDs as adjacent events; increment seq from the previous event; pick a timestamp 1 second after the model_call_started.)

- [ ] **Step 3: Verify subsequent seq values are higher than 6**

```bash
grep -o '"seq":[0-9]*' crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl
```

Expected: seq values monotonically increasing; insertion didn't break the ordering. If subsequent events have seq ≤ 6, increment them by 1.

- [ ] **Step 4: Run any fixture validation tests**

```bash
just test -p cogito-test-fixtures
just test -p cogito-protocol fixture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl
git commit -m "Sprint 3 P2: fixture sample-v1.jsonl add model_call_completed line" -m "Inserts the new sealing event between model_call_started and the first downstream tool_use_recorded / assistant_message_appended event, reflecting the H06 demux timing. Per Sprint 3 spec §4 Q1." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P2.5: Unify recorder method signatures to return EventId (TDD)

**Files:**

- Modify: `crates/cogito-core/src/harness/step_recorder.rs` (all `record_*` methods)
- Test: `crates/cogito-core/src/harness/step_recorder.rs` (existing tests if any; add one if not)

- [ ] **Step 1: Write failing test asserting EventId return**

Add or extend a unit test in `step_recorder.rs`:

```rust
#[tokio::test]
async fn record_methods_return_event_id() -> Result<(), Box<dyn std::error::Error>> {
    use cogito_protocol::ids::SessionId;
    let session_id = SessionId::new();
    let store = InMemoryStore::new();
    let mut recorder = StepRecorder::for_test(session_id, store);

    let event_id_1 = recorder
        .record_session_started(SessionMeta::default())
        .await?;
    let event_id_2 = recorder
        .record_turn_started(TurnId::new(), vec![ContentBlock::Text { text: "go".into() }])
        .await?;

    assert_ne!(event_id_1, event_id_2, "EventIds must be unique");
    Ok(())
}
```

(Adapt `for_test` constructor and `InMemoryStore` to match what `step_recorder.rs` exposes for testing; if those helpers don't exist, write minimal ones inside the test module.)

- [ ] **Step 2: Run test to verify failure**

```bash
just test -p cogito-core record_methods_return_event_id
```

Expected: compilation error (return type mismatch).

- [ ] **Step 3: Update `append` helper to return EventId**

In `crates/cogito-core/src/harness/step_recorder.rs`, change `append` signature:

```rust
async fn append(
    &mut self,
    turn_id: Option<TurnId>,
    payload: EventPayload,
) -> Result<EventId, StoreError> {
    let event_id = EventId::new();
    let event = ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: event_id.clone(),
        // ... rest unchanged
    };
    // ... persist as before
    Ok(event_id)
}
```

- [ ] **Step 4: Update all `record_*` methods to return `Result<EventId, StoreError>`**

Methods to update (full list per `step_recorder.rs`):

- `record_session_started`
- `record_turn_started` (if exists; otherwise it's elsewhere)
- `on_text_block_complete`
- `record_tool_use`
- `record_tool_result`
- `record_turn_paused`
- `record_job_completed`
- `record_turn_completed`
- `record_context_manage_entered`
- `record_context_manage_completed`
- `record_prompt_composed`
- `record_model_call_started`
- `record_turn_failed`

For each, change return type from `Result<(), StoreError>` to `Result<EventId, StoreError>` and propagate `self.append(...).await` (which now returns the EventId).

- [ ] **Step 5: Run test to verify pass**

```bash
just test -p cogito-core record_methods_return_event_id
```

Expected: PASS.

- [ ] **Step 6: Update callers**

Now compilation will fail at every caller of `record_*` methods that ignores the EventId. Run:

```bash
cargo build -p cogito-core 2>&1 | grep -i "error\[" | head -30
```

For each error, decide: does the caller need the EventId, or can it be ignored? Apply `let _ = recorder.record_foo(...).await?;` for ignore, or `let event_id = recorder.record_foo(...).await?;` for use cases (most notably `record_turn_failed` — that EventId goes into `TurnOutcome::Failed`).

- [ ] **Step 7: Fix `TurnOutcome::Failed { recorded_event_id }` stub in state.rs**

In `crates/cogito-core/src/harness/turn_driver/state.rs:135`, the current code has:

```rust
TurnState::Failed { reason } => TurnOutcome::Failed {
    reason,
    recorded_event_id: "unknown".into(),
},
```

This stub can't be fixed cleanly inside `into_outcome` because it doesn't have access to the recorder result. Update the FSM to write the `TurnFailed` event before constructing the `Failed` state, capturing the EventId at write time:

In every `transitions/*.rs` that produces `TurnState::Failed { reason }`, change to write the event first and embed the EventId. Example pattern (in `transitions/model_calling.rs`):

```rust
Err(e) => {
    let reason = TurnFailureReason::ModelGatewayFailed {
        message: e.to_string(),
    };
    let event_id = match deps.step.lock().await
        .record_turn_failed(ctx.turn_id, reason.clone())
        .await
    {
        Ok(id) => id,
        Err(_) => {
            // recorder error during failure handling — synthesize a placeholder
            // (this is the unrecoverable case; logged elsewhere)
            EventId::placeholder_for_recorder_failure()
        }
    };
    TurnState::Failed { reason, recorded_event_id: event_id }
}
```

Then update `TurnState::Failed` to carry `recorded_event_id: EventId` and `into_outcome` becomes:

```rust
TurnState::Failed { reason, recorded_event_id } => TurnOutcome::Failed {
    reason,
    recorded_event_id,
},
```

For `EventId::placeholder_for_recorder_failure()`, add to `cogito-protocol::ids`:

```rust
impl EventId {
    /// Placeholder used only when the recorder itself fails to write a
    /// `TurnFailed` event — the actor can't even record its own failure.
    /// Distinguishable from real EventIds (all-zeros ULID).
    pub fn placeholder_for_recorder_failure() -> Self {
        Self::from_string("00000000000000000000000000".into())
    }
}
```

- [ ] **Step 8: Build and run all tests**

```bash
cargo build -p cogito-core
just test -p cogito-core
```

Expected: clean build, all tests green.

- [ ] **Step 9: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs \
        crates/cogito-core/src/harness/turn_driver/state.rs \
        crates/cogito-core/src/harness/turn_driver/transitions/ \
        crates/cogito-protocol/src/ids.rs
git commit -m "Sprint 3 P2: unify recorder return type to Result<EventId, StoreError>" -m "All step_recorder record_* methods now return EventId. Wires into TurnState::Failed { recorded_event_id: EventId }, replacing Sprint 2 's \"unknown\" stub. Adds EventId::placeholder_for_recorder_failure for the unrecoverable case where the recorder itself fails while writing TurnFailed. Per Sprint 3 spec §5.4." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P2.6: Add `record_model_call_completed` + wire H06 demux (TDD)

**Files:**

- Modify: `crates/cogito-core/src/harness/step_recorder.rs`
- Modify: `crates/cogito-core/src/harness/stream_demux.rs`
- Test: `crates/cogito-core/src/harness/stream_demux.rs` (or co-located test module)

- [ ] **Step 1: Write failing test**

Add to `stream_demux.rs` test module (or create one):

```rust
#[tokio::test]
async fn demux_writes_model_call_completed_at_message_completed() -> Result<(), Box<dyn std::error::Error>> {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let store = Arc::new(InMemoryStore::new());
    let mut recorder = StepRecorder::for_test(session_id.clone(), store.clone());

    let events = futures::stream::iter(vec![
        Ok(ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        }),
    ]);

    let output = demux(events, &mut recorder, turn_id.clone()).await?;
    assert_eq!(output.stop_reason, StopReason::EndTurn);

    let logged = store.range(&session_id, ..).await?;
    let mcc = logged.iter().find(|e| matches!(e.payload, EventPayload::ModelCallCompleted { .. }));
    assert!(mcc.is_some(), "expected ModelCallCompleted event in store");
    Ok(())
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
just test -p cogito-core demux_writes_model_call_completed
```

Expected: FAIL (`ModelCallCompleted` event not found, because demux doesn't write it yet).

- [ ] **Step 3: Add `record_model_call_completed` to `step_recorder.rs`**

After `record_model_call_started`:

```rust
/// Record the sealing event for a model call. Called by H06 demux loop
/// when `ModelEvent::MessageCompleted` is observed. Must complete before
/// `demux` returns the sealed `ModelOutput`.
pub async fn record_model_call_completed(
    &mut self,
    turn_id: TurnId,
    stop_reason: StopReason,
    usage: Usage,
) -> Result<EventId, StoreError> {
    self.append(
        Some(turn_id),
        EventPayload::ModelCallCompleted { stop_reason, usage },
    )
    .await
}
```

(Imports at top of file: `use cogito_protocol::gateway::{StopReason, Usage};`.)

- [ ] **Step 4: Wire `demux` to call the recorder on `MessageCompleted`**

In `crates/cogito-core/src/harness/stream_demux.rs`, find the `ModelEvent::MessageCompleted` match arm (around line 77). Currently it just extracts stop_reason/usage. Change to:

```rust
ModelEvent::MessageCompleted {
    stop_reason: sr,
    usage: u,
} => {
    // Sprint 3: persist the sealing event before returning ModelOutput.
    // H03 relies on this to distinguish "model call done" from "in flight".
    recorder
        .record_model_call_completed(turn_id.clone(), sr.clone(), u.clone())
        .await
        .map_err(|e| ModelError::Provider {
            status: 0,
            message: format!("recorder model_call_completed: {e}"),
        })?;
    stop_reason = sr;
    usage = u;
}
```

- [ ] **Step 5: Run test to verify pass**

```bash
just test -p cogito-core demux_writes_model_call_completed
```

Expected: PASS.

- [ ] **Step 6: Run full demux + transitions tests**

```bash
just test -p cogito-core stream_demux
just test -p cogito-core model_calling
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs \
        crates/cogito-core/src/harness/stream_demux.rs
git commit -m "Sprint 3 P2: wire H06 demux to record ModelCallCompleted" -m "Adds step_recorder::record_model_call_completed (sealing event for one model call). demux() calls it when ModelEvent::MessageCompleted is observed, before returning the sealed ModelOutput. Preserves the 'sealing event before any tool dispatch event' causal ordering invariant required by H03. Per Sprint 3 spec §4 Q1 + H06 doc Recorder invocation timing." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P2.7: Open PR for P2

- [ ] **Step 1: Full CI**

```bash
just ci
```

Expected: all green; schema drift check passes (artifact matches generator).

- [ ] **Step 2: Push + PR**

```bash
git push -u origin impl/sprint-3-p2-protocol-recorder
gh pr create --title "Sprint 3 P2: protocol + recorder — ModelCallCompleted + unified EventId return" --body "$(cat <<'EOF'
## Summary

- `EventPayload::ModelCallCompleted { stop_reason, usage }` added (additive, no SCHEMA_VERSION bump)
- `docs/schemas/conversation-event-v1.json` regenerated
- `sample-v1.jsonl` fixture updated
- `step_recorder` all `record_*` methods now return `Result<EventId, StoreError>`
- `TurnState::Failed { recorded_event_id: EventId }` replaces Sprint 2's "unknown" stub
- H06 `demux` wired to call `record_model_call_completed` on `ModelEvent::MessageCompleted`

Per spec §4 Q1, §5.4. Requires P1 (doc propagation) merged.

## Test plan

- [ ] `just ci` green
- [ ] `just test -p cogito-protocol all_ten_variants_roundtrip` green
- [ ] `just test -p cogito-core demux_writes_model_call_completed` green
- [ ] Manual: `cogito chat` end-to-end still works (Sprint 2 happy path unaffected)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Surface PR URL. Phase P2 ends.**

---

## Phase 3 (P3) · H03 + Turn FSM — the core resume logic

> Implements `ResumeDecision` / `ResumePoint` / `ResumeError` types, the `replay()` decision table algorithm, the `TurnEntry` enum, and updates `enter_turn` to consume `TurnEntry`. Branch: `impl/sprint-3-p3-h03-replay`.

### Task P3.1: Create branch

- [ ] **Step 1**

```bash
git checkout main
git pull --ff-only
git checkout -b impl/sprint-3-p3-h03-replay
```

### Task P3.2: Define `ResumeDecision` / `ResumePoint` / `ResumePendingCall` / `ResumeError` types

**Files:**

- Modify: `crates/cogito-core/src/harness/resume.rs` (replace Sprint 2 stub types)

- [ ] **Step 1: Read current resume.rs**

```bash
cat crates/cogito-core/src/harness/resume.rs
```

- [ ] **Step 2: Replace types section**

Replace the file content (keeping the module-level doc comment with `Sprint 3` updates) with:

```rust
//! H03 Resume Coordinator — pure function from event log to resume decision.
//!
//! Sprint 3 implements the full decision table per spec §4–§5.
//! Pure function: same input → same output, no I/O, no clock, no random.
//! The actor calls `replay()` on startup and uses the result to bootstrap
//! either the FSM (via `TurnEntry`) or its own `InFlight` state.

use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
use cogito_protocol::gateway::ModelOutput;
use cogito_protocol::ids::TurnId;
use cogito_protocol::job::{JobId, JobOutcome};
use cogito_protocol::tool::ToolResult;

/// Output of H03 Resume Coordinator. Pure projection from the event log.
/// Never persisted (see spec §6 落盘语义).
#[derive(Debug, Clone)]
pub struct ResumeDecision {
    /// What state to resume into.
    pub point: ResumePoint,
    /// `seq` of the last event in the log when this decision was computed.
    /// `None` iff `point == FreshTurn` AND the log is empty.
    /// Actor initializes its event seq generator to `last_event_seq + 1`.
    pub last_event_seq: Option<u64>,
}

/// Resume entry point. Six variants covering every valid log shape.
#[derive(Debug, Clone)]
pub enum ResumePoint {
    /// Empty log, or last turn ended in `TurnCompleted` / `TurnFailed`.
    /// Actor idles until the next user `Input`.
    FreshTurn,

    /// In-flight turn where the most recent model call did not complete
    /// (no `ModelCallCompleted` after the latest `ModelCallStarted`).
    /// FSM enters `Init`; H04 rebuilds prompt from the event log; one
    /// model call gets re-billed.
    RestartCurrentTurn { turn_id: TurnId },

    /// Most recent `ModelCallCompleted` is the latest event in the turn
    /// AND no `ToolUseRecorded` follows. Actor crashed between writing
    /// the sealing event and writing `TurnCompleted`. FSM enters
    /// `ModelCompleted` with output rebuilt from events; fast-paths to
    /// `Completed` without re-calling the model.
    ResumeFromModelCompleted {
        turn_id: TurnId,
        rebuilt_output: ModelOutput,
    },

    /// Tool dispatch round in progress. May have 0+ completed results.
    /// FSM enters `ToolDispatching`. `enter_turn` re-runs H07 on `pending`
    /// to re-validate against current schemas, and triggers H10+H05 to
    /// rebuild the tool surface.
    ResumeFromToolDispatching {
        turn_id: TurnId,
        /// `ToolUseRecorded` since the latest `ModelCallCompleted` with no
        /// matching `ToolResultRecorded`. Order preserved from the log.
        pending: Vec<ResumePendingCall>,
        /// `(call_id, ToolResult)` pairs already in the log.
        completed: Vec<(String, ToolResult)>,
    },

    /// Turn paused on an async job. `TurnPaused` is the latest event;
    /// no `JobCompletedRecorded { job_id }` follows. Actor enters
    /// `InFlight::PausedOnJob` and re-registers `on_complete`.
    ResumePausedJob { turn_id: TurnId, job_id: JobId },

    /// Async job completed but Brain didn't consume the
    /// `JobCompletedRecorded` event before the crash. FSM enters
    /// `ToolDispatching` with the just-completed result injected as the
    /// last entry of `completed_before_pause` + `call_id` resolved.
    ResumeAfterJobCompletion {
        turn_id: TurnId,
        job_id: JobId,
        outcome: JobOutcome,
        /// Resolved by walking back to the latest unmatched `ToolUseRecorded`
        /// before `TurnPaused` (Sprint 3 invariant: ≤1 async dispatch per
        /// turn; Sprint 4 may add `call_id` to `TurnPaused` payload).
        call_id: String,
        /// Tool calls dispatched and completed before the pause.
        completed_before_pause: Vec<(String, ToolResult)>,
        /// Tool calls declared by the model but not yet dispatched at
        /// pause time. (Sprint 3 always empty; Sprint 4 may be non-empty.)
        pending_after_pause: Vec<ResumePendingCall>,
    },
}

/// Raw tool-call triple recovered from a `ToolUseRecorded` event.
/// Pre-validation — `enter_turn` re-runs through H07 before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePendingCall {
    pub call_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
}

/// Errors from `replay`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    /// Event log contradicts itself (e.g., `JobCompletedRecorded` with no
    /// matching prior `TurnPaused`; nested `TurnStarted` without
    /// terminator).
    #[error("malformed event log: {0}")]
    Malformed(String),
    /// Event log was written by a newer schema version than this build supports.
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    /// A tool referenced by a recovered call is no longer registered.
    /// (Sprint 3 returns this from `enter_turn` re-validation, not from
    /// `replay()` itself; reserved here for completeness.)
    #[error("tool `{tool_name}` (call_id `{call_id}`) no longer registered")]
    ToolUnavailable { call_id: String, tool_name: String },
    /// Persisted tool args fail current schema validation.
    #[error("tool `{tool_name}` schema rejects persisted args: {reason}")]
    ToolSchemaDrift { tool_name: String, reason: String },
}

/// Sprint 3 stub: see Task P3.3 for full algorithm. Placeholder so this
/// commit compiles; next commit replaces the body.
pub fn replay(_events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    Ok(ResumeDecision {
        point: ResumePoint::FreshTurn,
        last_event_seq: None,
    })
}
```

- [ ] **Step 3: Build to verify types compile**

```bash
cargo build -p cogito-core
```

Expected: clean build (note: any callers of old `ResumeDecision` variants like `FreshTurn` direct construction will fail — those are in `turn_driver/mod.rs` `enter_turn`; we'll fix that in P3.5).

For now, temporarily patch the callsites to use the new shape — `enter_turn` can match on `decision.point` instead of the bare enum.

In `crates/cogito-core/src/harness/turn_driver/mod.rs`, change the `enter_turn` signature/body to accept the new `ResumeDecision`:

```rust
pub async fn enter_turn(decision: ResumeDecision, ctx: TurnCtx, deps: TurnDeps) -> TurnOutcome {
    let initial = match decision.point {
        ResumePoint::FreshTurn => TurnState::Init {
            ctx,
            resume: ResumeDecision {
                point: ResumePoint::FreshTurn,
                last_event_seq: decision.last_event_seq,
            },
        },
        // Other variants will be added in P3.5; for now panic in dev
        // (real callers don't yet construct these).
        _ => unreachable!("Sprint 3 P3.5 will handle non-FreshTurn variants"),
    };
    run(initial, &deps).await
}
```

Also update `crates/cogito-core/src/runtime/actor.rs:226` to use the new shape:

```rust
let decision = replay(&[]).unwrap_or(ResumeDecision {
    point: ResumePoint::FreshTurn,
    last_event_seq: None,
});
```

- [ ] **Step 4: Build + run tests for cogito-core**

```bash
cargo build -p cogito-core
just test -p cogito-core
```

Expected: clean build, existing Sprint 2 tests pass (the `unreachable!` doesn't trigger because Sprint 2 callers only produce `FreshTurn`).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/resume.rs \
        crates/cogito-core/src/harness/turn_driver/mod.rs \
        crates/cogito-core/src/runtime/actor.rs
git commit -m "Sprint 3 P3: define ResumeDecision / ResumePoint / ResumeError types" -m "Replaces Sprint 2 stub types with the six-variant ResumePoint enum and the wrapper ResumeDecision struct with last_event_seq. ResumePendingCall and ResumeError landed alongside. replay() is still a FreshTurn stub — next commit implements the decision table. Per Sprint 3 spec §4 §Q2 后续修正." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P3.3: Implement H03 `replay()` decision table (TDD per-row)

**Files:**

- Modify: `crates/cogito-core/src/harness/resume.rs` (replace stub `replay` body)
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write failing tests for each decision table row**

Add a new `tests` module at the bottom of `resume.rs`:

```rust
#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::gateway::{StopReason, Usage};
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::session::SessionMeta;
    use cogito_protocol::tool::ToolResult;
    use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
    use chrono::Utc;

    fn evt(seq: u64, payload: EventPayload, turn: Option<TurnId>) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: turn,
            seq,
            ts: Utc::now(),
            payload,
        }
    }

    #[test]
    fn empty_log_returns_fresh_turn() {
        let d = replay(&[]).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, None);
    }

    #[test]
    fn only_session_started_returns_fresh_turn() {
        let events = vec![evt(0, EventPayload::SessionStarted { meta: SessionMeta::default() }, None)];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, Some(0));
    }

    #[test]
    fn turn_completed_returns_fresh_turn() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::SessionStarted { meta: SessionMeta::default() }, None),
            evt(1, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(2, EventPayload::TurnCompleted { outcome: TurnOutcome::Completed }, Some(t)),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
        assert_eq!(d.last_event_seq, Some(2));
    }

    #[test]
    fn turn_failed_returns_fresh_turn() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::TurnFailed { reason: TurnFailureReason::TurnTimedOut }, Some(t)),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::FreshTurn));
    }

    #[test]
    fn turn_started_no_model_call_returns_restart() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::PromptComposed { model: "m".into(), surface_size: 0 }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::RestartCurrentTurn { turn_id } => assert_eq!(turn_id, t),
            other => panic!("expected RestartCurrentTurn, got {other:?}"),
        }
    }

    #[test]
    fn model_call_started_no_completed_returns_restart() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::ModelCallStarted { model: "m".into() }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        assert!(matches!(d.point, ResumePoint::RestartCurrentTurn { .. }));
    }

    #[test]
    fn model_call_completed_no_tool_use_returns_resume_from_model_completed() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::ModelCallStarted { model: "m".into() }, Some(t.clone())),
            evt(2, EventPayload::AssistantMessageAppended { text: "hi".into() }, Some(t.clone())),
            evt(3, EventPayload::ModelCallCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 5, output_tokens: 5 },
            }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromModelCompleted { turn_id, rebuilt_output } => {
                assert_eq!(turn_id, t);
                assert_eq!(rebuilt_output.stop_reason, StopReason::EndTurn);
                assert!(rebuilt_output.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text == "hi")));
            }
            other => panic!("expected ResumeFromModelCompleted, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_recorded_no_results_returns_resume_from_tool_dispatching() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::ModelCallStarted { model: "m".into() }, Some(t.clone())),
            evt(2, EventPayload::ModelCallCompleted {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            }, Some(t.clone())),
            evt(3, EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({"p": "/x"}),
            }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromToolDispatching { turn_id, pending, completed } => {
                assert_eq!(turn_id, t);
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].call_id, "c1");
                assert!(completed.is_empty());
            }
            other => panic!("expected ResumeFromToolDispatching, got {other:?}"),
        }
    }

    #[test]
    fn partial_tool_results_returns_resume_from_tool_dispatching_with_split() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::ModelCallStarted { model: "m".into() }, Some(t.clone())),
            evt(2, EventPayload::ModelCallCompleted {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            }, Some(t.clone())),
            evt(3, EventPayload::ToolUseRecorded {
                call_id: "c1".into(), tool_name: "tool_a".into(), args: serde_json::json!({}),
            }, Some(t.clone())),
            evt(4, EventPayload::ToolUseRecorded {
                call_id: "c2".into(), tool_name: "tool_b".into(), args: serde_json::json!({}),
            }, Some(t.clone())),
            evt(5, EventPayload::ToolResultRecorded {
                call_id: "c1".into(), result: ToolResult::text("ok"),
            }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeFromToolDispatching { pending, completed, .. } => {
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].call_id, "c2");
                assert_eq!(completed.len(), 1);
                assert_eq!(completed[0].0, "c1");
            }
            other => panic!("expected partial split, got {other:?}"),
        }
    }

    #[test]
    fn turn_paused_no_job_completed_returns_resume_paused_job() {
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::TurnPaused { job_id: j.clone() }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumePausedJob { turn_id, job_id } => {
                assert_eq!(turn_id, t);
                assert_eq!(job_id, j);
            }
            other => panic!("expected ResumePausedJob, got {other:?}"),
        }
    }

    #[test]
    fn job_completed_after_paused_returns_resume_after_job_completion() {
        let t = TurnId::new();
        let j = JobId::default();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::ModelCallStarted { model: "m".into() }, Some(t.clone())),
            evt(2, EventPayload::ModelCallCompleted {
                stop_reason: StopReason::ToolUse, usage: Usage::default(),
            }, Some(t.clone())),
            evt(3, EventPayload::ToolUseRecorded {
                call_id: "c_async".into(),
                tool_name: "long_tool".into(),
                args: serde_json::json!({}),
            }, Some(t.clone())),
            evt(4, EventPayload::TurnPaused { job_id: j.clone() }, Some(t.clone())),
            evt(5, EventPayload::JobCompletedRecorded {
                job_id: j.clone(),
                outcome: JobOutcome::Cancelled, // representative; any variant
            }, Some(t.clone())),
        ];
        let d = replay(&events).unwrap();
        match d.point {
            ResumePoint::ResumeAfterJobCompletion { turn_id, job_id, call_id, .. } => {
                assert_eq!(turn_id, t);
                assert_eq!(job_id, j);
                assert_eq!(call_id, "c_async");
            }
            other => panic!("expected ResumeAfterJobCompletion, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_schema_version_returns_error() {
        let mut e = evt(0, EventPayload::SessionStarted { meta: SessionMeta::default() }, None);
        e.schema_version = SCHEMA_VERSION + 1;
        let err = replay(&[e]).unwrap_err();
        assert!(matches!(err, ResumeError::UnsupportedSchema(_)));
    }

    #[test]
    fn job_completed_without_matching_paused_is_malformed() {
        let t = TurnId::new();
        let events = vec![
            evt(0, EventPayload::TurnStarted { user_input: vec![] }, Some(t.clone())),
            evt(1, EventPayload::JobCompletedRecorded {
                job_id: JobId::default(),
                outcome: JobOutcome::Cancelled,
            }, Some(t)),
        ];
        let err = replay(&events).unwrap_err();
        assert!(matches!(err, ResumeError::Malformed(_)));
    }
}
```

- [ ] **Step 2: Run tests to verify failures**

```bash
just test -p cogito-core resume::tests
```

Expected: all 12 tests fail (stub always returns FreshTurn / never errors).

- [ ] **Step 3: Implement the decision table algorithm**

Replace `pub fn replay` body in `resume.rs`:

```rust
pub fn replay(events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    // ① schema check (must come first)
    if let Some(e) = events.iter().find(|e| e.schema_version > SCHEMA_VERSION) {
        return Err(ResumeError::UnsupportedSchema(e.schema_version));
    }

    let last_event_seq = events.last().map(|e| e.seq);

    if events.is_empty() {
        return Ok(ResumeDecision { point: ResumePoint::FreshTurn, last_event_seq });
    }

    // ② find the latest turn-boundary event (TurnStarted / TurnCompleted /
    // TurnFailed / TurnPaused)
    let boundary_idx = events.iter().enumerate().rev().find_map(|(i, e)| {
        match &e.payload {
            EventPayload::TurnStarted { .. }
            | EventPayload::TurnCompleted { .. }
            | EventPayload::TurnFailed { .. }
            | EventPayload::TurnPaused { .. } => Some(i),
            _ => None,
        }
    });

    let Some(boundary_idx) = boundary_idx else {
        // Only SessionStarted (or pre-turn events) — FreshTurn
        return Ok(ResumeDecision { point: ResumePoint::FreshTurn, last_event_seq });
    };

    let boundary = &events[boundary_idx];

    // Detect malformed: JobCompletedRecorded without preceding TurnPaused
    if let Some(last) = events.last() {
        if let EventPayload::JobCompletedRecorded { job_id, .. } = &last.payload {
            // Find the matching TurnPaused before
            let paused_before = events[..events.len() - 1].iter().rev().find_map(|e| {
                if let EventPayload::TurnPaused { job_id: jid } = &e.payload {
                    if jid == job_id {
                        return Some(true);
                    }
                }
                None
            });
            if paused_before.is_none() {
                return Err(ResumeError::Malformed(format!(
                    "JobCompletedRecorded for job_id={job_id:?} with no preceding TurnPaused"
                )));
            }
        }
    }

    match &boundary.payload {
        EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. } => {
            Ok(ResumeDecision { point: ResumePoint::FreshTurn, last_event_seq })
        }
        EventPayload::TurnPaused { job_id } => {
            // Check for following JobCompletedRecorded
            let tail = &events[boundary_idx + 1..];
            let job_done = tail.iter().find_map(|e| match &e.payload {
                EventPayload::JobCompletedRecorded { job_id: jid, outcome } if jid == job_id => {
                    Some(outcome.clone())
                }
                _ => None,
            });

            // Recover turn_id from the boundary event envelope
            let turn_id = boundary
                .turn_id
                .clone()
                .ok_or_else(|| ResumeError::Malformed("TurnPaused without turn_id".into()))?;

            match job_done {
                None => Ok(ResumeDecision {
                    point: ResumePoint::ResumePausedJob {
                        turn_id,
                        job_id: job_id.clone(),
                    },
                    last_event_seq,
                }),
                Some(outcome) => {
                    // Walk back from TurnPaused to find the last unmatched ToolUseRecorded
                    let (call_id, completed_before_pause, pending_after_pause) =
                        find_paused_call_context(events, boundary_idx)?;
                    Ok(ResumeDecision {
                        point: ResumePoint::ResumeAfterJobCompletion {
                            turn_id,
                            job_id: job_id.clone(),
                            outcome,
                            call_id,
                            completed_before_pause,
                            pending_after_pause,
                        },
                        last_event_seq,
                    })
                }
            }
        }
        EventPayload::TurnStarted { .. } => {
            let turn_id = boundary
                .turn_id
                .clone()
                .ok_or_else(|| ResumeError::Malformed("TurnStarted without turn_id".into()))?;

            // In-flight turn slice
            let turn_slice = &events[boundary_idx..];

            // Find latest ModelCallStarted and ModelCallCompleted within turn
            let latest_mcs = turn_slice.iter().rposition(|e| {
                matches!(e.payload, EventPayload::ModelCallStarted { .. })
            });
            let latest_mcc = turn_slice.iter().rposition(|e| {
                matches!(e.payload, EventPayload::ModelCallCompleted { .. })
            });

            match (latest_mcs, latest_mcc) {
                (None, _) | (Some(_), None) => Ok(ResumeDecision {
                    point: ResumePoint::RestartCurrentTurn { turn_id },
                    last_event_seq,
                }),
                (Some(s), Some(c)) if s > c => Ok(ResumeDecision {
                    point: ResumePoint::RestartCurrentTurn { turn_id },
                    last_event_seq,
                }),
                (Some(s), Some(c)) => {
                    debug_assert!(c >= s);
                    // Events after latest_mcc: tool dispatch round in progress
                    let after_mcc = &turn_slice[c + 1..];

                    let mut pending: Vec<ResumePendingCall> = Vec::new();
                    let mut completed: Vec<(String, ToolResult)> = Vec::new();
                    let mut completed_ids: Vec<&str> = Vec::new();

                    for e in after_mcc {
                        if let EventPayload::ToolResultRecorded { call_id, result } = &e.payload {
                            completed.push((call_id.clone(), result.clone()));
                            completed_ids.push(call_id);
                        }
                    }

                    for e in after_mcc {
                        if let EventPayload::ToolUseRecorded { call_id, tool_name, args } = &e.payload {
                            if !completed_ids.contains(&call_id.as_str()) {
                                pending.push(ResumePendingCall {
                                    call_id: call_id.clone(),
                                    tool_name: tool_name.clone(),
                                    args: args.clone(),
                                });
                            }
                        }
                    }

                    if pending.is_empty() && completed.is_empty() {
                        // ModelCallCompleted is last + no tools → rebuild output
                        let rebuilt_output = rebuild_model_output(&turn_slice[s..=c])?;
                        Ok(ResumeDecision {
                            point: ResumePoint::ResumeFromModelCompleted {
                                turn_id,
                                rebuilt_output,
                            },
                            last_event_seq,
                        })
                    } else {
                        Ok(ResumeDecision {
                            point: ResumePoint::ResumeFromToolDispatching {
                                turn_id,
                                pending,
                                completed,
                            },
                            last_event_seq,
                        })
                    }
                }
            }
        }
        _ => Err(ResumeError::Malformed(format!(
            "unexpected boundary event payload: {:?}",
            boundary.payload
        ))),
    }
}

/// Reconstruct `ModelOutput` from the slice spanning `ModelCallStarted` to
/// `ModelCallCompleted` (inclusive). Walks `AssistantMessageAppended` and
/// `ToolUseRecorded` events into ContentBlocks; pulls stop_reason + usage
/// from the closing `ModelCallCompleted`.
fn rebuild_model_output(slice: &[ConversationEvent]) -> Result<ModelOutput, ResumeError> {
    let mut content: Vec<cogito_protocol::content::ContentBlock> = Vec::new();
    let mut stop_reason = None;
    let mut usage = None;

    for e in slice {
        match &e.payload {
            EventPayload::AssistantMessageAppended { text } => {
                content.push(cogito_protocol::content::ContentBlock::Text { text: text.clone() });
            }
            EventPayload::ToolUseRecorded { call_id, tool_name, args } => {
                content.push(cogito_protocol::content::ContentBlock::ToolUse {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                });
            }
            EventPayload::ModelCallCompleted { stop_reason: sr, usage: u } => {
                stop_reason = Some(sr.clone());
                usage = Some(u.clone());
            }
            _ => {}
        }
    }

    Ok(ModelOutput {
        content,
        stop_reason: stop_reason
            .ok_or_else(|| ResumeError::Malformed("rebuild_model_output: no ModelCallCompleted".into()))?,
        usage: usage.unwrap_or_default(),
    })
}

/// For `ResumeAfterJobCompletion`: walk back from `paused_idx` to find the
/// latest unmatched `ToolUseRecorded`. Sprint 3 invariant: ≤1 async
/// dispatch per turn.
fn find_paused_call_context(
    events: &[ConversationEvent],
    paused_idx: usize,
) -> Result<(String, Vec<(String, ToolResult)>, Vec<ResumePendingCall>), ResumeError> {
    // Find latest ModelCallCompleted before paused_idx
    let mcc_idx = events[..paused_idx].iter().rposition(|e| {
        matches!(e.payload, EventPayload::ModelCallCompleted { .. })
    });
    let start = mcc_idx.map(|i| i + 1).unwrap_or(0);
    let dispatch_slice = &events[start..paused_idx];

    let mut completed: Vec<(String, ToolResult)> = Vec::new();
    let mut completed_ids: Vec<&str> = Vec::new();
    let mut all_tool_uses: Vec<(&str, &str, &serde_json::Value)> = Vec::new();

    for e in dispatch_slice {
        match &e.payload {
            EventPayload::ToolUseRecorded { call_id, tool_name, args } => {
                all_tool_uses.push((call_id, tool_name, args));
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                completed.push((call_id.clone(), result.clone()));
                completed_ids.push(call_id);
            }
            _ => {}
        }
    }

    // The "paused on" call is the first ToolUseRecorded without a matching ToolResultRecorded
    let paused_call = all_tool_uses
        .iter()
        .find(|(id, _, _)| !completed_ids.contains(id))
        .ok_or_else(|| ResumeError::Malformed("TurnPaused without preceding unmatched ToolUseRecorded".into()))?;
    let call_id = paused_call.0.to_string();

    // pending_after_pause: tool_uses after the paused call that haven't been completed
    let mut pending_after_pause: Vec<ResumePendingCall> = Vec::new();
    let mut after_paused = false;
    for (id, name, args) in &all_tool_uses {
        if after_paused && !completed_ids.contains(id) {
            pending_after_pause.push(ResumePendingCall {
                call_id: id.to_string(),
                tool_name: name.to_string(),
                args: (*args).clone(),
            });
        }
        if id == &call_id.as_str() {
            after_paused = true;
        }
    }

    Ok((call_id, completed, pending_after_pause))
}
```

- [ ] **Step 4: Run tests to verify pass**

```bash
just test -p cogito-core resume::tests
```

Expected: all 12 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/resume.rs
git commit -m "Sprint 3 P3: implement H03 replay() decision table" -m "Implements the full 9-row decision table from spec §5 over the 14 EventPayload variants. Pure function; O(N) single-pass with backward boundary scan + forward in-turn classification. Covers all 6 ResumePoint variants plus 3 ResumeError paths. 12 unit tests pin every decision table row + key edge cases (multi-tool partial dispatch, paused-then-completed). Per Sprint 3 spec §4–§5." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P3.4: Define `TurnEntry` and update `enter_turn`

**Files:**

- Modify: `crates/cogito-core/src/harness/turn_driver/mod.rs`

- [ ] **Step 1: Define `TurnEntry`**

Add to `crates/cogito-core/src/harness/turn_driver/mod.rs`:

```rust
/// Harness-internal translation of `ResumePoint` into the FSM-level shape
/// `enter_turn` consumes. Three variants because `FreshTurn` /
/// `ResumePausedJob` are actor-level (handled before `enter_turn` is
/// called).
pub(crate) enum TurnEntry {
    /// FSM enters Init. H04 rebuilds prompt; H10 re-selects strategy.
    FreshLikeInit,
    /// FSM enters ModelCompleted with rebuilt output; fast-paths to Completed.
    FromModelCompleted {
        output: cogito_protocol::gateway::ModelOutput,
    },
    /// FSM enters ToolDispatching with pending/completed pre-populated.
    /// H07 re-validates pending; H10+H05 rebuild surface.
    FromToolDispatching {
        pending: Vec<crate::harness::resume::ResumePendingCall>,
        completed: Vec<(String, cogito_protocol::tool::ToolResult)>,
    },
}
```

- [ ] **Step 2: Update `enter_turn` to accept `TurnEntry`**

Change `enter_turn` signature from `ResumeDecision` to `TurnEntry`:

```rust
pub async fn enter_turn(entry: TurnEntry, ctx: TurnCtx, deps: TurnDeps) -> TurnOutcome {
    let initial = match entry {
        TurnEntry::FreshLikeInit => TurnState::Init {
            ctx,
            // Pass FreshTurn to keep Sprint 2 Init transition logic compatible
            resume: ResumeDecision {
                point: ResumePoint::FreshTurn,
                last_event_seq: None,
            },
        },
        TurnEntry::FromModelCompleted { output } => {
            // Need to rebuild surface via H10+H05 before entering ModelCompleted
            let strategy = deps.strategy.select(&ctx.session_id).await;
            let surface = deps.tool_surface.build(&strategy);
            TurnState::ModelCompleted { ctx, output, surface }
        }
        TurnEntry::FromToolDispatching { pending, completed } => {
            let strategy = deps.strategy.select(&ctx.session_id).await;
            let surface = deps.tool_surface.build(&strategy);
            // Re-resolve pending through H07
            let resolved: Result<Vec<ToolInvocation>, _> = pending
                .into_iter()
                .map(|p| crate::harness::tool_resolver::resolve_one(
                    p.call_id, p.tool_name, p.args, &surface,
                ))
                .collect();
            match resolved {
                Ok(invocations) => TurnState::ToolDispatching {
                    ctx,
                    pending: invocations.into(),
                    completed,
                    surface,
                },
                Err(e) => TurnState::Failed {
                    reason: cogito_protocol::turn::TurnFailureReason::ResumeFailed {
                        message: format!("tool re-resolve: {e}"),
                    },
                    recorded_event_id: EventId::placeholder_for_recorder_failure(),
                },
            }
        }
    };
    run(initial, &deps).await
}
```

> If `cogito_protocol::turn::TurnFailureReason::ResumeFailed` doesn't exist, add it as a new variant under `#[non_exhaustive]` in `cogito-protocol::turn`. This is a small protocol addition; document in the next commit.

- [ ] **Step 3: Build to verify compilation**

```bash
cargo build -p cogito-core
```

Adjust any compile errors (e.g., `deps.strategy.select` / `tool_surface.build` exact API names — verify against existing Sprint 2 transitions to copy the same calls).

- [ ] **Step 4: Verify Sprint 2 tests still pass**

```bash
just test -p cogito-core
```

Expected: all existing tests pass (only `FreshLikeInit` callsite is exercised; others are still unreachable until P4 wires actor recovery).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/turn_driver/mod.rs \
        crates/cogito-protocol/src/turn.rs
git commit -m "Sprint 3 P3: define TurnEntry; enter_turn consumes 3-variant FSM input" -m "TurnEntry is harness-internal (pub(crate)) translation from actor-level ResumePoint to FSM input. enter_turn now rebuilds surface via H10+H05 for non-FreshTurn entries, and re-resolves pending tool calls via H07. Adds TurnFailureReason::ResumeFailed for re-resolve failures. Per Sprint 3 spec §5.3 + H01 doc Resume entry path." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P3.5: Open PR for P3

- [ ] **Step 1: Full CI**

```bash
just ci
```

- [ ] **Step 2: Push + PR**

```bash
git push -u origin impl/sprint-3-p3-h03-replay
gh pr create --title "Sprint 3 P3: H03 replay decision table + TurnEntry FSM entry" --body "$(cat <<'EOF'
## Summary

- `ResumeDecision { point: ResumePoint, last_event_seq }` types in `harness::resume`
- `ResumePoint` 6 variants, `ResumePendingCall`, `ResumeError` 4 variants
- `replay()` decision table fully implemented (9 rows, 12 unit tests pinning each)
- `TurnEntry` harness-internal enum (`pub(crate)`) with 3 variants
- `enter_turn` updated: rebuilds surface via H10+H05; re-validates pending via H07
- `TurnFailureReason::ResumeFailed` added for tool re-resolve failures

Per spec §4–§5. Requires P2 merged. Does NOT yet wire actor recovery — that's P4.

## Test plan

- [ ] `just ci` green
- [ ] `just test -p cogito-core resume::tests` — 12 tests green
- [ ] All Sprint 2 tests still pass (no regression on happy path)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Phase P3 ends.**

---

## Phase 4 (P4) · Runtime actor recovery wiring

> Updates `Runtime::open_session` to dispatch by `SessionMode`, `actor_main` to call `replay()` and `apply_resume_point`, and adds the new `RuntimeError` / `ShutdownOutcome` variants. Branch: `impl/sprint-3-p4-actor-recovery`.

### Task P4.1: Create branch

- [ ] **Step 1**

```bash
git checkout main
git pull --ff-only
git checkout -b impl/sprint-3-p4-actor-recovery
```

### Task P4.2: Add new RuntimeError / ShutdownOutcome variants

**Files:**

- Modify: `crates/cogito-core/src/runtime/types.rs`

- [ ] **Step 1: Find current `RuntimeError` + `ShutdownOutcome`**

```bash
grep -n "RuntimeError\|ShutdownOutcome" crates/cogito-core/src/runtime/types.rs
```

- [ ] **Step 2: Add variants**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeError {
    // ... existing variants ...
    #[error("session {id:?} already exists in store")]
    SessionAlreadyExists { id: SessionId },
    #[error("resume failed for session {id:?}: {reason}")]
    ResumeFailed { id: SessionId, reason: String },
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    // ... existing variants ...
    /// Resume failed before mailbox loop started.
    ResumeFailed(String),
    /// JobManager couldn't honor on_complete callback (job unknown).
    JobManagerUnavailable(String),
}
```

- [ ] **Step 3: Build to verify**

```bash
cargo build -p cogito-core
```

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/src/runtime/types.rs
git commit -m "Sprint 3 P4: add RuntimeError + ShutdownOutcome variants for resume" -m "RuntimeError gains SessionAlreadyExists (New mode collision) and ResumeFailed (Resume mode but no log). ShutdownOutcome gains ResumeFailed (post-startup schema / malformed log) and JobManagerUnavailable (on_complete failure during PausedOnJob recovery). Per Sprint 3 spec §5.5." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P4.3: Implement `open_session` SessionMode dispatch

**Files:**

- Modify: `crates/cogito-core/src/runtime/builder.rs`

- [ ] **Step 1: Read current `open_session`**

```bash
sed -n '40,100p' crates/cogito-core/src/runtime/builder.rs
```

- [ ] **Step 2: Write failing test**

Add to `cogito-core/tests/` (new file or existing runtime test):

```rust
// crates/cogito-core/tests/runtime_session_mode.rs
use cogito_core::runtime::{Runtime, RuntimeBuilder, SessionMode, RuntimeError};
use cogito_protocol::ids::SessionId;
// ... (set up runtime with InMemoryStore, etc.)

#[tokio::test]
async fn open_session_resume_missing_returns_resume_failed() {
    let runtime = build_test_runtime().await;
    let id = SessionId::new();
    let result = runtime.open_session(id.clone(), SessionMode::Resume).await;
    assert!(matches!(result, Err(RuntimeError::ResumeFailed { id: ref e_id, .. }) if e_id == &id));
}

#[tokio::test]
async fn open_session_new_existing_returns_session_already_exists() {
    let runtime = build_test_runtime().await;
    let id = SessionId::new();
    let _first = runtime.open_session(id.clone(), SessionMode::New).await.unwrap();
    let second = runtime.open_session(id.clone(), SessionMode::New).await;
    assert!(matches!(second, Err(RuntimeError::SessionAlreadyExists { .. })));
}
```

- [ ] **Step 3: Run tests to verify failures**

```bash
just test -p cogito-core runtime_session_mode
```

Expected: FAIL (mode dispatch not implemented).

- [ ] **Step 4: Implement dispatch**

In `crates/cogito-core/src/runtime/builder.rs::open_session`:

```rust
pub async fn open_session(
    self: &Arc<Self>,
    id: SessionId,
    mode: SessionMode,
) -> Result<SessionHandle, RuntimeError> {
    // Registry check unchanged
    if self.shared.session_registry.contains(&id).await {
        return Err(RuntimeError::SessionAlreadyOpen { id });
    }

    let initial_events: Vec<ConversationEvent> = match mode {
        SessionMode::New => {
            let existing = self.shared.store.range(&id, ..).await
                .map_err(|e| RuntimeError::StoreError(e.to_string()))?;
            if !existing.is_empty() {
                return Err(RuntimeError::SessionAlreadyExists { id });
            }
            Vec::new()
        }
        SessionMode::Resume => {
            let events = self.shared.store.range(&id, ..).await
                .map_err(|e| RuntimeError::StoreError(e.to_string()))?;
            if events.is_empty() {
                return Err(RuntimeError::ResumeFailed {
                    id,
                    reason: "no such session in store".into(),
                });
            }
            events
        }
        SessionMode::Attach => {
            self.shared.store.range(&id, ..).await
                .map_err(|e| RuntimeError::StoreError(e.to_string()))?
        }
    };

    let actor = SessionActor::spawn(self.shared.clone(), id, initial_events).await?;
    Ok(actor)
}
```

(Adjust `SessionActor::spawn` signature to accept `initial_events: Vec<ConversationEvent>`. If it doesn't already, that's part of P4.4.)

- [ ] **Step 5: Run tests to verify pass**

```bash
just test -p cogito-core runtime_session_mode
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/runtime/builder.rs \
        crates/cogito-core/tests/runtime_session_mode.rs
git commit -m "Sprint 3 P4: open_session dispatches by SessionMode" -m "New: errors if log non-empty (SessionAlreadyExists). Resume: errors if log empty (ResumeFailed). Attach: takes whatever exists. initial_events threaded to SessionActor::spawn for actor_main consumption. Per Sprint 3 spec §5.1." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P4.4: Update `actor_main` startup sequence (TDD)

**Files:**

- Modify: `crates/cogito-core/src/runtime/actor.rs` (the `actor_main` / spawn function around line 200+)

- [ ] **Step 1: Write failing test**

```rust
// crates/cogito-core/tests/runtime_resume_dispatch.rs

#[tokio::test]
async fn actor_resumes_into_correct_state_for_in_flight_turn() -> Result<(), Box<dyn std::error::Error>> {
    let store = build_test_store_with_in_flight_turn().await;
    // Pre-populate with events: TurnStarted + ModelCallStarted (no completed)
    // ...

    let runtime = build_test_runtime_with_store(store).await;
    let session = runtime.open_session(session_id, SessionMode::Resume).await?;

    // Send a no-op input that triggers FSM observation
    // Assert the next event written has seq > last_event_seq from the pre-populated log
    let observed = session.observe_next_event_with_timeout(Duration::from_secs(2)).await?;
    assert!(observed.seq > pre_seq);
    Ok(())
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
just test -p cogito-core actor_resumes_into_correct_state_for_in_flight_turn
```

Expected: FAIL (actor_main doesn't call replay yet).

- [ ] **Step 3: Update actor_main startup**

In `crates/cogito-core/src/runtime/actor.rs`, locate the function that's called immediately after task spawn (around `actor.rs:200-230`). Replace the Sprint 2 stub `let decision = replay(&[])` block with:

```rust
// ① Schema check
if let Some(evt) = initial_events
    .iter()
    .find(|e| e.schema_version > cogito_protocol::event::SCHEMA_VERSION)
{
    return ShutdownOutcome::ResumeFailed(format!(
        "unsupported schema_version={}", evt.schema_version
    ));
}

// ② H03 replay
let decision = match crate::harness::resume::replay(&initial_events) {
    Ok(d) => d,
    Err(e) => return ShutdownOutcome::ResumeFailed(e.to_string()),
};

// ③ seq generator initialization
let next_seq = decision.last_event_seq.map_or(0, |s| s + 1);
state.event_seq.store(next_seq, std::sync::atomic::Ordering::SeqCst);

// ④ New session: write SessionStarted
if initial_events.is_empty() {
    let meta = cogito_protocol::session::SessionMeta {
        cogito_version: env!("CARGO_PKG_VERSION").into(),
        ..Default::default()
    };
    if let Err(e) = state.recorder.lock().await.record_session_started(meta).await {
        return ShutdownOutcome::ResumeFailed(format!("record_session_started: {e}"));
    }
}

// ⑤ Apply resume point — dispatches to TurnDriver spawn or PausedOnJob path
if let Err(e) = apply_resume_point(&mut state, decision.point).await {
    return e;
}

// ⑥ Enter mailbox loop (existing actor_loop function)
actor_loop(state).await
```

(Note: `state.event_seq` is an `AtomicU64` field on `ActorState`. If it doesn't exist, add it.)

- [ ] **Step 4: Implement `apply_resume_point`**

Add to `actor.rs`:

```rust
async fn apply_resume_point(
    state: &mut ActorState,
    point: crate::harness::resume::ResumePoint,
) -> Result<(), ShutdownOutcome> {
    use crate::harness::resume::ResumePoint;
    use crate::harness::turn_driver::TurnEntry;

    match point {
        ResumePoint::FreshTurn => {
            // Idle; wait for next mailbox Input
            Ok(())
        }
        ResumePoint::RestartCurrentTurn { turn_id } => {
            spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit).await
        }
        ResumePoint::ResumeFromModelCompleted { turn_id, rebuilt_output } => {
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromModelCompleted { output: rebuilt_output },
            ).await
        }
        ResumePoint::ResumeFromToolDispatching { turn_id, pending, completed } => {
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromToolDispatching { pending, completed },
            ).await
        }
        ResumePoint::ResumePausedJob { turn_id, job_id } => {
            state.in_flight = InFlight::PausedOnJob { job_id: job_id.clone(), turn_id };
            state.job_manager
                .on_complete(job_id, state.job_completion_tx.clone())
                .await
                .map_err(|e| ShutdownOutcome::JobManagerUnavailable(e.to_string()))?;
            Ok(())
        }
        ResumePoint::ResumeAfterJobCompletion {
            turn_id, outcome, call_id,
            completed_before_pause, pending_after_pause, ..
        } => {
            let mut completed = completed_before_pause;
            completed.push((call_id, tool_result_from_job_outcome(outcome)));
            spawn_turn_driver(
                state,
                turn_id,
                TurnEntry::FromToolDispatching {
                    pending: pending_after_pause,
                    completed,
                },
            ).await
        }
    }
}

fn tool_result_from_job_outcome(outcome: cogito_protocol::job::JobOutcome) -> cogito_protocol::tool::ToolResult {
    use cogito_protocol::job::JobOutcome;
    use cogito_protocol::tool::ToolResult;
    match outcome {
        JobOutcome::Success(value) => ToolResult::Output(value),
        JobOutcome::Failed { message } => ToolResult::Error { message },
        JobOutcome::Cancelled => ToolResult::Error { message: "job cancelled".into() },
    }
}

async fn spawn_turn_driver(
    state: &mut ActorState,
    turn_id: TurnId,
    entry: crate::harness::turn_driver::TurnEntry,
) -> Result<(), ShutdownOutcome> {
    // Build TurnCtx + TurnDeps just like Sprint 2 Input handling does
    // ... (mirror the existing Input → spawn flow)
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify pass**

```bash
just test -p cogito-core actor_resumes_into_correct_state_for_in_flight_turn
just test -p cogito-core
```

Expected: PASS; Sprint 2 tests still green.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/runtime/actor.rs \
        crates/cogito-core/tests/runtime_resume_dispatch.rs
git commit -m "Sprint 3 P4: actor_main calls replay() and dispatches by ResumePoint" -m "Replaces Sprint 2 stub (replay(&[])) with the full startup sequence: schema check → replay → seq init → SessionStarted (if new) → apply_resume_point → mailbox loop. apply_resume_point handles all 6 ResumePoint variants including PausedOnJob (no TurnDriver spawn; re-registers on_complete). Per Sprint 3 spec §5.2 + §5.3." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P4.5: Open PR for P4

- [ ] **Step 1: Full CI**

```bash
just ci
```

- [ ] **Step 2: Push + PR**

```bash
git push -u origin impl/sprint-3-p4-actor-recovery
gh pr create --title "Sprint 3 P4: runtime actor recovery wiring" --body "$(cat <<'EOF'
## Summary

- `RuntimeError`: + `SessionAlreadyExists`, `ResumeFailed`
- `ShutdownOutcome`: + `ResumeFailed`, `JobManagerUnavailable`
- `Runtime::open_session` dispatches by `SessionMode` (New / Resume / Attach)
- `SessionActor::actor_main` startup: schema check → replay → seq init → SessionStarted (if new) → apply_resume_point → mailbox
- `apply_resume_point` covers all 6 ResumePoint variants, including PausedOnJob (no TurnDriver spawn) and ResumeAfterJobCompletion (job outcome injected)

Per spec §5.1 + §5.2 + §5.3. Requires P3 merged.

## Test plan

- [ ] `just ci` green
- [ ] `runtime_session_mode` tests green (3 SessionMode paths)
- [ ] `runtime_resume_dispatch` test green (in-flight turn resume)
- [ ] Sprint 2 happy path unaffected

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Phase P4 ends.**

---

## Phase 5 (P5) · Chaos test infrastructure + main test

> Builds `FaultInjectingStore` / `MockJobManager` / `chaos_scenarios` in `cogito-test-fixtures`, verifies (and possibly extends) `cogito-mock-model` for scripted determinism, then implements the 4 oracle helpers and the chaos test main entry (Y path + X path). Branch: `impl/sprint-3-p5-chaos-test`.

### Task P5.1: Create branch

- [ ] **Step 1**

```bash
git checkout main
git pull --ff-only
git checkout -b impl/sprint-3-p5-chaos-test
```

### Task P5.2: Verify or extend `cogito-mock-model` for scripted determinism (阻断性前置)

**Files:**

- Read: `crates/testing/cogito-mock-model/src/lib.rs`
- Possibly modify: same file

- [ ] **Step 1: Inspect current mock model API**

```bash
cat crates/testing/cogito-mock-model/src/lib.rs
```

- [ ] **Step 2: Decide: extend or new module**

The mock must satisfy: given the same `ModelInput.messages`, return **byte-identical** `ModelEvent` streams across repeated calls (within a single test process). Check current API:

- If it accepts a static `Vec<ModelEvent>` set at construction and replays them: ✓ already satisfies.
- If it generates events dynamically (e.g., based on inputs): NOT acceptable for chaos test.

- [ ] **Step 3 (if needed): Add `ScriptedMockModel`**

If current mock doesn't satisfy, add:

```rust
/// Scripted deterministic mock for chaos test. Given an `InputMatcher`,
/// returns a pre-recorded `OutputScript` byte-for-byte identical across
/// repeated calls within a process.
pub struct ScriptedMockModel {
    matchers: Vec<(InputMatcher, OutputScript)>,
}

pub enum InputMatcher {
    /// Match if last user message text equals this string.
    LastUserText(String),
    /// Match if last tool_result content text contains this substring.
    LastToolResultContains(String),
    /// Always match (fallback).
    Any,
}

pub struct OutputScript {
    pub events: Vec<ModelEvent>,
}

#[async_trait]
impl ModelGateway for ScriptedMockModel {
    async fn stream(&self, input: ModelInput, _ctx: ExecCtx) -> BoxStream<'static, Result<ModelEvent, ModelError>> {
        for (matcher, script) in &self.matchers {
            if matcher.matches(&input) {
                let events: Vec<_> = script.events.iter().cloned().map(Ok).collect();
                return Box::pin(futures::stream::iter(events));
            }
        }
        // No match → empty stream with end_turn (defensive)
        Box::pin(futures::stream::iter(vec![
            Ok(ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            })
        ]))
    }
}
```

- [ ] **Step 4: Unit test for scripted determinism**

```rust
#[tokio::test]
async fn scripted_mock_is_deterministic_across_calls() -> Result<(), Box<dyn std::error::Error>> {
    let mock = ScriptedMockModel {
        matchers: vec![(InputMatcher::Any, OutputScript {
            events: vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "hi".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "hi".into() },
                ModelEvent::MessageCompleted { stop_reason: StopReason::EndTurn, usage: Usage::default() },
            ],
        })],
    };
    let input = ModelInput::default();
    let ctx = ExecCtx::for_test();
    let collect = |s: BoxStream<_>| async move {
        s.collect::<Vec<_>>().await
    };
    let r1 = collect(mock.stream(input.clone(), ctx.clone()).await).await;
    let r2 = collect(mock.stream(input.clone(), ctx.clone()).await).await;
    assert_eq!(format!("{r1:?}"), format!("{r2:?}"));
    Ok(())
}
```

- [ ] **Step 5: Commit (if changes made)**

```bash
git add crates/testing/cogito-mock-model/src/lib.rs
git commit -m "Sprint 3 P5: ensure cogito-mock-model supports scripted determinism" -m "ScriptedMockModel provides byte-identical ModelEvent streams across repeated calls within a process — required for chaos test oracles ③ and ④ (tool mapping / final text equivalence). Determinism is verified by unit test. Per Sprint 3 spec §8.6 阻断前置." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.3: Implement `FaultInjectingStore` wrapper

**Files:**

- Create: `crates/testing/cogito-test-fixtures/src/fault_store.rs`

- [ ] **Step 1: Write the wrapper**

```rust
//! Test-only `ConversationStore` wrapper that injects faults after writing
//! the N-th event. Used by chaos test to simulate process crashes (X path:
//! real panic; Y path: oneshot notification then clean shutdown).
//!
//! Production code is zero-modified — fault injection lives entirely in
//! this testing crate via the protocol trait.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::{ConversationStore, StoreError};
use tokio::sync::{oneshot, Mutex};

/// Wraps any `ConversationStore` and triggers a configurable fault after
/// the N-th `append`.
pub struct FaultInjectingStore<S> {
    inner: Arc<S>,
    written_count: AtomicU64,
    trigger: Mutex<FaultTrigger>,
}

/// Configurable fault behavior. Default is `None` (pass-through).
pub enum FaultTrigger {
    /// Pass-through. No fault injected.
    None,
    /// After writing event N (1-indexed), `panic!` with the given message.
    /// The event IS persisted before the panic — simulates "wrote then crashed".
    PanicAt { event_no: u64, message: &'static str },
    /// After writing event N, signal the oneshot. Used by Y path — test
    /// waits on the receiver then calls `SessionHandle::shutdown`.
    NotifyAt {
        event_no: u64,
        signal: Option<oneshot::Sender<()>>,
    },
}

impl<S> FaultInjectingStore<S> {
    pub fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            written_count: AtomicU64::new(0),
            trigger: Mutex::new(FaultTrigger::None),
        }
    }

    pub async fn set_trigger(&self, trigger: FaultTrigger) {
        *self.trigger.lock().await = trigger;
    }
}

#[async_trait]
impl<S: ConversationStore + Send + Sync> ConversationStore for FaultInjectingStore<S> {
    async fn append(&self, evt: &ConversationEvent) -> Result<cogito_protocol::ids::EventId, StoreError> {
        let id = self.inner.append(evt).await?;
        let n = self.written_count.fetch_add(1, Ordering::SeqCst) + 1;

        let mut trigger = self.trigger.lock().await;
        match &mut *trigger {
            FaultTrigger::PanicAt { event_no, message } if *event_no == n => {
                panic!("FaultInjectingStore: {} (event_no={})", message, n);
            }
            FaultTrigger::NotifyAt { event_no, signal } if *event_no == n => {
                if let Some(tx) = signal.take() {
                    let _ = tx.send(());
                }
            }
            _ => {}
        }
        Ok(id)
    }

    async fn range(&self, session: &SessionId, range: std::ops::RangeFull) -> Result<Vec<ConversationEvent>, StoreError> {
        self.inner.range(session, range).await
    }

    async fn tail(&self, session: &SessionId, limit: usize) -> Result<Vec<ConversationEvent>, StoreError> {
        self.inner.tail(session, limit).await
    }
    // Forward other methods identically
}
```

- [ ] **Step 2: Add to crate module**

Modify `crates/testing/cogito-test-fixtures/src/lib.rs`:

```rust
pub mod fault_store;
```

- [ ] **Step 3: Add unit test**

```rust
#[tokio::test]
async fn fault_injecting_store_notifies_at_n() -> Result<(), Box<dyn std::error::Error>> {
    let inner = Arc::new(InMemoryStore::new());
    let store = FaultInjectingStore::new(inner);
    let (tx, rx) = oneshot::channel();
    store.set_trigger(FaultTrigger::NotifyAt {
        event_no: 2,
        signal: Some(tx),
    }).await;

    let session = SessionId::new();
    let e1 = build_test_event(&session, 0);
    let e2 = build_test_event(&session, 1);
    let e3 = build_test_event(&session, 2);

    store.append(&e1).await?;
    assert!(rx.try_recv().is_err());  // not yet at event 2
    store.append(&e2).await?;
    rx.await?;  // event 2 notified
    store.append(&e3).await?;  // event 3 passes through
    Ok(())
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/testing/cogito-test-fixtures/src/lib.rs \
        crates/testing/cogito-test-fixtures/src/fault_store.rs
git commit -m "Sprint 3 P5: FaultInjectingStore wrapper for chaos test injection" -m "ConversationStore wrapper that triggers panic (X path) or oneshot notification (Y path) after writing the N-th event. Zero modification to production code — fault injection lives entirely in cogito-test-fixtures. Per Sprint 3 spec §7.2 + §8.1." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.4: Implement `MockJobManager`

**Files:**

- Create: `crates/testing/cogito-test-fixtures/src/mock_job_manager.rs`

- [ ] **Step 1: Write the implementation**

```rust
//! Test-only `JobManager` for chaos test PausedOnJob scenario. Honors the
//! contract from Sprint 3 spec §8.4:
//! - Contract 1: if job already completed, on_complete triggers sink immediately
//! - Contract 2: if job not yet completed, on_complete stores sink for later

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::job::{
    JobCompletionEvent, JobId, JobManager, JobOutcome, JobState,
};
use tokio::sync::{mpsc, Mutex};

pub struct MockJobManager {
    jobs: Arc<Mutex<HashMap<JobId, JobLifecycle>>>,
}

struct JobLifecycle {
    state: JobState,
    on_complete_sink: Option<mpsc::Sender<JobCompletionEvent>>,
}

impl MockJobManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a job that will need to be completed later.
    pub async fn register(&self, job_id: JobId) {
        self.jobs.lock().await.insert(job_id, JobLifecycle {
            state: JobState::Running,
            on_complete_sink: None,
        });
    }

    /// Test API: mark job as completed and fire on_complete sink (if registered).
    pub async fn complete(&self, job_id: JobId, outcome: JobOutcome) {
        let mut jobs = self.jobs.lock().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.state = JobState::Completed(outcome.clone());
            if let Some(sink) = job.on_complete_sink.take() {
                let _ = sink.send(JobCompletionEvent { job_id, outcome }).await;
            }
        }
    }
}

#[async_trait]
impl JobManager for MockJobManager {
    async fn status(&self, job_id: JobId) -> Result<JobState, cogito_protocol::job::JobManagerError> {
        let jobs = self.jobs.lock().await;
        jobs.get(&job_id)
            .map(|j| j.state.clone())
            .ok_or_else(|| cogito_protocol::job::JobManagerError::UnknownJob(job_id))
    }

    async fn result(&self, job_id: JobId) -> Result<JobOutcome, cogito_protocol::job::JobManagerError> {
        let jobs = self.jobs.lock().await;
        match jobs.get(&job_id).map(|j| &j.state) {
            Some(JobState::Completed(outcome)) => Ok(outcome.clone()),
            Some(_) => Err(cogito_protocol::job::JobManagerError::NotYetComplete(job_id)),
            None => Err(cogito_protocol::job::JobManagerError::UnknownJob(job_id)),
        }
    }

    async fn cancel(&self, _job_id: JobId) -> Result<(), cogito_protocol::job::JobManagerError> {
        Ok(())
    }

    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<(), cogito_protocol::job::JobManagerError> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .get_mut(&job_id)
            .ok_or_else(|| cogito_protocol::job::JobManagerError::UnknownJob(job_id.clone()))?;

        match &job.state {
            JobState::Completed(outcome) => {
                // Contract 1: fire immediately
                let _ = sink.send(JobCompletionEvent {
                    job_id,
                    outcome: outcome.clone(),
                }).await;
            }
            _ => {
                // Contract 2: store for later
                job.on_complete_sink = Some(sink);
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Unit test the two contracts**

```rust
#[tokio::test]
async fn mock_job_manager_fires_on_complete_immediately_if_already_completed() -> Result<(), Box<dyn std::error::Error>> {
    let mgr = MockJobManager::new();
    let job = JobId::default();
    mgr.register(job.clone()).await;
    mgr.complete(job.clone(), JobOutcome::Success(serde_json::json!({}))).await;

    let (tx, mut rx) = mpsc::channel(1);
    mgr.on_complete(job.clone(), tx).await?;

    let evt = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await??;
    assert_eq!(evt.job_id, job);
    Ok(())
}

#[tokio::test]
async fn mock_job_manager_fires_on_complete_after_completion() -> Result<(), Box<dyn std::error::Error>> {
    let mgr = MockJobManager::new();
    let job = JobId::default();
    mgr.register(job.clone()).await;

    let (tx, mut rx) = mpsc::channel(1);
    mgr.on_complete(job.clone(), tx).await?;
    assert!(rx.try_recv().is_err());

    mgr.complete(job.clone(), JobOutcome::Cancelled).await;
    let evt = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await??;
    assert_eq!(evt.job_id, job);
    Ok(())
}
```

- [ ] **Step 3: Add to lib.rs and commit**

```bash
git add crates/testing/cogito-test-fixtures/src/lib.rs \
        crates/testing/cogito-test-fixtures/src/mock_job_manager.rs
git commit -m "Sprint 3 P5: MockJobManager for chaos test PausedOnJob scenario" -m "Honors Sprint 3 spec §8.4 contracts: (1) if job already completed, on_complete triggers sink immediately; (2) if job not yet completed, store sink and fire on complete(). Both contracts unit-tested. Per Sprint 3 spec §8.4." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.5: Implement `chaos_scenarios` fixture

**Files:**

- Create: `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs`

- [ ] **Step 1: Implement four scenarios**

```rust
//! Chaos test scenario catalog. Each scenario is a deterministic recipe
//! for driving cogito through a turn (or paused-job sequence) end-to-end.
//! Scripts the mock model output for each input shape.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};

pub struct ChaosScenario {
    pub name: &'static str,
    pub user_input: Vec<ContentBlock>,
    /// Ordered model event scripts: scenario calls model N times, returns
    /// scripts[i] on the i-th call. Determinism requirement is enforced by
    /// scripts being static `Vec`s.
    pub model_scripts: Vec<Vec<ModelEvent>>,
    /// Whether this scenario expects a paused-on-job sub-flow.
    pub uses_async_job: bool,
}

pub fn all() -> Vec<ChaosScenario> {
    vec![
        single_tool_happy_path(),
        no_tool_short_turn(),
        tool_returns_error(),
        paused_async_job(),
    ]
}

fn single_tool_happy_path() -> ChaosScenario {
    ChaosScenario {
        name: "single_tool_happy_path",
        user_input: vec![ContentBlock::Text { text: "read /etc/hostname".into() }],
        model_scripts: vec![
            // First call: emit text + tool_use
            vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "Reading file...".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "Reading file...".into() },
                ModelEvent::ToolUseStarted { block_index: 1, call_id: "c1".into(), tool_name: "read_file".into() },
                ModelEvent::ToolUseCompleted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage { input_tokens: 50, output_tokens: 20 },
                },
            ],
            // Second call: emit final text + end_turn
            vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "The hostname is foo.".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "The hostname is foo.".into() },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage { input_tokens: 75, output_tokens: 10 },
                },
            ],
        ],
        uses_async_job: false,
    }
}

fn no_tool_short_turn() -> ChaosScenario {
    ChaosScenario {
        name: "no_tool_short_turn",
        user_input: vec![ContentBlock::Text { text: "say hi".into() }],
        model_scripts: vec![
            vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "Hi.".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "Hi.".into() },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage { input_tokens: 5, output_tokens: 2 },
                },
            ],
        ],
        uses_async_job: false,
    }
}

fn tool_returns_error() -> ChaosScenario {
    // Same first model call as single_tool_happy_path but the test harness
    // wires the tool to return ToolResult::Error
    ChaosScenario {
        name: "tool_returns_error",
        user_input: vec![ContentBlock::Text { text: "read /nonexistent".into() }],
        model_scripts: vec![
            vec![
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/nonexistent"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
            ],
            // Second call: handle the error
            vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "File not found.".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "File not found.".into() },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ],
        ],
        uses_async_job: false,
    }
}

fn paused_async_job() -> ChaosScenario {
    ChaosScenario {
        name: "paused_async_job",
        user_input: vec![ContentBlock::Text { text: "run long task".into() }],
        model_scripts: vec![
            vec![
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "c_async".into(),
                    tool_name: "long_tool".into(),
                    args: serde_json::json!({}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
            ],
            vec![
                ModelEvent::TextBlockCompleted { block_index: 0, text: "Done.".into() },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ],
        ],
        uses_async_job: true,
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/testing/cogito-test-fixtures/src/lib.rs \
        crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs
git commit -m "Sprint 3 P5: chaos_scenarios fixture (4 scenarios)" -m "Defines single_tool_happy_path, no_tool_short_turn, tool_returns_error, paused_async_job. Each scenario scripts the mock model output for every call in the turn. Static data ensures determinism for chaos test oracles ③④. Per Sprint 3 spec §8.3." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.6: Implement chaos test main + 4 oracle helpers

**Files:**

- Create: `crates/cogito-core/tests/resume_chaos.rs`

- [ ] **Step 1: Implement the test file**

```rust
//! Sprint 3 chaos test — verifies cogito's resume guarantees.
//!
//! Drives 4 scenarios through every event-boundary crash point (Y path)
//! and 8 curated panic points (X path). Each crash is followed by a fresh
//! Runtime resume; the resumed event log is compared against the golden
//! (uncrashed) log via 4 oracles per spec §8.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{Runtime, RuntimeBuilder, SessionMode};
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::ids::SessionId;
use cogito_test_fixtures::{
    chaos_scenarios::{self, ChaosScenario},
    fault_store::{FaultInjectingStore, FaultTrigger},
    mock_job_manager::MockJobManager,
};
use cogito_mock_model::{InputMatcher, OutputScript, ScriptedMockModel};
use tokio::sync::oneshot;

#[derive(Debug)]
struct GoldenRun {
    events: Vec<ConversationEvent>,
    terminal: EventPayload,
}

async fn run_to_completion_without_faults(scenario: &ChaosScenario) -> GoldenRun {
    // ... build a Runtime with an unwrapped InMemoryStore + ScriptedMockModel
    // built from scenario.model_scripts. Open new session, send user input,
    // wait for terminal event. Return the full log + terminal.
    unimplemented!("see step 2")
}

async fn run_with_y_fault(
    scenario: &ChaosScenario,
    crash_after_n: u64,
) -> Vec<ConversationEvent> {
    // ... build a Runtime with FaultInjectingStore wrapping InMemoryStore.
    // Set trigger = NotifyAt { event_no: crash_after_n, signal: tx }.
    // Open new session, send user input.
    // Wait on rx for the notification, then call session.shutdown(short_timeout).
    // Build a SECOND Runtime with the same InMemoryStore (shared Arc).
    // Open session in Resume mode. Wait for terminal. Return full log.
    unimplemented!("see step 2")
}

async fn run_with_x_fault(
    scenario: &ChaosScenario,
    panic_after_n: u64,
) -> Vec<ConversationEvent> {
    // ... similar to Y but trigger = PanicAt. Set a panic hook to
    // suppress the noise. The Runtime catch_unwind catches the panic;
    // SessionHandle observes a panic-shutdown outcome. Then resume.
    unimplemented!("see step 2")
}

// === Oracles ===

#[derive(Debug, PartialEq)]
struct Canonical {
    schema_version: u32,
    session_id: SessionId,
    turn_id: Option<cogito_protocol::ids::TurnId>,
    seq: u64,
    payload: EventPayload,
}

fn canonical(e: &ConversationEvent) -> Canonical {
    Canonical {
        schema_version: e.schema_version,
        session_id: e.session_id.clone(),
        turn_id: e.turn_id.clone(),
        seq: e.seq,
        payload: e.payload.clone(),
    }
}

fn assert_prefix_immutable(golden: &[ConversationEvent], resumed: &[ConversationEvent], n: u64) {
    let prefix_len = n as usize;
    assert!(resumed.len() >= prefix_len, "resumed log shorter than crash point");
    let golden_prefix: Vec<_> = golden[..prefix_len].iter().map(canonical).collect();
    let resumed_prefix: Vec<_> = resumed[..prefix_len].iter().map(canonical).collect();
    assert_eq!(golden_prefix, resumed_prefix, "pre-crash prefix diverged at n={n}");
}

fn terminal_payload(events: &[ConversationEvent]) -> &EventPayload {
    events.iter().rev().find_map(|e| match &e.payload {
        EventPayload::TurnCompleted { .. }
        | EventPayload::TurnFailed { .. }
        | EventPayload::TurnPaused { .. } => Some(&e.payload),
        _ => None,
    }).expect("no terminal event in log")
}

fn assert_terminal_equivalent(g: &EventPayload, r: &EventPayload) {
    use EventPayload::*;
    match (g, r) {
        (TurnCompleted { .. }, TurnCompleted { .. }) => {}
        (TurnFailed { reason: r1 }, TurnFailed { reason: r2 }) => {
            assert_eq!(std::mem::discriminant(r1), std::mem::discriminant(r2));
        }
        (TurnPaused { job_id: j1 }, TurnPaused { job_id: j2 }) => {
            assert_eq!(j1, j2);
        }
        _ => panic!("terminal kind differs: golden={g:?} resumed={r:?}"),
    }
}

fn collect_tool_mapping(events: &[ConversationEvent])
    -> std::collections::BTreeMap<String, (String, serde_json::Value, cogito_protocol::tool::ToolResult)>
{
    use std::collections::{BTreeMap, HashMap};
    let mut uses: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut results: HashMap<String, cogito_protocol::tool::ToolResult> = HashMap::new();
    for e in events {
        match &e.payload {
            EventPayload::ToolUseRecorded { call_id, tool_name, args } => {
                uses.insert(call_id.clone(), (tool_name.clone(), args.clone()));
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                results.insert(call_id.clone(), result.clone());
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for (id, (name, args)) in uses {
        if let Some(r) = results.get(&id) {
            out.insert(id, (name, args, r.clone()));
        }
    }
    out
}

fn assert_tool_mapping_equivalent(g: &[ConversationEvent], r: &[ConversationEvent]) {
    assert_eq!(collect_tool_mapping(g), collect_tool_mapping(r), "tool mappings differ");
}

fn assert_final_text_equivalent(g: &[ConversationEvent], r: &[ConversationEvent]) {
    let collect_text = |events: &[ConversationEvent]| -> String {
        events.iter()
            .filter_map(|e| match &e.payload {
                EventPayload::AssistantMessageAppended { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>().concat()
    };
    assert_eq!(collect_text(g), collect_text(r), "final text differs");
}

// === Tests ===

#[tokio::test]
async fn chaos_y_path_every_event_boundary() {
    for scenario in chaos_scenarios::all() {
        let golden = run_to_completion_without_faults(&scenario).await;
        let total = golden.events.len() as u64;
        for crash_after_n in 1..total {
            let resumed = run_with_y_fault(&scenario, crash_after_n).await;
            assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
            assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
            assert_tool_mapping_equivalent(&golden.events, &resumed);
            assert_final_text_equivalent(&golden.events, &resumed);
        }
    }
}

#[tokio::test]
async fn chaos_x_path_curated_panic_points() {
    // Suppress panic noise from the FaultInjectingStore panics
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let panic_after_names = [
        "after_turn_started", "after_prompt_composed", "after_model_call_started",
        "after_model_call_completed", "after_assistant_message_appended",
        "after_tool_use_recorded", "after_tool_result_recorded",
        "after_turn_paused",
    ];

    for scenario in chaos_scenarios::all() {
        let golden = run_to_completion_without_faults(&scenario).await;
        for name in &panic_after_names {
            // Find the event index that matches this label
            if let Some(crash_n) = find_event_index_for_label(&golden.events, name) {
                let resumed = run_with_x_fault(&scenario, crash_n).await;
                assert_prefix_immutable(&golden.events, &resumed, crash_n);
                assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
                assert_tool_mapping_equivalent(&golden.events, &resumed);
                assert_final_text_equivalent(&golden.events, &resumed);
            }
        }
    }

    std::panic::set_hook(prev_hook);
}

fn find_event_index_for_label(events: &[ConversationEvent], label: &str) -> Option<u64> {
    let target = match label {
        "after_turn_started" => |p: &EventPayload| matches!(p, EventPayload::TurnStarted { .. }),
        "after_prompt_composed" => |p: &EventPayload| matches!(p, EventPayload::PromptComposed { .. }),
        "after_model_call_started" => |p: &EventPayload| matches!(p, EventPayload::ModelCallStarted { .. }),
        "after_model_call_completed" => |p: &EventPayload| matches!(p, EventPayload::ModelCallCompleted { .. }),
        "after_assistant_message_appended" => |p: &EventPayload| matches!(p, EventPayload::AssistantMessageAppended { .. }),
        "after_tool_use_recorded" => |p: &EventPayload| matches!(p, EventPayload::ToolUseRecorded { .. }),
        "after_tool_result_recorded" => |p: &EventPayload| matches!(p, EventPayload::ToolResultRecorded { .. }),
        "after_turn_paused" => |p: &EventPayload| matches!(p, EventPayload::TurnPaused { .. }),
        _ => return None,
    };
    events.iter().position(|e| target(&e.payload)).map(|i| (i + 1) as u64)
}
```

- [ ] **Step 2: Implement the test harness helpers (`run_to_completion_without_faults`, `run_with_y_fault`, `run_with_x_fault`)**

These are non-trivial. Pattern for `run_to_completion_without_faults`:

```rust
async fn run_to_completion_without_faults(scenario: &ChaosScenario) -> GoldenRun {
    let store = Arc::new(cogito_store_jsonl::JsonlStore::tmp()); // or InMemoryStore
    let model = build_scripted_mock_for_scenario(scenario);
    let job_manager = Arc::new(MockJobManager::new());
    let runtime = RuntimeBuilder::new()
        .store(store.clone())
        .model_gateway(Arc::new(model))
        .job_manager(job_manager.clone())
        .build()
        .expect("build runtime");

    let session_id = SessionId::new();
    let session = runtime.open_session(session_id.clone(), SessionMode::New).await.unwrap();
    session.send_user(scenario.user_input.clone()).await.unwrap();

    // For paused_async_job scenarios, wait for TurnPaused then call mock_job_manager.complete
    if scenario.uses_async_job {
        // observe via events_out broadcast or poll store; on TurnPaused, complete the job
        observe_until_paused_then_complete(&session, &job_manager).await;
    }

    let terminal = wait_for_terminal_event(&session, Duration::from_secs(5)).await;
    let events = store.range(&session_id, ..).await.unwrap();
    let terminal_payload = events.iter().rev()
        .find_map(|e| match &e.payload {
            EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. } => Some(e.payload.clone()),
            _ => None,
        })
        .expect("no terminal event");
    GoldenRun { events, terminal: terminal_payload }
}
```

`run_with_y_fault` and `run_with_x_fault` follow the same pattern but wrap `store` in `FaultInjectingStore` and dispose / rebuild Runtime on crash:

```rust
async fn run_with_y_fault(scenario: &ChaosScenario, crash_after_n: u64) -> Vec<ConversationEvent> {
    let inner_store = Arc::new(InMemoryStore::new());
    let store = Arc::new(FaultInjectingStore::new(inner_store.clone()));
    let (tx, rx) = oneshot::channel();
    store.set_trigger(FaultTrigger::NotifyAt {
        event_no: crash_after_n,
        signal: Some(tx),
    }).await;

    // First Runtime: run until notify, then shutdown
    let runtime1 = build_runtime_with_store(store.clone(), scenario).await;
    let session_id = SessionId::new();
    let session = runtime1.open_session(session_id.clone(), SessionMode::New).await.unwrap();
    session.send_user(scenario.user_input.clone()).await.unwrap();
    rx.await.expect("notification");
    session.shutdown(Duration::from_millis(500)).await.unwrap();
    drop(runtime1);  // Simulate process exit

    // Second Runtime: same inner_store, Resume mode
    let store2 = Arc::new(FaultInjectingStore::new(inner_store.clone())); // fresh wrapper
    store2.set_trigger(FaultTrigger::None).await;
    let runtime2 = build_runtime_with_store(store2.clone(), scenario).await;
    let session2 = runtime2.open_session(session_id.clone(), SessionMode::Resume).await.unwrap();

    // For async job scenarios, drive completion
    if scenario.uses_async_job {
        // The on_complete callback fires from the resumed actor;
        // we wait briefly then call mock_job_manager.complete
        // (which was preserved if MockJobManager is also shared across Runtimes)
    }

    wait_for_terminal_event(&session2, Duration::from_secs(5)).await;
    store2.inner().range(&session_id, ..).await.unwrap()
}
```

- [ ] **Step 3: Run the chaos test**

```bash
just test -p cogito-core --test resume_chaos
```

Expected: PASS for all scenarios × all crash points.

- [ ] **Step 4: Time the test**

```bash
time just test -p cogito-core --test resume_chaos
```

Expected: < 10 seconds total per Sprint 3 §8.5 budget.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/tests/resume_chaos.rs
git commit -m "Sprint 3 P5: chaos test main — Y + X paths × 4 scenarios × 4 oracles" -m "Y path covers every event boundary via NotifyAt + clean shutdown + Resume. X path covers 8 curated panic points via PanicAt + Runtime drop + Resume. Both paths apply the 4 oracles (prefix immutable / terminal equivalent / tool mapping / final text). Runs under 10s; included in default just ci. Per Sprint 3 spec §8.2 + §8.5." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.7: ROADMAP Sprint 3 checklist tick

**Files:**

- Modify: `ROADMAP.md`

- [ ] **Step 1: Tick all Sprint 3 checkboxes**

In `ROADMAP.md` Sprint 3 section, change every `- [ ]` to `- [x]` for the 9 items P1.10 added.

- [ ] **Step 2: Commit**

```bash
git add ROADMAP.md
git commit -m "Sprint 3 P5: tick ROADMAP Sprint 3 checklist (resume coordinator complete)" -m "All 9 Sprint 3 deliverables done. Chaos test green across 4 scenarios × every event boundary (Y) + 8 curated panic points (X). Resume-from-paused-job validated via MockJobManager. Per Sprint 3 spec." -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

### Task P5.8: Open PR for P5

- [ ] **Step 1: Full CI**

```bash
just ci
```

Expected: all green; chaos test included; total CI time stays within budget.

- [ ] **Step 2: Push + PR**

```bash
git push -u origin impl/sprint-3-p5-chaos-test
gh pr create --title "Sprint 3 P5: chaos test infrastructure + main test" --body "$(cat <<'EOF'
## Summary

- `cogito-mock-model` `ScriptedMockModel` (verified deterministic across calls)
- `cogito-test-fixtures::fault_store::FaultInjectingStore` (ConversationStore wrapper)
- `cogito-test-fixtures::mock_job_manager::MockJobManager` (honors on_complete contracts)
- `cogito-test-fixtures::chaos_scenarios` — 4 scenarios
- `crates/cogito-core/tests/resume_chaos.rs` — Y path + X path × 4 scenarios × 4 oracles
- ROADMAP Sprint 3 checklist ticked

Per spec §7–§8. Requires P4 merged.

## Test plan

- [ ] `just ci` green; chaos test under 10s
- [ ] `chaos_y_path_every_event_boundary` PASS for all 4 scenarios
- [ ] `chaos_x_path_curated_panic_points` PASS for all 4 scenarios × 8 panic points
- [ ] Manual: `cogito chat` end-to-end still works

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Phase P5 ends. Sprint 3 complete.**

---

## Self-review

### Spec coverage

Going through spec §1.1 (in-scope items) row by row:

1. ✅ `EventPayload::ModelCallCompleted` — P2.2 + P2.3 + P2.4 (variant + schema + fixture)
2. ✅ H03 `replay()` — P3.2 + P3.3 (types + algorithm)
3. ✅ `runtime/actor.rs` replay wiring — P4.4
4. ✅ `Runtime::open_session(SessionMode::Resume)` — P4.3
5. ✅ EventId / seq generator initialization + `recorded_event_id` stub fix — P2.5
6. ✅ `tests/resume_chaos.rs` — P5.6
7. ✅ `FaultInjectingStore` + `MockJobManager` — P5.3 + P5.4
8. ✅ `consecutive_tool_errors` reset to 0 — implicit in P3.4 `enter_turn` (TurnCtx constructed fresh)
9. ✅ Doc propagation (ARCHITECTURE + ADRs + components + jsonl-v1 + ROADMAP) — Phase P1 all 9 commits

### Placeholder scan

Searched: no `TBD`, `TODO`, `FIXME`, `XXX`. The two `unimplemented!("see step 2")` calls in P5.6 are intentional skeleton markers — the same task body explicitly provides the implementation pattern for each helper. Acceptable.

### Type consistency

- `ResumeDecision { point: ResumePoint, last_event_seq: Option<u64> }` is consistent across P3.2 (definition) / P3.3 (test usage) / P3.4 (consumer) / P4.4 (consumer).
- `ResumePoint` 6 variants are consistent across P3.2 / P3.3 / P3.4 / P4.4.
- `TurnEntry` 3 variants (`FreshLikeInit` / `FromModelCompleted` / `FromToolDispatching`) consistent across P3.4 / P4.4.
- `FaultTrigger { None, PanicAt, NotifyAt }` consistent across P5.3 / P5.6.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-20-sprint-3-resume-coordinator.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Good for breadth: 5 phases × 5-8 tasks each.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`. Batch execution with checkpoints for review.

**Which approach?**

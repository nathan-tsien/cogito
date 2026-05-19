# H11 · Context Manage

> **Status**: 🧭 Architectural slot reserved · 🚧 Mechanism design pending ADR-0008 (Context Management initiative) · 🚧 Not implemented (target: post-Sprint 2)

## Why this component exists

Three observations forced H11 to exist:

1. **H04 Prompt Composer is pure** (`invariant #2: no I/O`, see H04 doc). It cannot perform summarization-based compaction because summarization requires a model call.
2. **H05 Tool Surface Builder is pure and strategy-static**. It cannot perform context-aware tool injection (e.g., "drop the `write_file` tool because we're in a read-only review subtask").
3. **H10 Strategy Selector is even more passive** — it produces a `HarnessStrategy` value and never speaks again.

Yet every production agent must, at some point per turn, **decide what context the model should see**: how much history to include, whether to summarize, what system context to inject, which tool subset is appropriate now. Without a dedicated home for these decisions, they leak into H04 (breaking its pure-function invariant) or into the consumer's calling code (breaking encapsulation).

H11 is that home. It sits **between H10 (strategy lookup) and H04 (passive composition)** in the Init → ContextManaged → PromptBuilt sequence, and is allowed to do I/O (model calls for summarization), which H04/H05/H10 are not.

## Role in Harness

Decide, for the turn that is about to start, the **context shape** that H04 will then mechanically render into a `ModelInput`. Context shape covers:

- **History compaction**: do we need to summarize old events? What's the summary? Which seq-range does it supersede?
- **System prompt injection**: any per-turn additions on top of `strategy.system_prompt` (date/locale, sub-task context, tenant-specific preamble)?
- **Tool surface override**: any per-turn restriction beyond what H05 would produce from strategy alone (e.g., "this is a planning subagent — drop all file-write tools regardless of strategy")?
- **Length-budget enforcement**: if compaction is needed, how aggressively?

H11 emits a `ContextDecision` value plus zero-or-more **persisted events** describing what it did (compaction events, override events). H04 then reads the event log (including H11's just-written events) and composes the prompt deterministically.

## State machine placement

```
Init
  │  H10 strategy (pure)
  │  H03 resume decision (pure)
  ▼
ContextManaged ◄── new state introduced by ADR-0006 amendment (this PR)
  │  H11 manage(context)
  │     ├─ may call ModelGateway for summarization
  │     └─ writes 0+ events via H02 (ContextCompacted, ContextDecision, ...)
  ▼
PromptBuilt
  │  H04 compose (pure; reads history including H11's just-written events)
  │  H05 surface (pure; respects H11's tool override if any)
  │  H09 pre_prompt hook (pure; may Allow / Modify / Reject)
  ▼
ModelCalling → ModelCompleted → ToolDispatching → {Completed | Paused | Failed}
```

`ContextManaged` is a real FSM state (not an inline call) because:

- **It does I/O** (summarization model call) — possibly long-running.
- **It must be resumable**: H03 needs to know if a crash happened mid-summarization, so H11 transitions must be visible to the event log.
- **It writes its own events** to the log — `ContextCompacted`, `ContextDecisionRecorded`, etc. (exact variant set TBD by ADR-0008).

## Interface (PROVISIONAL — final shape in ADR-0008)

```rust
// In cogito-protocol (final placement TBD)
#[async_trait]
pub trait ContextManager: Send + Sync {
    /// Make one context-management decision for an upcoming turn.
    /// May persist events via the recorder; returns the decision H04/H05
    /// must honor when composing the prompt.
    async fn manage(
        &self,
        input: ContextManageInput<'_>,
    ) -> Result<ContextDecision, ContextError>;
}

pub struct ContextManageInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub strategy: &'a HarnessStrategy,
    pub store: &'a dyn ConversationStore,    // for reading history
    pub model_gateway: &'a dyn ModelGateway, // for summarization calls
    pub recorder: &'a mut StepRecorder,      // for persisting decisions
}

pub struct ContextDecision {
    /// History range that should be projected through compaction summaries
    /// instead of literal events. H04 honors this when projecting.
    pub compaction_replacements: Vec<CompactionRange>,
    /// Optional per-turn system prompt suffix appended after strategy.system.
    pub system_prompt_suffix: Option<String>,
    /// Optional per-turn tool filter that intersects with H05's strategy filter.
    pub tool_filter_override: Option<ToolFilter>,
}
```

**These are placeholders, not contracts.** The Context Management initiative will refine them. What is locked by this PR:

- **H11 sits between H10 and H04 in the lifecycle.**
- **H11 is allowed to do I/O** (model calls); H04/H05/H10 are not.
- **H11 writes events through H02 like any other component.**
- **`ContextManaged` is a real FSM state** in the H01 Turn Driver state machine.

## Dependencies

**Calls (out)** (provisional):

- `ConversationStore::replay` — read history to decide if compaction is needed.
- `ModelGateway::stream` — issue a summarization call when compacting (this is the only Brain component besides H01's main-loop call that uses ModelGateway).
- `StepRecorder` — persist `ContextCompacted` / decision events.

**Called by**: H01 Turn Driver, at the `Init → ContextManaged` transition.

**Does NOT call**: H04, H05, H07, H08, H09 (per AGENTS.md §1 "H01 is the only coordinator"). H11 produces a value; H01 hands that value to H04/H05.

## Critical invariants (locked by this PR)

1. **Brain-side**. Lives in `cogito-core::harness`. Imports `cogito-protocol` only.
2. **Recorder pass-through, not Hand**. H11 writes events through H02 (which writes to `ConversationStore`). H11 does NOT acquire its own `Arc<dyn ConversationStore>` outside of H02 — Storage stays single-writer per session.
3. **ModelGateway access is read-allowed, write-restricted**. H11 may call `ModelGateway::stream` for summarization, but the result is NEVER a real assistant message persisted as `AssistantMessageAppended`. Summarization output is persisted as a context-management variant (TBD ADR-0008).
4. **Decisions are turn-scoped**. A `ContextDecision` applies to the current turn only. The next turn's H11 invocation re-decides from scratch (reading the persisted history, including prior compactions).
5. **No cross-turn state in H11 struct fields**. State lives in the event log (AGENTS.md §3). H11 may have read-only configuration injected at construction.
6. **Recoverable from log**. If a crash occurs mid-`ContextManaged` (e.g., during a summarization call), H03 reads the persisted partial events and decides whether to redo or skip the H11 step. This requires H11 events to be self-describing — exact event shape is the central deliverable of ADR-0008.

## Open design questions (resolved by Context Management initiative)

- **Trigger policy**: token-count threshold (Codex pattern) vs strategy-flag-driven vs hook-gated? Probably all three are valid configurations — the question is the default.
- **Summarization model**: same as the turn's model, or a separate "summarization model" injectable via strategy?
- **Replacement semantics**: do compactions cascade (summary of summaries)? Is there a max compaction depth?
- **Tool injection mechanism**: is there a separate "context-aware tool catalog" Hand, or does H11 always work from H05's filter as a starting point?
- **System prompt evolution**: append-only vs replace-all per turn? Persisted as part of `ContextDecisionRecorded` or as a separate `SystemPromptUpdated` event?
- **EventPayload variants needed**: at minimum `ContextCompacted`. Possibly also `ContextDecisionRecorded`, `SystemPromptInjected`, `ToolFilterOverridden`. ADR-0008 will enumerate and lock.
- **Trait placement**: `ContextManager` in `cogito-protocol` (consumer-overridable) vs `cogito-core` internal (cogito-controlled)?
- **Composability with H09 hooks**: do `pre_context` and `post_context` hook lifecycle points exist? If yes, where in the H01 FSM?

## v0.1–v0.2 plan (this PR's commitment)

- **This PR**: lock the architectural slot. `ContextManaged` state added to ADR-0006's FSM by amendment. H01 doc updated. AGENTS.md component count updated 10 → 11. H11 doc (this file) exists as the design anchor.
- **Sprint 2** (Minimal Loop): H01 implements `ContextManaged` as a pass-through (no work — immediately transitions to `PromptBuilt`). This keeps the FSM honest while the real H11 is being designed.
- **Context Management initiative** (post-Sprint 2, before/parallel-to Sprint 3): research + spec + ADR-0008 + implementation sprint(s). At that point H11 transitions to real work.
- **Resume Coordinator (Sprint 3)**: must understand `ContextManaged` state from day one — the resume decision table includes "crashed in ContextManaged" as a case (pass-through in Sprint 2/3, real semantics when H11 is implemented).

## Testing strategy (post-implementation)

- **Unit**: each decision branch (no-op, compact-needed, system-prompt-override, tool-override) tested with mocked `ModelGateway` and in-memory store.
- **Integration**: full turn through Init → ContextManaged → PromptBuilt against a scripted compaction scenario.
- **Chaos**: inject crash between every event H11 writes; H03 must recover correctly (either redo the H11 step or use the partial result deterministically).
- **Property**: H04 projection through a `ContextDecision` is deterministic; same `(history, decision)` → same `ModelInput`.

## References

- AGENTS.md §"Inviolable design principles" #1 (H01 is the only coordinator), #6 (Brain sees Hands only through Protocol)
- ADR-0003 (state-machine Turn Driver — extended by this amendment)
- ADR-0004 (Brain / Hands / Session boundaries)
- ADR-0006 (Runtime + H01 execution model — amended by this PR to add `ContextManaged` state)
- ADR-0008 (Context Management — pending; this doc is a placeholder for it)
- H01 Turn Driver doc §"Init → ContextManaged → PromptBuilt sequence" (canonical walkthrough)
- H04 Prompt Composer doc §"Pure function invariant" (the constraint that forced H11 to exist)
- Codex Rust prior art: `codex-rs/core/src/codex.rs:2913` (pre-turn `if total_usage_tokens >= auto_compact_limit`); `codex-rs/core/src/compact.rs` (compaction implementation)
- Claude Code: `/compact` slash command + auto-compaction at context-window threshold

# H01 · Turn Driver

> **Status**: 🚧 Not implemented · Sprint 2–3

## Role in Harness

Drive one iteration of the agent loop as an **explicit finite state
machine**. The only coordinator inside Brain — H02 through H10 do not call
each other; H01 calls them.

## State machine

```
Init → ContextManaged → PromptBuilt → ModelCalling → ModelCompleted
                                                       │
                                                       ▼
                               ┌─── ToolDispatching ───┤
                               │                       │
                               ▼                       ▼
                         {Completed | Paused}        Failed
```

| State | Entered when | H01 calls |
|---|---|---|
| `Init` | Turn starts (new or resumed) | H10 (strategy), H03 (resume decision) |
| `ContextManaged` | Context decisions (compaction, system-prompt overrides, tool overrides) finalized for this turn | H11 (manage); H02 records H11's decisions |
| `PromptBuilt` | Prompt + tool surface assembled | H04 (compose), H05 (surface), H09 (`pre_prompt`) |
| `ModelCalling` | ModelGateway streaming started | H06 (demux) per chunk |
| `ModelCompleted` | Stream ended with `stop_reason` | H07 (resolve tool calls), H09 (`post_model`) |
| `ToolDispatching` | One or more tool calls present | H08 (invoke), H09 (`pre_dispatch`) |
| `Completed` | Model returned `end_turn` without tools | H09 (`post_turn`) |
| `Paused` | A tool returned `InvokeOutcome::Async(JobId)` | H09 (`post_turn`); turn re-entered when job finishes |
| `Failed` | Unrecoverable error or hook `Reject` | H09 (`on_error`) |

> **`ContextManaged` was added 2026-05-19 by PR #6** as an ADR-0006 amendment.
> Rationale and the full H10/H11/H04/H05/H09 collaboration walkthrough are
> below in §"Init → ContextManaged → PromptBuilt sequence". v0.1 Sprint 2 ships
> `ContextManaged` as a pass-through (no work); the real H11 work lands with
> the Context Management initiative (ADR-0008, pending).

## Init → ContextManaged → PromptBuilt sequence (canonical)

This is the **five-component collaboration** that produces a ready-to-stream `ModelInput`. Reviewers and implementers MUST consult this when touching H04, H05, H09, H10, or H11.

```
H01 in state = Init
  │
  │  step 1 ──► H10 Strategy Selector::select(model_id, task, registry)
  │             ─ pure value lookup, no I/O
  │             ─ returns HarnessStrategy { system_prompt, allowed_tools,
  │               tool_order, length_budget, model_params, hooks, ... }
  │             ─ result cached for the entire turn (used by H11, H04, H05, H09)
  │             ─ H02 records: TurnStarted { strategy_id, ... }
  │
  │  step 2 ──► H03 Resume Coordinator (if resuming)
  │             ─ decides starting state for this turn
  │             ─ may say "skip to ToolDispatching with prior tool call"
  │
H01 transitions: Init → ContextManaged   (H02 records the transition)
  │
  │  step 3 ──► H11 Context Manage::manage(ContextManageInput { strategy, ... })
  │             ─ ALLOWED to do I/O: may call ModelGateway for summarization
  │             ─ may write multiple events via H02:
  │                 · ContextCompacted (when compaction occurs)
  │                 · ContextDecisionRecorded (always — captures the decision)
  │                 · SystemPromptInjected (when overriding strategy.system)
  │                 · ToolFilterOverridden (when narrowing H05's filter)
  │             ─ returns ContextDecision { compaction_replacements,
  │                                         system_prompt_suffix,
  │                                         tool_filter_override }
  │             ─ ContextDecision is held by H01 for the rest of the turn,
  │               but H04/H05 read the equivalent information from the
  │               event log (not from the in-memory value) — this matches
  │               AGENTS.md §3 "State lives in Conversation Service".
  │             ─ v0.1 Sprint 2: pass-through (no H11 implementation; H01
  │               immediately transitions to PromptBuilt with an empty
  │               ContextDecision and no events written).
  │
H01 transitions: ContextManaged → PromptBuilt   (H02 records the transition)
  │
  │  step 4 ──► H04 Prompt Composer::compose(history, strategy, surface)
  │             ─ PURE: deterministic, no I/O (invariant #2)
  │             ─ history projection rule: walk events in seq order, when
  │               a ContextCompacted event is encountered, skip events in
  │               its replaced_seq_range and emit its replacement instead
  │             ─ system = strategy.system_prompt + any
  │               SystemPromptInjected from this turn's H11 events
  │             ─ returns ModelInput { system, messages, tools, params }
  │
  │  step 5 ──► H05 Tool Surface Builder::surface(strategy, provider)
  │             ─ PURE: deterministic, no I/O (invariant #1/2)
  │             ─ filters provider.list() by strategy.allowed_tools
  │             ─ if H11 wrote a ToolFilterOverridden event this turn,
  │               H05 intersects its strategy filter with the override
  │               (override only narrows; H05 never expands beyond strategy)
  │             ─ returns Vec<ToolDescriptor> sorted by strategy.tool_order
  │               or by name (stable for prompt-cache hit rate)
  │             ─ output is plugged into the ModelInput.tools field
  │
  │  step 6 ──► H09 Hook Pipeline::pre_prompt(ModelInput, hooks)
  │             ─ PURE: hooks may not do I/O (AGENTS.md §6 + H09 doc)
  │             ─ each hook returns Allow / Modify(new_input) / Reject(reason)
  │             ─ Modify chains: the next hook sees the modified input
  │             ─ Reject: H01 transitions Init → Failed with HookRejected reason
  │             ─ H02 records: HookFired { name, decision } for each hook
  │
H01 transitions: PromptBuilt → ModelCalling   (H02 records; ModelGateway::stream begins)
```

**Responsibility matrix**:

| Component | Sees strategy? | Sees history? | May do I/O? | Output |
|---|:-:|:-:|:-:|---|
| H10 Strategy Selector | (produces it) | ✗ | ✗ | `HarnessStrategy` value |
| H11 Context Manage | ✓ | ✓ | ✓ (model call) | `ContextDecision` + persisted events |
| H04 Prompt Composer | ✓ | ✓ (via projection) | ✗ | `ModelInput` value |
| H05 Tool Surface Builder | ✓ | ✗ | ✗ (provider.list() is read-only) | `Vec<ToolDescriptor>` |
| H09 Hook Pipeline (`pre_prompt`) | ✓ | ✓ (via input) | ✗ | `HookDecision` per hook |

**Why H11 sits where it does**:

- **Before H04**: H04 reads history *through* H11's compaction decisions. Putting H11 after H04 would mean composing a wasteful full prompt first, then throwing parts away.
- **After H10**: H11 needs `HarnessStrategy` to know the length budget, summarization model preference, and tool-filter starting point.
- **Before H09**: hooks observe the *post-context-management* input. A hook that says "reject if prompt mentions 'reset password'" should run against the compacted, ready-to-ship prompt, not against the raw history.
- **As its own FSM state, not inline in `Init`**: because H11 does I/O (summarization can take seconds). Hiding I/O inside a transition violates ADR-0003's "each transition writes an event" intent — H03 Resume Coordinator must be able to distinguish "crashed in H10 lookup" from "crashed mid-summarization".

## Interface (design level)

- `Turn::run(req: TurnRequest) -> TurnOutcome`
- `TurnRequest { session_id, input: NewMessage(text) | ResumeAfter(job_id) }`
- `TurnOutcome { Completed | Paused { reason } | Failed { error_kind, message } }`
- Implementation is async; one in-flight turn per session at a time (enforced by Runtime, not by H01).

## Dependencies

**Calls (out)**:
- H03 Resume Coordinator — once on entry, decides starting state
- H10 Strategy Selector — once on entry, produces `HarnessStrategy` value
- H11 Context Manage — at `Init → ContextManaged`
- H04 Prompt Composer, H05 Tool Surface Builder — at `ContextManaged → PromptBuilt`
- H06 Stream Demultiplexer — during `ModelCalling`
- H07 Tool Call Resolver — at `ModelCompleted`
- H08 Tool Dispatcher — at `ToolDispatching`
- H09 Hook Pipeline — at `pre_prompt`, `pre_dispatch`, `post_model`, `post_turn`, `on_error`
- H02 Step Recorder — at **every** state transition and every meaningful sub-step
- `ModelGateway::stream(model_input, ctx)` — at `PromptBuilt → ModelCalling`

**Called by**: Runtime layer (a session task spawned by `cogito-core::runtime`).

## Critical invariants

1. **Every state transition writes an event before the transition completes.** If a crash happens after the event is durably recorded but before the next call returns, H03 must be able to reconstruct state from the event alone.
2. **Brain never propagates `Err` upward from tool / hook calls.** All failures arrive as `ToolResult::Error` or `HookDecision::Reject` and are recorded as events; only Runtime-level errors (DI failure, store I/O failure) escape as `TurnOutcome::Failed`.
3. **Panics in tools, hooks, or gateways are caught at the Runtime boundary** (panic-catch around the H01 task). A panic fails one turn; it never brings down the process.
4. **Same session may be entered by another Brain instance after crash.** H03 alone decides where to resume; H01 must accept that decision and start from there without checking external state.
5. **Tools dispatched sequentially in v0.1.** Parallel dispatch is a 0.x option gated by a strategy flag.

## Open design questions

- Pause semantics for the consumer: does the consumer see `Paused { job_id }` and decide whether to await/cancel, or does the Runtime auto-resume on `JobCompleted`? Initial answer: Runtime auto-resumes via a subscription to `JobManager`; consumer just polls turn state.
- Multiple tool calls in one model response: dispatch order = order emitted by model. If one fails, do remaining tools still run? Initial answer: yes (all dispatched, each gets its own `ToolResult`); the model decides next turn based on full result set.
- **Context management mechanism** — deferred to ADR-0008 (Context Management initiative). The architectural slot is locked by this PR: H11 lives at the `Init → ContextManaged` transition, is allowed to do I/O, and writes its own events via H02. What's open: trigger policy (token-threshold vs strategy-flag), summarization model choice, exact `EventPayload` variants needed (at minimum `ContextCompacted`), `ContextManager` trait placement (cogito-protocol vs cogito-core internal), and whether `pre_context`/`post_context` hook lifecycle points should exist. See `docs/components/H11-context-manage.md` for the placeholder.

## Testing strategy

- **Unit**: each state-transition function tested in isolation with mocked dependencies (`MockToolProvider`, `MockModelGateway`, in-memory store).
- **Integration**: full turn against scripted mock model + scripted tool provider; verify the event sequence matches the golden trace.
- **Chaos** (`crates/cogito-core/tests/resume_chaos.rs`): inject a crash between every adjacent pair of events; on restart, H03 + H01 must reach a semantically equivalent end-state.
- **Property**: arbitrary turn scripts produce event sequences that satisfy "every state transition is preceded by its corresponding event".

## References

- ARCHITECTURE.md §"Turn state machine"
- ADR-0003 (state-machine Turn Driver)
- AGENTS.md §"Inviolable design principles" #1, #3, #4

## Implementation note (v0.1)

H01 runs as a per-turn tokio task spawned by `SessionActor`
(`crates/cogito-core/src/runtime/actor.rs`). The FSM is an `enum
TurnState` where each variant carries the data its transition needs
(prompt, stream, surface, strategy), so the type system enforces
state-data invariants and forbids skipped transitions.

A single TurnDriver task = one `input → final answer or paused`
cycle. Multi-turn tool loops are an *inner* loop within the FSM
(re-entering `TurnState::Init` after `DispatchOutcome::AllSync`); a
paused turn ends the current task and is later resumed by *another*
TurnDriver task that starts at `TurnState::ToolDispatching` with the
async result. The actor coordinates the handoff.

See `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
§5 for the FSM pseudocode and ADR-0006 for the load-bearing decisions.

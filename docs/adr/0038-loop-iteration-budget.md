# ADR-0038: Agent-loop iteration budget — enforce `HarnessStrategy::max_turns`

## Status

Proposed (2026-06-07) — design only; implementation is a follow-up (no code
landed with this ADR).

Related: ADR-0003 (Turn Driver as explicit state machine; H01 owns loop
termination), ADR-0006 (Runtime + H01 execution model), ADR-0026 (strategy
registry; `HarnessStrategy` is the per-turn knob set), ADR-0005 (production
scope / quality gates), ADR-0007 (additive event-log evolution).

## Context

A "controllable harness" must guarantee its agent loop terminates. The H01 FSM
(`Init → ContextManaged → PromptBuilt → ModelCalling → ModelCompleted →
ToolDispatching → Init …`) has these stop conditions today:

1. **Normal completion** — the model returns `end_turn` with zero `tool_use`
   blocks → `Completed` (`transitions/model_completed.rs`).
2. **Async pause** — a tool returns `InvokeOutcome::Async(job_id)` → `Paused`.
3. **Unproductive tool loop** — `MAX_CONSECUTIVE_TOOL_ERRORS = 4`
   (`turn_driver/state.rs:26`) consecutive rounds where *every* tool call fails
   validation/dispatch → `Failed`. This catches a model that cannot satisfy a
   tool schema.
4. **Runtime faults** — store I/O, gateway hard error, panic, hook reject, or a
   consumer-set wall-clock `tokio::time::timeout` (`TurnFailureReason::TurnTimedOut`)
   → `Failed`.

None of these bounds a **productive-but-non-terminating** loop: a model that
keeps emitting tool calls that keep *succeeding* loops indefinitely. The only
ceilings are the context window and an optional wall-clock timeout — neither is
an iteration control, and a fast model can run a very large number of cheap
iterations under any timeout. For an embeddable runtime whose consumers pay for
every model call, an unbounded inner loop is a missing control.

The control is already half-specified and unwired:

- `HarnessStrategy::max_turns: u32` exists with a default of `16`
  (`cogito-protocol/src/strategy.rs:41` / `:70`), and the YAML registry exposes
  `max_turns: Option<u32>` (`cogito-strategy/src/schema.rs:46`).
- Its doc comment already states the intended behavior verbatim: *"Safety
  budget: maximum number of inner-loop iterations (Init -> ToolDispatching ->
  Init -> ...) before H01 stops the turn with
  `TurnFailureReason::MaxTurnsExceeded`."*
- But nothing reads `max_turns`, and `TurnFailureReason` has **no**
  `MaxTurnsExceeded` variant (`cogito-protocol/src/turn.rs`). The field is dead
  and the promised failure reason does not exist.

This ADR wires the control the codebase already designed.

## Decision

### 1. Add `TurnFailureReason::MaxTurnsExceeded { turns: u32 }`

An additive struct variant on the existing `#[non_exhaustive]`,
`#[serde(tag = "kind")]` enum. Additive per ADR-0007 (b-档 forward
compatibility) — no `SCHEMA_VERSION` bump, downstream `match` arms unaffected.
It carries the budget that was hit so a consumer can branch and, if it chooses,
resume with a larger budget.

### 2. Enforce in H01, counting model calls

Define **one inner-loop iteration = one model call within the turn.** A turn's
iteration count is the number of `EventPayload::ModelCallStarted` events since
its `TurnStarted` — which means it is **replay-derivable** and therefore
compatible with the "rebuild from the event log" rule (AGENTS.md §3): on resume,
H03 reconstructs the count from the log, exactly as it reconstructs the rest of
the turn. In the hot path the count is mirrored in-memory on `TurnCtx` (the same
pattern as `consecutive_tool_errors`) so the FSM does not rescan the log per
iteration.

Enforcement point: when the FSM is about to issue the next model call after a
`ToolDispatching` round and the count has reached `max_turns`, the Turn Driver
records `TurnFailed { MaxTurnsExceeded { turns } }` and transitions to `Failed`
**instead of** calling the model again (write-before-transition, ADR-0003). The
budget therefore bounds **model calls, not tool calls**: a single model call may
emit any number of tool calls; the budget is the loop-iteration count the doc
comment describes.

`max_turns` defaults to `16` (unchanged). The YAML `Option<u32>` maps `None` →
the strategy default. The budget is per-strategy, so different agent modes
(ADR-0026) tune their own ceiling.

### 3. On-hit policy = fail (terminal), deliberately not summarize/continue

When the budget is hit the turn ends `Failed { MaxTurnsExceeded }`. This is the
honest primitive and matches the existing `strategy.rs` intent. We deliberately
do **not** bake "synthesize a final answer" (LangChain `early_stopping_method =
"generate"` / Hermes degrade) or "auto-continue N more windows" (Hermes
proposal) into core — both are **policy**, and cogito is mechanism (ADR-0014
ethos: behavior/authorization layered by the consumer). The consumer observes
`TurnOutcome::Failed { reason: MaxTurnsExceeded }` and may:

- **Continue** — resume the session (ADR-0034 `get_session`) or `submit` a
  follow-up turn with a higher `max_turns` strategy (the Claude Agent SDK
  `error_max_turns` → resume pattern).
- **Summarize** — submit one more turn with a tool-stripped strategy
  (`ToolFilter::Allow(vec![])`) to force a final synthesis.
- **Stop** — surface the partial result.

### 4. Three independent stop conditions, three reasons

`MaxTurnsExceeded` (productive-but-too-long) is orthogonal to
`MAX_CONSECUTIVE_TOOL_ERRORS` (unproductive all-error loop — stays a private
`const`, not a strategy knob) and to `TurnTimedOut` (wall-clock). Each is a
distinct control with a distinct `TurnFailureReason`; none subsumes the others.

## Consequences

What becomes easier:

- The agent loop is bounded by default. A runaway tool-calling model cannot burn
  unbounded tokens/cost in a single turn. The ceiling is per-strategy and
  already configurable via the YAML registry.
- Consumers get a typed terminal (`MaxTurnsExceeded { turns }`) they can branch
  on to implement continue / summarize / stop — without that policy living in
  core.
- The dead `max_turns` field and the dangling `MaxTurnsExceeded` reference in
  the doc comment are reconciled with reality.

What we give up / accept:

- A hard fail at the budget is a blunt instrument; a consumer wanting graceful
  degradation must implement it (recipes above). We judge this the correct
  layering for an embeddable core.
- The "iteration count = model-call count" definition must stay consistent
  between the live FSM counter and the H03 resume re-derivation, or resume could
  mis-count. This is the implementation risk and is covered by the resume chaos
  tests (`make chaos`): a turn paused/resumed mid-loop must resume with the same
  count.

## Alternatives considered

- **Rely only on the wall-clock `TurnTimedOut`.** Rejected: time is not
  iterations. A fast model can run many cheap iterations under any timeout, and
  not every consumer sets a turn timeout. An iteration budget is the direct
  control the doc comment already promised.
- **Count tool calls instead of model calls.** Rejected: one model call can emit
  N tool calls, so a tool-call budget depends on fan-out width rather than on the
  `Init → ToolDispatching → Init` loop iteration. Model-call count is the
  iteration.
- **Make summarize / auto-continue the default on hit.** Rejected as policy in
  core (Decision 3). Reserved as consumer recipes.
- **Enforce in the Runtime layer rather than H01.** Rejected: loop termination
  is a Brain/Turn-Driver invariant (ADR-0003), and the budget must be re-derived
  from the event log on resume — that is H01/H03 territory, not Runtime's.

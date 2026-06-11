# ADR-0040: Turn-level truncation signal via `TurnCompleted.stop_reason`

## Status

Accepted (2026-06-11) — implemented. Additive, backward-compatible: a new
optional `stop_reason` field on the broadcast `StreamEvent::TurnCompleted`,
plus a `tracing::warn!` when a turn ends on `StopReason::MaxTokens`. No new
`TurnOutcome` / `TurnFailureReason` variant; the persisted event schema is
unchanged. Strategy-level policy (fail / auto-continue) and replay-parity
persistence are explicitly deferred to a future ADR.

Related: ADR-0003 (FSM terminal states + write-before-transition), ADR-0007
(additive wire-contract evolution), ADR-0011 (subagent observability bridge),
ADR-0038 (`MaxTurnsExceeded` — the budget-exhaustion terminal this is compared
against). Raised by a consumer reverse-requirement (praxis RR-8), tracked as
cogito issue #69 part 1.

## Context

The model adapters classify a `max_tokens` cutoff correctly
(`StopReason::MaxTokens`), and the H06 demux persists it in
`EventPayload::ModelCallCompleted { stop_reason, usage }`. So the **event log
holds the truth**.

But the turn driver never looks at it. `model_completed::transit` branches only
on whether tool calls are present: a truncated output with no complete
`tool_use` block takes the happy path — `record_turn_completed(Completed)` +
`TurnState::Completed` with the truncated text as `final_assistant_content`.
`output.stop_reason` is dropped on the floor.

Consequence for consumers: a `MaxTokens`-truncated turn is indistinguishable at
the turn level from a successful one — same `TurnCompleted { outcome: Completed }`
broadcast, half-finished assistant text presented as the final answer. No
`StreamEvent` carries the model-call `stop_reason` live, so a subscriber cannot
see truncation at the turn boundary without already knowing to scan the
`ModelCallCompleted` events. In praxis's first end-to-end run this was the
single most expensive failure to diagnose: with a small `max_tokens` (4096),
mid-task turns kept "completing" with truncated output, and locating the cause
required a mitm proxy to read `finish_reason=length` off the wire.

The question this ADR settles: how should cogito surface a `MaxTokens`
truncation at the **turn** level?

## Decision

Carry the turn's terminal stop reason on the **broadcast**
`StreamEvent::TurnCompleted`, and emit a `tracing::warn!` on truncation.

1. **Protocol.** Add `stop_reason: Option<StopReason>` to
   `StreamEvent::TurnCompleted`, with
   `#[serde(default, skip_serializing_if = "Option::is_none")]` — the same
   additive-optional pattern as the existing `subagent_call_id` field
   (ADR-0007). A live subscriber detects truncation with
   `stop_reason == Some(StopReason::MaxTokens)`; `matches!(.., { .. })`
   consumers are unaffected.

2. **Recorder.** `StepRecorder::record_turn_completed` takes a
   `stop_reason: StopReason` and broadcasts it. The value is the **final** model
   call's stop reason: `record_turn_completed` is reached from
   `model_completed::transit` only when no tool calls remain, so `output.stop_reason`
   at that point is exactly the reason the turn ended.

3. **Diagnosability.** `model_completed::transit` emits `tracing::warn!` when the
   terminal `stop_reason` is `MaxTokens`, so the truncation is visible in logs
   without any consumer wiring.

4. **Subagent bridge.** `tag_subagent` preserves `stop_reason` when re-tagging a
   forwarded `TurnCompleted` for the parent stream (ADR-0011).

### What this deliberately does NOT do

- **No new terminal variant.** `TurnOutcome` stays `Completed` for a truncated
  turn. Truncation is not a Runtime failure (unlike `MaxTurnsExceeded`, which is
  a `TurnFailureReason`): the model produced output, it is just incomplete, and
  the partial text remains available as `final_assistant_content`. A
  consumer that wants to treat truncation as fatal can do so from the signal.

- **No persisted turn-event field.** `EventPayload::TurnCompleted { outcome }`
  is unchanged. The turn-level truth is already in the immediately-preceding
  persisted `ModelCallCompleted { stop_reason }`, so duplicating it onto the
  turn event would be a redundant schema change with wide constructor churn for
  no new information. The accepted limitation: a replay-reconstructed
  `StreamEvent::TurnCompleted` (e.g. `cogito-tui` history) carries
  `stop_reason: None`; such consumers can read the adjacent `ModelCallCompleted`
  if they need the flag on replay. Live broadcast was the gap that hurt.

- **No strategy policy.** A `HarnessStrategy` knob (fail-the-turn /
  auto-continue / ignore on `MaxTokens`) is out of scope. Signaling alone
  unblocks consumers; an auto-continue loop carries its own budget and
  termination concerns and should get its own ADR if a consumer needs it.

## Consequences

- Consumers distinguish truncation from a clean end-of-turn at the turn
  boundary, live, without correlating `ModelCallCompleted` events.
- Operators see truncation in logs immediately (`warn!`), removing the
  mitm-proxy diagnosis step.
- The change is additive and backward-compatible; no migration.
- Replay/live asymmetry (`None` on reconstructed events) is the one rough edge,
  documented above and cheap to revisit if a replay consumer needs parity.

## Alternatives considered

- **`TurnFailureReason::OutputTruncated` (fail the turn).** Consistent with the
  `MaxTurnsExceeded` precedent and gives a clean terminal, but reframes a turn
  that *did* produce (partial) output as a failure and pushes that text out of
  `final_assistant_content`. Rejected: truncation is a completion caveat, not a
  Runtime failure; a consumer can still escalate it to failure from the signal.

- **New `TurnOutcome::Truncated` variant.** Most explicit, but the heaviest
  surface change — every consumer `match` on `TurnOutcome` must handle it, and
  it introduces new persisted-event semantics. Rejected as disproportionate to
  a signal that fits cleanly on the existing terminal event.

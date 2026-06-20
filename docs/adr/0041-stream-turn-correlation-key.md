# ADR-0041: Per-message correlation id (`MessageId`) across the live stream and the persisted log

## Status

Accepted (2026-06-20) — implemented. Introduces a dedicated per-assistant-message
identifier minted at message-open, carried on a new live message-open
`StreamEvent` and on the message's persisted events, plus an auxiliary `turn_id`
on every turn-scoped `StreamEvent`. Both persisted additions are
additive-optional fields (`#[serde(default, skip_serializing_if = ...)]`,
ADR-0007) — no `SCHEMA_VERSION` bump, no migration; old logs read back with
`None`.

Related: ADR-0007 (additive wire-contract evolution), ADR-0011 (subagent
observability bridge — the `subagent_call_id` pattern reused), ADR-0019
(additive `EventPayload` field without a version bump — the precedent for the
persisted side here), ADR-0040 (`TurnCompleted.stop_reason` — the
additive-optional broadcast field this began from). Raised by a consumer
reverse-requirement (praxis RR-11), tracked as cogito issue #74.

## Context

cogito fans out two parallel streams from one turn (ADR-0006 §7): the
**persisted** `ConversationEvent` log (append-only source of truth) and the
**live** `StreamEvent` broadcast (real-time, non-persisted, lossy for lagged
subscribers). A consumer that renders the live stream **and** reads history —
for reconnect/resubscribe dedup, or to upsert an in-flight message into a store
it later reads back — needs the streamed message and the persisted message to
share one stable identifier.

The first cut of this ADR put `turn_id` on the live stream. A consumer review
(praxis RR-11) showed `turn_id` is the wrong **granularity** for a *message*
identity:

- A single turn holds more than one message: the user input plus one or more
  assistant messages. A HITL `message_ask_user` pause splits one turn's
  assistant output into two distinct messages (pre-pause and post-answer).
- Keying a message id on `turn_id` therefore collides: the user and assistant
  messages of a turn share an id, and the two assistant messages of a HITL turn
  share an id. Patching that needs role/sub-index discriminators baked into the
  id — pushing turn-structure and role into what should be an opaque,
  message-scoped value (and `role` is already a separate field).

The identity also **cannot be derived consumer-side**: the live stream is
fine-grained (`TextDelta`/`ThinkingDelta` per chunk) while the persisted log is
batched (`AssistantMessageAppended` coalesces deltas at the content-block
boundary). The "first live delta" does not line up one-to-one with the "first
persisted event", so a consumer cannot reconstruct a persisted-event identity
from the live side. The identity must be **minted by cogito at message-open and
carried on both sides**.

## Decision

Mint a dedicated `MessageId` when an assistant message opens, broadcast it on a
new live message-open event, and stamp it on the message's persisted events.
Keep `turn_id` on the stream as an auxiliary turn-linkage field.

1. **Message granularity = one model call.** An assistant message is the output
   of a single model call. This matches the Anthropic API model (each model
   completion is one assistant message; tool results return as the next user
   message) and represents the HITL split for free: a pause ends the model call,
   resume starts the next, so pre-pause and post-answer land in separate
   messages without any special case. A sync tool-loop turn likewise yields one
   message per model call.

2. **`MessageId` newtype** (`cogito-protocol::ids`, same ULID-backed
   `id_newtype!` as `TurnId`/`EventId`). Opaque, stable, no role/turn structure
   encoded.

3. **Mint point = model-call-start.** H02 `StepRecorder::record_model_call_started`
   mints a fresh `MessageId`, holds it as `current_message_id`, and broadcasts a
   new `StreamEvent::AssistantMessageStarted { message_id, turn_id,
   subagent_call_id }` — the message-open live event. A subscriber learns the id
   before any content streams and folds the following deltas into that message.

4. **Live carry.** `message_id` rides (optional, serde-default) on the
   delta/tool events that compose a message — `TextDelta`, `ThinkingDelta`,
   `ToolDispatchStarted`, `ToolDispatchEnded` — so each event self-attributes to
   its message even under interleaving (subagent forwarding, reconnect).

5. **Persisted identity.** `message_id: Option<MessageId>` is stamped (additive,
   serde-default — ADR-0007/0019, no `SCHEMA_VERSION` bump) on the persisted
   events that compose an assistant message: `AssistantMessageAppended`,
   `ThinkingBlockRecorded`, `ToolUseRecorded`. A history projection groups these
   by `message_id` into one assistant message, arriving at the *same* id the
   live subscriber saw. The in-flight text/thinking buffers capture
   `current_message_id` at open so the flushed event carries the right id.

6. **`turn_id` stays as the auxiliary turn-linkage field** on every turn-scoped
   `StreamEvent` variant (the original RR-11 ask, now demoted from primary). The
   consumer keeps role and turn linkage in their own fields and keys
   `Message.id` on the bare `message_id`. `turn_id` is additive, serde-default,
   broadcast-only.

7. **Subagent bridge.** `tag_subagent` (ADR-0011) preserves the child's
   `message_id` and `turn_id` and (re)stamps only `subagent_call_id`, including
   for the new `AssistantMessageStarted` variant.

### Why a dedicated id (not the first composing `event_id`)

Reusing the first composing event's `EventId` avoids a new concept but is
circular: a projection must already know message boundaries to know *which*
event_id is "the message id", and the live message-open event would have to
pre-mint that `EventId` before the batched event is written. A dedicated
`MessageId` stamped on every composing event makes grouping trivial and the
live/persisted identity exact. (Issue #74 preferred form 1.)

### What this deliberately does NOT do

- **No `SCHEMA_VERSION` bump.** Both persisted additions are optional fields;
  old logs deserialize with `None` (ADR-0007/0019 precedent).
- **No role/turn structure in the id.** `role` and `turn_id` stay in their own
  fields; `MessageId` is opaque.
- **No `seq` exposure.** `seq` remains a per-event ordering field, not a message
  identity. It can be added as an auxiliary later if a consumer needs it.
- **No message-close event (yet).** A message ends at the next
  `AssistantMessageStarted` or at turn end; a dedicated close event can be added
  additively if a consumer needs an explicit boundary.

## Consequences

- A live subscriber and a history reader independently derive the *same*
  `MessageId` for the same assistant message — fixing the empty/unkeyable
  streamed-message id (reconnect dedup, in-flight upsert) without role/turn
  discriminators leaking into the id.
- `turn_id` remains available for turn linkage as a separate field.
- The persisted log gains an opaque message grouping key, replay-stable because
  it is written into the events themselves (like `EventId`).
- Surface/test churn: a new `StreamEvent` variant and new optional fields mean
  `match` arms and constructors mention the new shape. Mechanical, no behavior
  change. The formerly-unit `TurnPaused`/`TurnResumed`/`TurnCancelled` become
  struct variants (byte-identical wire form when fields are `None`).
- Replay-reconstructed live events carry `None` for the optional fields; such
  consumers read the ids off the persisted events they reconstructed from.

## Alternatives considered

- **`turn_id` as the message id (original ask).** Wrong granularity — collides
  across the messages of one turn (user vs assistant, HITL pre/post). Kept as an
  auxiliary turn-linkage field instead.
- **First composing `event_id` as the message id (issue form 2).** Circular
  boundary detection + pre-minting; rejected in favor of a dedicated id.
- **Per-text-block or per-turn message granularity.** Per-block over-splits a
  single model response with two text blocks; per-turn under-splits and cannot
  represent the HITL split. Per-model-call is the natural boundary.

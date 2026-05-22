# JSONL Event Log — schema v1

> **Status**: stable for the cogito 0.x line. Governed by
> `ConversationEvent::schema_version` per ADR-0005 §4 #2 and ADR-0007.
>
> **Audience**: external (Go / Python / Node) services reading the
> conversation event log; cogito library consumers writing custom
> backends.

## File layout

- One file per session at `<root>/<session_id>.jsonl`.
- `<session_id>` is a [ULID](https://github.com/ulid/spec) rendered as
  the canonical 26-character Crockford base32 string.
- Lines are UTF-8 JSON objects terminated by `\n`. No leading whitespace.
- The file is **append-only** during writer activity. Readers MAY tail
  the file but MUST handle truncated final lines gracefully (the writer
  may be mid-write).
- The first non-empty line of every file is the `SessionStarted` event
  (`type = "session_started"`).
- Lines are in strict ascending `seq` order. Gaps in `seq` are
  forbidden; readers encountering a gap SHOULD treat the file as
  corrupt.

## Line schema (envelope)

```json
{
  "schema_version": 1,
  "event_id": "01J9C0R0K3T0X8K3T0X8K3T0X8",
  "session_id": "01J9C0R0K0SESSION0SESSION0",
  "turn_id": "01J9C0R0K0TURN0TURN0TURN00",
  "seq": 42,
  "ts": "2026-05-18T10:00:00.123Z",
  "type": "tool_use_recorded",
  "data": {"call_id": "toolu_01", "tool_name": "read_file", "args": {"path": "/tmp/x"}}
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `schema_version` | int | ✓ | `1` for this version. Bumped together with the envelope or any payload shape on breaking change. |
| `event_id` | ULID string | ✓ | Globally unique, monotonic per writing process. |
| `session_id` | ULID string | ✓ | Identifies the session. Matches the filename. |
| `turn_id` | ULID string \| `null` | ✓ | `null` for session-level events (e.g. `session_started`). |
| `seq` | uint64 | ✓ | Monotonic per session, starts at 0. Used by Resume Coordinator. |
| `ts` | RFC 3339 timestamp (UTC) | ✓ | Wall-clock at write time. Use for display only; causality is `seq`. |
| `type` | string | ✓ | One of the payload variants below (snake_case); 10 shipped as of Sprint 2, with `model_call_completed` pending Sprint 3 P2.2. |
| `data` | object | ✓ | Variant-specific payload; see "Payload variants" below. |

## Payload variants (12 shipped; 1 planned)

The original 9 variants (`session_started` through `turn_failed`) shipped in Sprint 1.
Sprint 2 added `model_call_started` (documented below). Sprint 3 P2.2 will add
`model_call_completed` (see its section below — marked pending). Sprint 4.7 added
`thinking_block_recorded`. Sprint 5 added `hook_rejected`.

### `session_started`

```json
{"type": "session_started", "data": {"meta": {"cogito_version": "0.1.0", "strategy": "default", "model": "claude-sonnet-4-6", "user_id": "u_42"}}}
```

Always the first line of a file. `meta` carries optional consumer-supplied
metadata (see `SessionMeta` schema in `docs/schemas/conversation-event-v1.json`).

### `turn_started`

```json
{"type": "turn_started", "data": {"user_input": [{"type": "text", "data": {"text": "read /tmp/x"}}]}}
```

`user_input` is a `Vec<ContentBlock>`. v1 supports `text`, `tool_use`,
`tool_result` content block types.

### `assistant_message_appended`

```json
{"type": "assistant_message_appended", "data": {"text": "Reading /tmp/x now."}}
```

One per wire-protocol content_block_stop for an assistant text block.
The recorder does NOT persist individual streaming deltas — they appear
only on the live `StreamEvent` channel.

### `thinking_block_recorded`

Recorded by H02 when a reasoning/"thinking" content block is sealed by
the provider. Sibling to `assistant_message_appended` and
`tool_use_recorded` — one event per completed block, ordered by
envelope `seq`. Within one assistant turn, `thinking_block_recorded`
events always precede `assistant_message_appended` and
`tool_use_recorded` for the same turn (the provider emits thinking
first; seq order preserves that). H04 walks events in `seq` order to
rebuild the assistant message's `content` array as
`[Thinking, Text, ToolUse, …]`.

**Payload**:

| Field | Type | Required | Description |
|---|---|---|---|
| `text` | string | yes | Full reasoning text. Empty string for safety-redacted blocks (e.g. Anthropic `redacted_thinking`). |
| `provider_opaque` | object \| null | yes (may be null) | Provider-specific round-trip payload. Schema not interpreted by cogito; see provider docs. |

**Concrete `provider_opaque` shapes observed in production**:

- Anthropic plain `thinking`: `{"signature": "<base64>"}`
- Anthropic `redacted_thinking`: `{"data": "<opaque>"}` (no signature; the reasoning itself is encrypted)
- OpenAI Responses `reasoning`: `{"item_id": "<id>", "encrypted_content": "<opaque>"}`
- OpenAI-compat (`chat.completions`-style backends like DeepSeek-R1, QwQ, vLLM with `--enable-reasoning`): `null`

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":11,"ts":"2026-05-22T10:00:01.100Z","type":"thinking_block_recorded","data":{"text":"I should grep for the symbol.","provider_opaque":{"signature":"abc123"}}}
```

Added: ADR-0019 (`SCHEMA_VERSION` unchanged at 1 — additive variant per ADR-0007 precedent).

### `model_call_started`

Recorded by H01 Turn Driver at the `PromptBuilt → ModelCalling` transition boundary,
immediately before the gateway stream opens. Documents which model is being called
for this turn.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `model` | string | Model identifier (e.g., `"claude-opus-4-7"`, `"gpt-5"`). Source of truth: the value resolved by H10 strategy selection. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":4,"ts":"2026-05-20T10:00:00Z","type":"model_call_started","data":{"model":"claude-opus-4-7"}}
```

Always followed by exactly one `model_call_completed` event (Sprint 3 P2.2 — see next
section) in the same turn, unless the actor crashed mid-call.

Added: Sprint 2.

### `model_call_completed`

*Sprint 3 P2.2 — not yet shipped.* Sealing event for one model call. Will be written by H06 Stream Demultiplexer when the gateway stream emits `MessageCompleted` (Anthropic `message_delta` with stop_reason; OpenAI `finish_reason`). Always follows a preceding `model_call_started` event in the same turn; `seq` is strictly larger.

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

Added: Sprint 3 P2.2 (planned for 2026-05-20+). No `SCHEMA_VERSION` bump (additive, b-档 compatible per ADR-0007 §"Additive variant precedent").

### `tool_use_recorded`

```json
{"type": "tool_use_recorded", "data": {"call_id": "toolu_01", "tool_name": "read_file", "args": {"path": "/tmp/x"}}}
```

### `tool_result_recorded`

```json
{"type": "tool_result_recorded", "data": {"call_id": "toolu_01", "result": {"output": ["file contents"]}}}
```

`ToolResult` uses `#[serde(rename_all = "snake_case")]`, so the variant tag
is lowercase `output` (not `Output`). The `Output` variant carries
`Vec<serde_json::Value>` — opaque JSON values, not nested `ContentBlock`s.
The v0.2 multimodal upgrade will swap this for `Vec<ContentBlock>`
(`ToolResult` doc comment in `cogito-protocol::tool`).

### `turn_paused`

```json
{"type": "turn_paused", "data": {"job_id": "01J9C0R0K0JOB0JOB0JOB0JOB0"}}
```

### `job_completed_recorded`

```json
{"type": "job_completed_recorded", "data": {"job_id": "01J9C0R0K0JOB0JOB0JOB0JOB0", "outcome": { /* JobCompletionEvent */ }}}
```

### `turn_completed`

```json
{"type": "turn_completed", "data": {"outcome": {"kind": "completed"}}}
```

`TurnOutcome` is internally tagged with `tag = "kind"` and
`rename_all = "snake_case"`, so even the unit `Completed` variant wears
the discriminator object `{"kind": "completed"}`.

### `hook_rejected`

Recorded by H01 Turn Driver immediately before the turn transitions to
`Failed` when an H09 hook returns `HookDecision::Reject`. This event
gives downstream readers a structured, queryable record of which hook
fired and why, without having to parse the free-text `reason` field in
`turn_failed`.

**Ordering**: appears immediately before `turn_failed` in the event log.
When a `pre_prompt` hook rejects, the sequence is
`… model_call_started → hook_rejected → turn_failed`. When a
`pre_dispatch` hook rejects, the sequence is
`… tool_use_recorded → hook_rejected → turn_failed`.

**Payload**:

| Field | Type | Required | Description |
|---|---|---|---|
| `hook_name` | string | yes | Stable identifier of the hook that rejected (from `HookHandler::name()`). Kebab-case; unique within a deployment. |
| `point` | string enum | yes | Lifecycle point at which the rejection occurred. One of: `pre_prompt`, `pre_dispatch`, `post_model`, `post_turn`, `on_error`. |
| `reason` | string | yes | Human-readable rejection reason provided by the hook. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":5,"ts":"2026-05-22T10:00:01.200Z","type":"hook_rejected","data":{"hook_name":"sensitive-content","point":"pre_prompt","reason":"AWS key pattern detected in prompt"}}
```

Added: Sprint 5 as an additive variant under ADR-0007. No
`SCHEMA_VERSION` bump.

### `turn_failed`

```json
{"type": "turn_failed", "data": {"reason": {"kind": "turn_timed_out"}}}
```

`TurnFailureReason` is internally tagged with `tag = "kind"` and
`rename_all = "snake_case"`. Variants in this enum are
`store_unavailable`, `model_gateway_failed`, `turn_panicked`,
`turn_timed_out`, `hook_rejected`. The canonical fixture uses
`turn_timed_out`.

## Forward compatibility

- **Additive changes** (new `EventPayload` variant, new optional field
  on `SessionMeta`) do NOT bump `schema_version`. Readers MUST tolerate
  unknown `type` values (skip the line or log) and MUST ignore unknown
  object keys.
- **Breaking changes** (rename a field, change a field's type, remove a
  variant) bump `schema_version`. cogito ships a migration tool for
  every breaking 0.x change; readers SHOULD pin their understanding to
  a known `schema_version` window.

## Validation

A JSON Schema artifact is generated from the Rust source at
`docs/schemas/conversation-event-v1.json`. CI ensures it does not drift
from the implementation. External services SHOULD use it for typed
deserialization.

## Canonical example

A worked sample of the original 9 variants in one session is at
`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`.

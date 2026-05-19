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
| `type` | string | ✓ | One of the 9 payload variants below (snake_case). |
| `data` | object | ✓ | Variant-specific payload; see "Payload variants" below. |

## Payload variants (9)

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

### `tool_use_recorded`

```json
{"type": "tool_use_recorded", "data": {"call_id": "toolu_01", "tool_name": "read_file", "args": {"path": "/tmp/x"}}}
```

### `tool_result_recorded`

```json
{"type": "tool_result_recorded", "data": {"call_id": "toolu_01", "result": {"Output": [{"type": "text", "data": {"text": "file contents"}}]}}}
```

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
{"type": "turn_completed", "data": {"outcome": "Completed"}}
```

### `turn_failed`

```json
{"type": "turn_failed", "data": {"reason": "Cancelled"}}
```

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

A worked sample of all 9 variants in one session is at
`testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`.

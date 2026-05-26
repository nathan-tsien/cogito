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
| `type` | string | ✓ | One of the payload variants below (snake_case). Variant set has grown additively under ADR-0007 each sprint; consult the "Payload variants" header for the current count. |
| `data` | object | ✓ | Variant-specific payload; see "Payload variants" below. |

## Payload variants (13 shipped; 1 planned)

The original 9 variants (`session_started` through `turn_failed`) shipped in Sprint 1.
Sprint 2 added `model_call_started` (documented below). Sprint 3 P2.2 will add
`model_call_completed` (see its section below — marked pending). Sprint 4.7 added
`thinking_block_recorded`. Sprint 5 added `hook_rejected`. Sprint 8 added
`job_submitted` (next to `job_completed_recorded`).

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

### `job_submitted`

Written by H08 Tool Dispatcher when `ToolProvider::invoke` returns
`InvokeOutcome::Async(job_id)`. Recorded **before** H08 registers the
`on_complete` sink with `JobManager`, and **before** the subsequent
`turn_paused` event — so a crash between record and registration leaves
a recoverable state where H03 can synthesize a `JobOutcome::Failed` for
the unknown job (the in-memory `LocalJobManager` does not survive
restart). This event also carries the `call_id` that H03 uses to map
the eventual `job_completed_recorded` back to the originating
`tool_use_recorded` — replacing the legacy Sprint 3 walk-back.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `call_id` | string | The `tool_use` block's `call_id` that produced this async job. Matches a preceding `tool_use_recorded.call_id` within the same turn. |
| `job_id` | ULID string | Identifier minted by `JobManager`. Matches the subsequent `turn_paused.job_id` and the eventual `job_completed_recorded.job_id`. |
| `tool_name` | string | Tool name. Informational; aids debugging / log readers. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":8,"ts":"2026-05-24T10:00:00.500Z","type":"job_submitted","data":{"call_id":"toolu_01","job_id":"01J9C0R0K0JOB0JOB0JOB0JOB0","tool_name":"run_tests"}}
```

Added: Sprint 8 as an additive variant under ADR-0007. No
`SCHEMA_VERSION` bump.

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
`… context_manage_completed → prompt_composed → hook_rejected → turn_failed`.
`pre_prompt` fires at the end of the `ContextManaged` state, before the FSM
transitions to `PromptBuilt`; `model_call_started` only fires at the
`PromptBuilt → ModelCalling` transition and is therefore absent from the
rejection sequence. When a `pre_dispatch` hook rejects, the sequence is
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

## Context management events

Added Sprint 6 (ADR-0008). Four additive `EventPayload` variants; no `SCHEMA_VERSION` bump.

These variants belong to `EventCategory::ContextDecision`. External readers must tolerate unknown `type` values per ADR-0007.

A worked session demonstrating one truncate compaction is at
`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-truncate-v1.jsonl`.

### `context_compacted`

Written by the `Compactor` when it actually compacts history. A no-op compactor (`NoneCompactor`) never writes this event. At most one per turn in v0.1.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `turn_id` | ULID string | Turn during which compaction was decided. |
| `replaced_seq_range` | `[u64, u64]` | Inclusive `[start_seq, end_seq]` covered by this compaction. Boundaries align to turn start/end. |
| `produced_by` | string | `Compactor::id()` — e.g. `"truncate"`. |
| `replacement` | object | What replaces the covered range in projection. `{"kind":"drop"}` for truncation; `{"kind":"summary","text":"...","model":"..."}` for summarization. |
| `token_estimate_before` | u64 \| null | Estimated prompt tokens before compaction (informational). |
| `token_estimate_after` | u64 \| null | Estimated prompt tokens after compaction (informational). |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":12,"ts":"2026-05-23T10:00:02.000Z","type":"context_compacted","data":{"turn_id":"01HFZZZ","replaced_seq_range":[1,8],"produced_by":"truncate","replacement":{"kind":"drop"},"token_estimate_before":1200,"token_estimate_after":300}}
```

### `system_prompt_injected`

Written by `SystemPromptInjector` every turn, even when the suffix is empty (audit invariant). The suffix is appended after `strategy.system_prompt` with a `\n\n` separator when non-empty.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `turn_id` | ULID string | Turn whose system prompt this suffix applies to. |
| `suffix` | string | Text appended to the base system prompt. Empty string for no-op injectors. |
| `contributors` | string[] | Tags identifying what contributed (e.g. `["date", "skill:plan-review"]`). |
| `produced_by` | string | `Injector::id()` — e.g. `"none"`. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":13,"ts":"2026-05-23T10:00:02.100Z","type":"system_prompt_injected","data":{"turn_id":"01HFZZZ","suffix":"","contributors":[],"produced_by":"none"}}
```

### `tool_filter_overridden`

Written by `ToolFilterOverrider` every turn, even when the mode is `Inherit` (audit invariant). H05 reads this event to apply the override on top of `strategy.allowed_tools`.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `turn_id` | ULID string | Turn whose tool surface this override applies to. |
| `mode` | object | `{"kind":"inherit"}`, `{"kind":"intersect","tools":[...]}`, or `{"kind":"replace","tools":[...]}`. |
| `contributors` | string[] | Tags identifying what contributed. |
| `produced_by` | string | `Overrider::id()` — e.g. `"none"`. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":14,"ts":"2026-05-23T10:00:02.200Z","type":"tool_filter_overridden","data":{"turn_id":"01HFZZZ","mode":{"kind":"inherit"},"contributors":[],"produced_by":"none"}}
```

### `context_decision_recorded`

Written by H11 itself after all three traits finish. This is the summary index for the turn: it cross-references the event ids of the three trait events, and captures any per-trait errors from the degrade path.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `turn_id` | ULID string | Turn this decision summary belongs to. |
| `compactions` | ULID string[] | Event ids of `context_compacted` events written this turn (0 or 1 for v0.1). |
| `system_prompt_event` | ULID string | Event id of this turn's `system_prompt_injected`. |
| `tool_filter_event` | ULID string | Event id of this turn's `tool_filter_overridden`. |
| `errors` | object | Per-trait error capture. Fields: `compactor`, `injector`, `overrider` — each is a string (serialized error) or `null` if the trait ran cleanly. |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":15,"ts":"2026-05-23T10:00:02.300Z","type":"context_decision_recorded","data":{"turn_id":"01HFZZZ","compactions":["01HFCCC"],"system_prompt_event":"01HFSSS","tool_filter_event":"01HFTTT","errors":{"compactor":null,"injector":null,"overrider":null}}}
```

## Sprint 7 additive entries (no schema bump)

Added Sprint 7 (ADR-0020). One additive `EventPayload` variant
(`SkillActivated`) plus one additive optional field on the existing
`TurnStarted` payload. No `SCHEMA_VERSION` bump per ADR-0007.

### `turn_started.activate_skills`

New optional field on the existing `TurnStarted` payload.

- Type: `string[]`
- Default on read: `[]` (older fixtures and turns triggered via
  `TurnTrigger::UserText` continue to parse cleanly).
- Source: user-channel skill activations carried with this turn — see
  `TurnTrigger::SkillActivation` and the `/skill <name>` slash command
  wired in by Surface. Sigil-based (model-channel) activations are NOT
  recorded here; H11's `SkillInjector` re-derives them from previous-turn
  assistant text.

### `skill_activated`

Written by `SkillInjector` (H11) — one event per newly activated skill
within a turn. Dedupe rules and channel precedence live in the Sprint 7
spec §11. Cross-references `system_prompt_injected.contributors` (which
holds the same skill names) and the per-turn `system_prompt_event` in
`context_decision_recorded`.

**Payload**:

| Field | Type | Description |
|---|---|---|
| `skill_name` | string | Bare name (`foo`) or `<plugin_id>:<name>` for Plugin scope. |
| `source` | object | Where the skill was discovered. `{"kind":"repo","dir":"<workspace>"}` / `{"kind":"user"}` / `{"kind":"plugin","plugin_id":"<id>"}` / `{"kind":"system"}`. |
| `channel` | object | What triggered the activation. `{"kind":"model_sigil"}` (assistant emitted `$Name` in prior-turn text) or `{"kind":"user_slash"}` (user typed `/skill <name>`). |

**Example**:

```json
{"schema_version":1,"event_id":"01HFXXX","session_id":"01HFYYY","turn_id":"01HFZZZ","seq":2,"ts":"2026-05-23T00:00:00.200Z","type":"skill_activated","data":{"skill_name":"invoice-parser","source":{"kind":"user"},"channel":{"kind":"user_slash"}}}
```

A worked session demonstrating one user-channel skill activation is at
`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl`.

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

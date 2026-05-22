# ADR-0019 Thinking Content Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ADR-0019 — first-class representation of model
reasoning/"thinking" content across `cogito-protocol`, `cogito-core::harness`,
and the two provider adapters (`anthropic`, `openai_compat`), without bumping
`SCHEMA_VERSION` and without rewriting any persisted JSONL.

**Architecture:** Additive enum variants on `ContentBlock`, `EventPayload`,
`ModelEvent`, `StreamEvent`. H02 gains a thinking-block buffer paralleling the
text-block buffer; H06 routes the new `ModelEvent::Thinking*` variants
through it; H04 projects the new `EventPayload::ThinkingBlockRecorded` into
`ContentBlock::Thinking` in its history walk. Anthropic adapter maps the
first-class wire `thinking_delta` / `signature_delta` events; OpenAI-compat
adapter implements both a `delta.reasoning_content` reader and a two-state
SSE parser for inline `<think>...</think>` tags. `provider_opaque` is a
single `Option<serde_json::Value>` slot carrying whatever each provider
requires for verbatim round-trip.

**Tech Stack:** Rust 1.85 edition 2024, `serde` adjacent tagging, `tokio` async,
`async_trait`, `cargo-nextest`. Inviolable rules from CLAUDE.md and ADR-0019.

**Reference:** `docs/adr/0019-reasoning-content-modeling.md` is the single
authoritative spec. Every task below traces to a numbered decision in §1–§4
or a follow-on in that ADR. The §5 walkthrough is the worked example to
keep in mind when sanity-checking event flow.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `crates/cogito-protocol/src/content.rs` | `ContentBlock::Thinking` variant | Modify |
| `crates/cogito-protocol/src/event.rs` | `EventPayload::ThinkingBlockRecorded` variant | Modify |
| `crates/cogito-protocol/src/gateway.rs` | `ModelEvent::ThinkingDelta` + `ThinkingBlockCompleted` variants | Modify |
| `crates/cogito-protocol/src/stream.rs` | `StreamEvent::ThinkingDelta` variant | Modify |
| `docs/schemas/conversation-event-v1.json` | Schema artifact (regenerated) | Regenerate |
| `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl` | Canonical fixture (append new event row) | Modify |
| `crates/cogito-core/src/harness/step_recorder.rs` | H02 `on_thinking_delta` + `on_thinking_block_complete` | Modify |
| `crates/cogito-core/src/harness/stream_demux.rs` | H06 routes `ModelEvent::Thinking*` | Modify |
| `crates/cogito-core/src/harness/prompt.rs` | H04 maps `ThinkingBlockRecorded` → `ContentBlock::Thinking` | Modify |
| `crates/cogito-model/src/anthropic/wire.rs` | Wire types for `thinking` / `signature_delta` / `redacted_thinking` | Modify |
| `crates/cogito-model/src/anthropic/decode.rs` | Anthropic SSE → `ModelEvent::Thinking*` | Modify |
| `crates/cogito-model/src/anthropic/encode.rs` | `ContentBlock::Thinking` → Anthropic wire thinking block | Modify |
| `crates/cogito-model/src/openai_compat/wire.rs` | Add `reasoning_content` field on delta | Modify |
| `crates/cogito-model/src/openai_compat/decode.rs` | `reasoning_content` field reader + `<think>` tag two-state parser | Modify |
| `crates/cogito-model/src/openai_compat/encode.rs` | Honor `include_prior_thinking` (default drop) | Modify |
| `crates/cogito-model/src/provider_config.rs` | Add `include_prior_thinking: bool` field on OpenAI-compat config | Modify |
| `crates/cogito-core/tests/resume_chaos.rs` | New scenario with `ThinkingBlockRecorded` in seq order | Modify |
| `docs/components/H02-step-recorder.md` | Document thinking-block buffer + flush | Modify |
| `docs/components/H04-prompt-composer.md` | Document `ThinkingBlockRecorded` projection | Modify |
| `docs/components/H06-stream-demux.md` | Document `Thinking*` routing | Modify |
| `docs/data-model/jsonl-v1.md` | Document `ThinkingBlockRecorded` (additive, filename unchanged) | Modify |
| `AGENTS.md` | Add two inviolable rules: thinking-block ordering + no-rewrite | Modify |
| `ROADMAP.md` | Add ADR-0019 work item under current iteration | Modify |

**File-size note:** `step_recorder.rs` is already at 457 lines and gains ~80
more for the thinking buffer + methods + tests. Still under the 600-line
threshold the repo informally observes. No split needed.

---

## Phase 1 — Protocol foundation

Each protocol-layer change is **strictly additive** (`#[non_exhaustive]`
enum + new variant). No `SCHEMA_VERSION` bump per ADR-0007 precedent.

### Task 1: `ContentBlock::Thinking` variant

**Files:**
- Modify: `crates/cogito-protocol/src/content.rs`

- [ ] **Step 1: Write the failing roundtrip test**

Add to the `tests` module at the bottom of `content.rs`:

```rust
#[test]
fn thinking_roundtrips_with_provider_opaque() -> serde_json::Result<()> {
    let cb = ContentBlock::Thinking {
        text: "let me think...".into(),
        provider_opaque: Some(serde_json::json!({"signature": "abc123"})),
    };
    let json = serde_json::to_string(&cb)?;
    assert_eq!(
        json,
        r#"{"type":"thinking","data":{"text":"let me think...","provider_opaque":{"signature":"abc123"}}}"#
    );
    let back: ContentBlock = serde_json::from_str(&json)?;
    assert_eq!(cb, back);
    Ok(())
}

#[test]
fn thinking_roundtrips_without_provider_opaque() -> serde_json::Result<()> {
    let cb = ContentBlock::Thinking {
        text: "implicit reasoning".into(),
        provider_opaque: None,
    };
    let json = serde_json::to_string(&cb)?;
    let back: ContentBlock = serde_json::from_str(&json)?;
    assert_eq!(cb, back);
    Ok(())
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-protocol
```

Expected: both tests fail with `no variant named Thinking on ContentBlock`.

- [ ] **Step 3: Add the variant**

In `crates/cogito-protocol/src/content.rs`, inside the `ContentBlock` enum
(after `ToolResult` and before the closing `}`):

```rust
    /// Model reasoning/"thinking" content. Carried inside the assistant
    /// message's content array; placed before Text / ToolUse blocks in
    /// the same message per provider requirements (see ADR-0019 §4).
    /// `provider_opaque` is None for backends that need no round-trip
    /// material (OpenAI-compat), Some for Anthropic (signature) and
    /// OpenAI Responses (encrypted_content + item_id).
    Thinking {
        /// Human-readable reasoning text. May be empty when the provider
        /// returns only an encrypted blob (e.g. Anthropic redacted_thinking).
        text: String,
        /// Provider-defined opaque payload required for next-turn
        /// validation. Schema is provider-specific; cogito does not
        /// interpret the contents.
        provider_opaque: Option<serde_json::Value>,
    },
```

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-protocol
```

Expected: both tests pass; existing tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/content.rs
git commit -m "feat(protocol): add ContentBlock::Thinking variant

Additive variant per ADR-0019 §1. Carries provider-opaque payload
required for verbatim round-trip on Anthropic and OpenAI Responses;
None for OpenAI-compat. No SCHEMA_VERSION bump."
```

---

### Task 2: `EventPayload::ThinkingBlockRecorded` variant

**Files:**
- Modify: `crates/cogito-protocol/src/event.rs`

- [ ] **Step 1: Write the failing test**

In `crates/cogito-protocol/src/event.rs`, find the test
`all_fourteen_variants_roundtrip` and rename it to
`all_fifteen_variants_roundtrip`. Add a new fixture entry inside the
`variants` vec (immediately before `EventPayload::ModelCallCompleted`):

```rust
EventPayload::ThinkingBlockRecorded {
    text: "I should grep for the symbol.".into(),
    provider_opaque: Some(serde_json::json!({"signature": "abc123"})),
},
```

Also add a focused test below the renamed one:

```rust
#[test]
fn thinking_block_recorded_roundtrips() -> serde_json::Result<()> {
    let event = sample_envelope(EventPayload::ThinkingBlockRecorded {
        text: "private chain of thought".into(),
        provider_opaque: Some(serde_json::json!({"item_id": "rs_01"})),
    });
    let json = serde_json::to_string(&event)?;
    assert!(
        json.contains(r#""type":"thinking_block_recorded""#),
        "missing tag: {json}"
    );
    let back: ConversationEvent = serde_json::from_str(&json)?;
    assert_eq!(event, back);
    Ok(())
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-protocol
```

Expected: both tests fail (variant missing).

- [ ] **Step 3: Add the variant**

In `crates/cogito-protocol/src/event.rs`, inside the `EventPayload` enum,
immediately after `ModelCallCompleted { ... }`:

```rust
    /// One reasoning/"thinking" content block has been sealed by the
    /// provider. Sibling to `AssistantMessageAppended` and
    /// `ToolUseRecorded` — one event per completed block, ordered by
    /// envelope `seq`. See ADR-0019 §2. `turn_id` is on the envelope.
    ThinkingBlockRecorded {
        /// Full reasoning text for the sealed block. May be empty when
        /// the provider exposes only an opaque encrypted payload.
        text: String,
        /// Provider-opaque blob (signature / encrypted_content / item
        /// id) that must round-trip verbatim on the next model call.
        /// Schema is provider-specific; cogito does not interpret it.
        provider_opaque: Option<serde_json::Value>,
    },
```

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-protocol
```

Expected: all tests pass, including the renamed `all_fifteen_*`.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/event.rs
git commit -m "feat(protocol): add EventPayload::ThinkingBlockRecorded variant

Additive variant per ADR-0019 §2 and ADR-0007 additive precedent.
SCHEMA_VERSION stays at 1. Sibling to AssistantMessageAppended;
block order across the three assistant-side events is reconstructed
by envelope seq."
```

---

### Task 3: `ModelEvent::ThinkingDelta` and `ThinkingBlockCompleted`

**Files:**
- Modify: `crates/cogito-protocol/src/gateway.rs`

- [ ] **Step 1: Write the failing test**

In `crates/cogito-protocol/src/gateway.rs`, add a test module at the bottom
if one does not exist; if it does, append:

```rust
#[cfg(test)]
mod thinking_tests {
    use super::*;

    #[test]
    fn thinking_delta_roundtrips() -> serde_json::Result<()> {
        let evt = ModelEvent::ThinkingDelta {
            block_index: 0,
            chunk: "I should ".into(),
        };
        let json = serde_json::to_string(&evt)?;
        assert!(json.contains(r#""kind":"thinking_delta""#), "tag missing: {json}");
        let back: ModelEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }

    #[test]
    fn thinking_block_completed_roundtrips() -> serde_json::Result<()> {
        let evt = ModelEvent::ThinkingBlockCompleted {
            block_index: 0,
            text: "I should grep.".into(),
            provider_opaque: Some(serde_json::json!({"signature":"abc"})),
        };
        let json = serde_json::to_string(&evt)?;
        let back: ModelEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-protocol
```

Expected: both tests fail (variants missing).

- [ ] **Step 3: Add the variants**

In `crates/cogito-protocol/src/gateway.rs`, inside the `ModelEvent` enum,
immediately before `MessageCompleted { ... }`:

```rust
    /// One streaming reasoning chunk inside an in-flight thinking block.
    /// Forwarded to the broadcast channel for live UI; persistence
    /// waits for `ThinkingBlockCompleted`. See ADR-0019 §3.
    ThinkingDelta {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Partial reasoning text for this delta.
        chunk: String,
    },
    /// A thinking block has been sealed by the provider; carries the
    /// full accumulated text plus any provider-opaque payload (signature
    /// for Anthropic, encrypted_content + item_id for OpenAI Responses,
    /// `None` for OpenAI-compat). H06 calls
    /// `recorder.on_thinking_block_complete(...)`.
    ThinkingBlockCompleted {
        /// Zero-based index of the block within the response.
        block_index: u32,
        /// Full accumulated reasoning text for the completed block.
        text: String,
        /// Provider-opaque round-trip payload (see ADR-0019 §1).
        provider_opaque: Option<serde_json::Value>,
    },
```

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-protocol
```

Expected: tests pass; existing gateway tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/gateway.rs
git commit -m "feat(protocol): add ModelEvent::ThinkingDelta + ThinkingBlockCompleted

Symmetric to TextDelta / TextBlockCompleted per ADR-0019 §3.
H06 will route ThinkingDelta to the broadcast channel and
ThinkingBlockCompleted into the persisted event log."
```

---

### Task 4: `StreamEvent::ThinkingDelta`

**Files:**
- Modify: `crates/cogito-protocol/src/stream.rs`

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/cogito-protocol/src/stream.rs`:

```rust
#[cfg(test)]
mod thinking_stream_tests {
    use super::*;

    #[test]
    fn thinking_delta_roundtrips() -> serde_json::Result<()> {
        let evt = StreamEvent::ThinkingDelta {
            chunk: "thinking...".into(),
        };
        let json = serde_json::to_string(&evt)?;
        assert!(json.contains(r#""kind":"thinking_delta""#), "tag missing: {json}");
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(evt, back);
        Ok(())
    }
}
```

- [ ] **Step 2: Run the failing test**

```bash
make test CRATE=cogito-protocol
```

Expected: test fails.

- [ ] **Step 3: Add the variant**

In `crates/cogito-protocol/src/stream.rs`, inside `StreamEvent`,
immediately after `TextDelta`:

```rust
    /// Per-chunk reasoning delta from the model stream. Not persisted
    /// as-is; the store writer batches into `ThinkingBlockRecorded`
    /// at the wire-protocol block-completion boundary. See ADR-0019 §3.
    ThinkingDelta {
        /// The reasoning chunk emitted by the model.
        chunk: String,
    },
```

- [ ] **Step 4: Run the test**

```bash
make test CRATE=cogito-protocol
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/stream.rs
git commit -m "feat(protocol): add StreamEvent::ThinkingDelta

Live broadcast channel for UI 'Thinking…' surfaces per ADR-0019 §3.
Not persisted; renderers fold/collapse at their discretion."
```

---

### Task 5: Regenerate JSON schema + extend fixture

**Files:**
- Regenerate: `docs/schemas/conversation-event-v1.json`
- Modify: `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`

- [ ] **Step 1: Locate the schema-gen invocation**

The generator lives at `tools/cogito-gen-schema`. Inspect its CLI:

```bash
cargo run -p cogito-gen-schema -- --help
```

Capture the actual flag for output path (likely `--out` or positional).
The remaining steps assume `cargo run -p cogito-gen-schema -- docs/schemas/conversation-event-v1.json`;
adjust to match what `--help` printed.

- [ ] **Step 2: Regenerate the schema**

```bash
cargo run -p cogito-gen-schema -- docs/schemas/conversation-event-v1.json
```

Expected: the file is updated; `git diff docs/schemas/conversation-event-v1.json`
shows a new `ThinkingBlockRecorded` definition referenced from the
`EventPayload` `oneOf`, plus a new `Thinking` variant inside `ContentBlock`.

- [ ] **Step 3: Append a fixture row**

Append to `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
(use a fresh event_id `000000000C…` and seq one greater than the current
maximum, currently 10 → use seq `11`; ts 2026-05-18T10:00:01.100Z):

```json
{"schema_version":1,"event_id":"000000000C0000000000000000","session_id":"01J9C0R0K0SESS10NSESS10NSE","turn_id":"01J9C0R0K0TRN0TRN0TRN0TRN0","seq":11,"ts":"2026-05-18T10:00:01.100Z","type":"thinking_block_recorded","data":{"text":"I should grep for the symbol.","provider_opaque":{"signature":"abc123"}}}
```

- [ ] **Step 4: Run CI schema drift gate + workspace tests**

```bash
make ci
```

Expected: green. If the schema drift gate fires, the regenerated
artifact and the source enum agree; if not, re-run Step 2 and recommit.

- [ ] **Step 5: Commit**

```bash
git add docs/schemas/conversation-event-v1.json \
  crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl
git commit -m "build(schema): regenerate v1 schema; extend fixture for thinking_block_recorded

Picks up ContentBlock::Thinking and EventPayload::ThinkingBlockRecorded
additions from Tasks 1-3. SCHEMA_VERSION unchanged (still 1) per
ADR-0019 §2 and ADR-0007 additive precedent. Filename unchanged."
```

---

## Phase 2 — Brain integration

H02/H04/H06 each gain exactly one additional code path that mirrors the
existing text-block path.

### Task 6: `StepRecorder::on_thinking_delta` + `on_thinking_block_complete`

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `tests` module at the bottom of `step_recorder.rs`
(if no `tests` module exists yet, scan for one — file is 457 lines, tests
live near the bottom). Add:

```rust
#[tokio::test]
async fn thinking_block_flush_persists_thinking_block_recorded()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store: Arc<dyn ConversationStore> =
        Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (tx, mut rx) = broadcast::channel(64);
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let mut recorder =
        StepRecorder::new(Arc::clone(&store), tx, session_id, 0);

    recorder.record_session_started(SessionMeta {
        cogito_version: "0.1.0".into(),
        ..Default::default()
    }).await?;

    recorder.on_thinking_delta(turn_id, "I should ".into());
    recorder.on_thinking_delta(turn_id, "grep.".into());
    let id = recorder
        .on_thinking_block_complete(Some(serde_json::json!({"signature":"abc"})))
        .await?;
    assert!(id.is_some(), "expected an EventId from flush");

    // Two ThinkingDelta StreamEvents broadcast then nothing more.
    match rx.try_recv()? {
        StreamEvent::ThinkingDelta { chunk } => assert_eq!(chunk, "I should "),
        other => panic!("unexpected stream event: {other:?}"),
    }
    match rx.try_recv()? {
        StreamEvent::ThinkingDelta { chunk } => assert_eq!(chunk, "grep."),
        other => panic!("unexpected stream event: {other:?}"),
    }

    // Persisted shape: read the JSONL file and confirm the payload type.
    let session_file = std::fs::read_dir(tmp.path())?
        .next()
        .ok_or("no session file")?
        .map_err(|e| format!("{e}"))?
        .path();
    let text = tokio::fs::read_to_string(session_file).await?;
    assert!(
        text.contains("thinking_block_recorded"),
        "expected thinking_block_recorded line, got: {text}"
    );
    assert!(
        text.contains(r#""provider_opaque":{"signature":"abc"}"#),
        "expected provider_opaque payload preserved, got: {text}"
    );
    Ok(())
}

#[tokio::test]
async fn thinking_block_complete_with_no_buffered_deltas_is_noop()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store: Arc<dyn ConversationStore> =
        Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (tx, _rx) = broadcast::channel(64);
    let mut recorder =
        StepRecorder::new(Arc::clone(&store), tx, SessionId::new(), 0);
    let id = recorder.on_thinking_block_complete(None).await?;
    assert!(id.is_none(), "no buffered deltas → no event written");
    Ok(())
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-core
```

Expected: tests fail with `no method named on_thinking_delta`.

- [ ] **Step 3: Add the buffer struct + methods**

In `crates/cogito-core/src/harness/step_recorder.rs`:

(a) Add a sibling buffer struct beside `TextBlockBuf` (around line 47):

```rust
/// In-flight thinking block accumulator. Filled by `on_thinking_delta`
/// and drained by `on_thinking_block_complete` into a single
/// `ThinkingBlockRecorded` event. `provider_opaque` is supplied at
/// flush time because adapters pre-aggregate signature/encrypted blobs
/// and only know the final payload after the wire `content_block_stop`.
struct ThinkingBlockBuf {
    turn_id: TurnId,
    text: String,
}
```

(b) Add a field on `StepRecorder` (next to `current_text_block`):

```rust
    current_thinking_block: Option<ThinkingBlockBuf>,
```

Initialize it to `None` in `StepRecorder::new`.

(c) Add the two methods (paste after `on_text_block_complete`, around line 128):

```rust
    /// Buffer a streaming reasoning chunk and broadcast it live as
    /// [`StreamEvent::ThinkingDelta`]. Does NOT persist — call
    /// [`StepRecorder::on_thinking_block_complete`] when the wire
    /// protocol signals the block is finished.
    pub fn on_thinking_delta(&mut self, turn_id: TurnId, chunk: String) {
        let buf = self
            .current_thinking_block
            .get_or_insert_with(|| ThinkingBlockBuf {
                turn_id,
                text: String::new(),
            });
        buf.text.push_str(&chunk);
        let _ = self.events_tx.send(StreamEvent::ThinkingDelta { chunk });
    }

    /// Persist the accumulated thinking block as one
    /// `ThinkingBlockRecorded` event. `provider_opaque` is taken from
    /// the gateway's `ThinkingBlockCompleted` event (signature for
    /// Anthropic, encrypted_content for OpenAI Responses, None for
    /// OpenAI-compat). No-op when no `on_thinking_delta` calls have
    /// arrived since the last flush.
    pub async fn on_thinking_block_complete(
        &mut self,
        provider_opaque: Option<serde_json::Value>,
    ) -> Result<Option<EventId>, StoreError> {
        let Some(buf) = self.current_thinking_block.take() else {
            return Ok(None);
        };
        let event_id = self
            .append(
                Some(buf.turn_id),
                EventPayload::ThinkingBlockRecorded {
                    text: buf.text,
                    provider_opaque,
                },
            )
            .await?;
        Ok(Some(event_id))
    }
```

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-core
```

Expected: new tests pass; existing tests stay green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs
git commit -m "feat(harness): H02 buffers thinking blocks + flushes ThinkingBlockRecorded

Symmetric to the text-block buffer. provider_opaque is taken at flush
time because adapters only know the final signature/encrypted payload
after the wire content_block_stop. Per ADR-0019 §2-3."
```

---

### Task 7: H06 routes `ModelEvent::Thinking*`

**Files:**
- Modify: `crates/cogito-core/src/harness/stream_demux.rs`

- [ ] **Step 1: Write the failing test**

In the existing `#[cfg(test)] mod tests { ... }` block at the bottom of
`stream_demux.rs`, append:

```rust
#[tokio::test]
async fn demux_routes_thinking_delta_and_completed()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store: Arc<dyn ConversationStore> =
        Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (tx, _rx) = broadcast::channel(64);
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let mut recorder = StepRecorder::new(Arc::clone(&store), tx, session_id, 0);
    recorder.record_session_started(cogito_protocol::session::SessionMeta {
        cogito_version: "0.1.0".into(),
        ..Default::default()
    }).await?;

    let events = stream::iter(vec![
        Ok(ModelEvent::ThinkingDelta { block_index: 0, chunk: "I should ".into() }),
        Ok(ModelEvent::ThinkingDelta { block_index: 0, chunk: "grep.".into() }),
        Ok(ModelEvent::ThinkingBlockCompleted {
            block_index: 0,
            text: "I should grep.".into(),
            provider_opaque: Some(serde_json::json!({"signature":"sig"})),
        }),
        Ok(ModelEvent::TextBlockCompleted { block_index: 1, text: "ok".into() }),
        Ok(ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 1, output_tokens: 1 },
        }),
    ]);

    let output = demux(events, &mut recorder, turn_id).await?;

    // ModelOutput.content carries the Thinking block at index 0, Text at 1.
    assert_eq!(output.content.len(), 2);
    match &output.content[0] {
        cogito_protocol::content::ContentBlock::Thinking { text, provider_opaque } => {
            assert_eq!(text, "I should grep.");
            assert_eq!(provider_opaque.as_ref().and_then(|v| v.get("signature")),
                       Some(&serde_json::json!("sig")));
        }
        other => panic!("expected Thinking at idx 0, got {other:?}"),
    }
    match &output.content[1] {
        cogito_protocol::content::ContentBlock::Text { text } => assert_eq!(text, "ok"),
        other => panic!("expected Text at idx 1, got {other:?}"),
    }

    // Persisted: a thinking_block_recorded event preceded the assistant_message_appended.
    let session_file = std::fs::read_dir(tmp.path())?
        .next().ok_or("no session file")?
        .map_err(|e| format!("{e}"))?
        .path();
    let log = tokio::fs::read_to_string(session_file).await?;
    let think_pos = log.find("thinking_block_recorded").expect("thinking event missing");
    let text_pos = log.find("assistant_message_appended").expect("text event missing");
    assert!(think_pos < text_pos, "thinking event must precede text event by seq");
    Ok(())
}
```

- [ ] **Step 2: Run the failing test**

```bash
make test CRATE=cogito-core
```

Expected: fails with "expected Thinking at idx 0, got Text" or similar
(demux falls through to `_ => {}`).

- [ ] **Step 3: Add the routing arms**

In `crates/cogito-core/src/harness/stream_demux.rs`, inside the
`match evt? { ... }` block, between `TextBlockCompleted` and
`ToolUseCompleted`:

```rust
            ModelEvent::ThinkingDelta {
                block_index: _,
                chunk,
            } => {
                recorder.on_thinking_delta(turn_id, chunk);
            }
            ModelEvent::ThinkingBlockCompleted {
                block_index,
                text,
                provider_opaque,
            } => {
                recorder
                    .on_thinking_block_complete(provider_opaque.clone())
                    .await
                    .map_err(|e| ModelError::Provider {
                        status: 0,
                        message: format!("recorder thinking flush: {e}"),
                    })?;
                content.push((
                    block_index,
                    ContentBlock::Thinking { text, provider_opaque },
                ));
            }
```

- [ ] **Step 4: Run the test**

```bash
make test CRATE=cogito-core
```

Expected: pass; all existing tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/stream_demux.rs
git commit -m "feat(harness): H06 routes ModelEvent::Thinking* through StepRecorder

ThinkingDelta -> StepRecorder buffer + broadcast.
ThinkingBlockCompleted -> flush as ThinkingBlockRecorded and append
the corresponding ContentBlock::Thinking to ModelOutput. Per ADR-0019 §3."
```

---

### Task 8: H04 projects `ThinkingBlockRecorded` → `ContentBlock::Thinking`

**Files:**
- Modify: `crates/cogito-core/src/harness/prompt.rs`

- [ ] **Step 1: Write the failing test**

Append a `#[cfg(test)] mod tests` block to `prompt.rs` (current file has
none — confirm by reading the bottom of the file before writing):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cogito_protocol::event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
    use cogito_protocol::gateway::Message;
    use cogito_protocol::ids::{EventId, SessionId, TurnId};
    use cogito_protocol::strategy::HarnessStrategy;

    fn evt(seq: u64, payload: EventPayload, turn_id: Option<TurnId>) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id,
            seq,
            ts: Utc::now(),
            payload,
        }
    }

    #[test]
    fn project_history_emits_thinking_before_text_within_assistant_message() {
        let turn_id = TurnId::new();
        let history = vec![
            evt(
                0,
                EventPayload::TurnStarted {
                    user_input: vec![ContentBlock::Text { text: "go".into() }],
                },
                Some(turn_id),
            ),
            evt(
                1,
                EventPayload::ThinkingBlockRecorded {
                    text: "I should grep.".into(),
                    provider_opaque: Some(serde_json::json!({"signature":"sig"})),
                },
                Some(turn_id),
            ),
            evt(
                2,
                EventPayload::AssistantMessageAppended { text: "OK.".into() },
                Some(turn_id),
            ),
            evt(
                3,
                EventPayload::ToolUseRecorded {
                    call_id: "c1".into(),
                    tool_name: "grep".into(),
                    args: serde_json::json!({"pattern":"foo"}),
                },
                Some(turn_id),
            ),
        ];

        let messages = project_history(&history);
        assert_eq!(messages.len(), 2, "1 user + 1 assistant message");
        match &messages[1] {
            Message::Assistant { content } => {
                assert_eq!(content.len(), 3);
                assert!(
                    matches!(content[0], ContentBlock::Thinking { .. }),
                    "Thinking must be at index 0 (precedes Text/ToolUse)"
                );
                assert!(matches!(content[1], ContentBlock::Text { .. }));
                assert!(matches!(content[2], ContentBlock::ToolUse { .. }));
            }
            other => panic!("expected Assistant message, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run the failing test**

```bash
make test CRATE=cogito-core
```

Expected: fails — `Thinking` block is missing from projected assistant
content (current `_ => {}` arm drops it).

- [ ] **Step 3: Add the match arm**

In `crates/cogito-core/src/harness/prompt.rs`, inside `project_history`'s
`match &evt.payload` (before the catch-all `_ => {}` arm):

```rust
            EventPayload::ThinkingBlockRecorded {
                text,
                provider_opaque,
            } => {
                current_assistant
                    .get_or_insert_with(Vec::new)
                    .push(ContentBlock::Thinking {
                        text: text.clone(),
                        provider_opaque: provider_opaque.clone(),
                    });
            }
```

- [ ] **Step 4: Run the test**

```bash
make test CRATE=cogito-core
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/prompt.rs
git commit -m "feat(harness): H04 projects ThinkingBlockRecorded → ContentBlock::Thinking

Same seq-ordered walk used today for text + tool_use interleaving.
provider_opaque rides verbatim from the event log back into the
outgoing Message::Assistant.content array. Per ADR-0019 §4."
```

---

## Phase 3 — Anthropic adapter

Anthropic's wire format already represents thinking as a first-class
content block; the work is straight mapping.

### Task 9: Anthropic wire types

**Files:**
- Modify: `crates/cogito-model/src/anthropic/wire.rs`

- [ ] **Step 1: Read the existing wire shape**

Open `wire.rs` and identify the `ContentBlockDelta` / `ContentBlockStart`
enums. Anthropic's SSE protocol emits:

- `content_block_start { type: "thinking" }`
- `content_block_delta { type: "thinking_delta", thinking: "<chunk>" }`
- `content_block_delta { type: "signature_delta", signature: "<blob>" }`
- `content_block_start { type: "redacted_thinking", data: "<blob>" }`

Match these onto the existing enums; **don't invent new top-level types**.

- [ ] **Step 2: Write the failing roundtrip tests**

Append wire-deserialization tests for each new variant. Example shape:

```rust
#[test]
fn deserializes_thinking_delta() {
    let json = r#"{"type":"thinking_delta","thinking":"I should grep."}"#;
    let d: ContentBlockDelta = serde_json::from_str(json).unwrap();
    match d {
        ContentBlockDelta::ThinkingDelta { thinking } => assert_eq!(thinking, "I should grep."),
        other => panic!("expected ThinkingDelta, got {other:?}"),
    }
}

#[test]
fn deserializes_signature_delta() {
    let json = r#"{"type":"signature_delta","signature":"sig_xyz"}"#;
    let d: ContentBlockDelta = serde_json::from_str(json).unwrap();
    match d {
        ContentBlockDelta::SignatureDelta { signature } => assert_eq!(signature, "sig_xyz"),
        other => panic!("expected SignatureDelta, got {other:?}"),
    }
}

#[test]
fn deserializes_thinking_block_start() {
    let json = r#"{"type":"thinking"}"#;
    let s: ContentBlockStart = serde_json::from_str(json).unwrap();
    assert!(matches!(s, ContentBlockStart::Thinking { .. }));
}

#[test]
fn deserializes_redacted_thinking_block_start() {
    let json = r#"{"type":"redacted_thinking","data":"enc_blob"}"#;
    let s: ContentBlockStart = serde_json::from_str(json).unwrap();
    match s {
        ContentBlockStart::RedactedThinking { data } => assert_eq!(data, "enc_blob"),
        other => panic!("expected RedactedThinking, got {other:?}"),
    }
}
```

- [ ] **Step 3: Run failing tests**

```bash
make test CRATE=cogito-model
```

Expected: all four fail (variants missing).

- [ ] **Step 4: Add the variants to the wire enums**

Inside `ContentBlockStart` (or whatever the start-event enum is called):

```rust
    /// Anthropic thinking block start (extended-thinking mode).
    /// Body fields arrive in subsequent `thinking_delta` and
    /// `signature_delta` deltas.
    Thinking {},
    /// Anthropic safety-filtered reasoning block. Carries an opaque
    /// `data` blob; no further deltas follow.
    RedactedThinking { data: String },
```

Inside `ContentBlockDelta`:

```rust
    /// One streamed chunk of the in-flight thinking block.
    ThinkingDelta { thinking: String },
    /// Signature for the in-flight thinking block. Arrives once,
    /// immediately before the corresponding `content_block_stop`.
    SignatureDelta { signature: String },
```

- [ ] **Step 5: Run the tests**

```bash
make test CRATE=cogito-model
```

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-model/src/anthropic/wire.rs
git commit -m "feat(model/anthropic): wire types for thinking + signature + redacted_thinking

Mirrors Anthropic Messages API SSE shape verbatim. Used by decode.rs
to produce ModelEvent::Thinking* and by encode.rs to round-trip
ContentBlock::Thinking back to the wire."
```

---

### Task 10: Anthropic decode — emit `ModelEvent::Thinking*`

**Files:**
- Modify: `crates/cogito-model/src/anthropic/decode.rs`

- [ ] **Step 1: Write a replay-style failing test**

Locate the existing replay test pattern in
`crates/cogito-model/tests/anthropic_replay.rs`. Add a new fixture function
or extend an existing one with a thinking-block scenario:

```rust
#[tokio::test]
async fn replays_thinking_block_with_signature() {
    // Synthetic SSE: thinking block with two deltas + signature, then
    // a text block + message_delta. Format follows Anthropic Messages
    // streaming SSE.
    let sse = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-4-7\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"I should \"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"grep.\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_xyz\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let events = drive_decoder(sse).await; // helper that exists in this test file or stream them manually
    let mut iter = events.into_iter();
    assert!(matches!(iter.next(), Some(Ok(ModelEvent::ThinkingDelta { block_index: 0, chunk })) if chunk == "I should "));
    assert!(matches!(iter.next(), Some(Ok(ModelEvent::ThinkingDelta { block_index: 0, chunk })) if chunk == "grep."));
    match iter.next() {
        Some(Ok(ModelEvent::ThinkingBlockCompleted { block_index, text, provider_opaque })) => {
            assert_eq!(block_index, 0);
            assert_eq!(text, "I should grep.");
            assert_eq!(
                provider_opaque,
                Some(serde_json::json!({"signature": "sig_xyz"}))
            );
        }
        other => panic!("expected ThinkingBlockCompleted, got {other:?}"),
    }
}
```

If `drive_decoder` does not already exist in the test file, write it as
a thin SSE-bytes-to-`Vec<Result<ModelEvent, ModelError>>` helper using the
same code path `cogito-model::anthropic` uses in production.

- [ ] **Step 2: Run the failing test**

```bash
make test CRATE=cogito-model
```

Expected: test fails — current decoder drops thinking events.

- [ ] **Step 3: Extend the decoder**

In `crates/cogito-model/src/anthropic/decode.rs`, locate the per-block
state machine (look for where `text_delta` is currently handled). Mirror
the structure for thinking:

(a) When `content_block_start { type: "thinking" }` arrives, mark the
    current block as a thinking block (state: `BlockKind::Thinking`),
    initialize an empty text accumulator and a `None` signature.

(b) When `content_block_delta { type: "thinking_delta" }` arrives for a
    thinking block, append to the accumulator and emit
    `ModelEvent::ThinkingDelta { block_index, chunk }`.

(c) When `content_block_delta { type: "signature_delta" }` arrives,
    capture the signature into the in-flight thinking block's state.
    Do NOT emit a delta — signature is part of the completion payload.

(d) When `content_block_stop` arrives for a thinking block, emit
    `ModelEvent::ThinkingBlockCompleted { block_index, text, provider_opaque }`
    where `provider_opaque = Some(json!({"signature": <captured>}))` if a
    signature was seen; `Some(json!({"redacted_data": <data>}))` if the
    block was `RedactedThinking`; `None` only as a defensive default for
    malformed streams.

(e) When `content_block_start { type: "redacted_thinking", data }` arrives,
    emit a `ThinkingBlockCompleted { text: String::new(), provider_opaque:
    Some(json!({"redacted_data": data})) }` immediately (no deltas
    follow per Anthropic's protocol).

- [ ] **Step 4: Run the test**

```bash
make test CRATE=cogito-model
```

Expected: pass. Also write a second test case for `redacted_thinking` if
not already in the fixture, and verify it passes.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/anthropic/decode.rs crates/cogito-model/tests/anthropic_replay.rs
git commit -m "feat(model/anthropic): decode thinking_delta + signature_delta + redacted_thinking

Per-block state machine recognises Thinking kind and packs the
signature (or redacted data) into ModelEvent::ThinkingBlockCompleted.
provider_opaque is Some({signature}) or Some({redacted_data}).
Per ADR-0019 §5.1."
```

---

### Task 11: Anthropic encode — `ContentBlock::Thinking` → wire

**Files:**
- Modify: `crates/cogito-model/src/anthropic/encode.rs`

- [ ] **Step 1: Write the failing test**

In an existing encode test file (or appending to `encode.rs`'s `#[cfg(test)]`
module):

```rust
#[test]
fn encodes_thinking_block_with_signature() {
    let msg = cogito_protocol::gateway::Message::Assistant {
        content: vec![
            cogito_protocol::content::ContentBlock::Thinking {
                text: "I should grep.".into(),
                provider_opaque: Some(serde_json::json!({"signature": "sig_xyz"})),
            },
            cogito_protocol::content::ContentBlock::Text { text: "OK.".into() },
        ],
    };
    let wire = encode_assistant_message(&msg).unwrap(); // exact fn name to be confirmed in file
    let json = serde_json::to_value(&wire).unwrap();

    let content = json.get("content").unwrap().as_array().unwrap();
    assert_eq!(content[0].get("type").unwrap(), "thinking");
    assert_eq!(content[0].get("thinking").unwrap(), "I should grep.");
    assert_eq!(content[0].get("signature").unwrap(), "sig_xyz");
    assert_eq!(content[1].get("type").unwrap(), "text");
}

#[test]
fn encodes_thinking_block_redacted() {
    let msg = cogito_protocol::gateway::Message::Assistant {
        content: vec![
            cogito_protocol::content::ContentBlock::Thinking {
                text: String::new(),
                provider_opaque: Some(serde_json::json!({"redacted_data": "enc_blob"})),
            },
        ],
    };
    let wire = encode_assistant_message(&msg).unwrap();
    let json = serde_json::to_value(&wire).unwrap();
    let content = json.get("content").unwrap().as_array().unwrap();
    assert_eq!(content[0].get("type").unwrap(), "redacted_thinking");
    assert_eq!(content[0].get("data").unwrap(), "enc_blob");
}
```

Replace `encode_assistant_message` with the actual function/method name
used by the current encoder when read — keep behavior identical, only
add the Thinking arm.

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-model
```

Expected: fail with `ContentBlock::Thinking` falling through to a default
arm that produces invalid wire output, or panicking.

- [ ] **Step 3: Add the encode arm**

In the match over `ContentBlock` variants inside `encode.rs`, add:

```rust
ContentBlock::Thinking { text, provider_opaque } => {
    // Disambiguate redacted vs. plain thinking via provider_opaque
    // keys. Redacted thinking has no `signature`, instead carries an
    // opaque `redacted_data` blob. See ADR-0019 §5.1.
    let opaque = provider_opaque.as_ref();
    if let Some(data) = opaque.and_then(|o| o.get("redacted_data")).and_then(|v| v.as_str()) {
        wire_blocks.push(json!({
            "type": "redacted_thinking",
            "data": data,
        }));
    } else {
        let mut block = json!({
            "type": "thinking",
            "thinking": text,
        });
        if let Some(sig) = opaque.and_then(|o| o.get("signature")).and_then(|v| v.as_str()) {
            block["signature"] = json!(sig);
        }
        wire_blocks.push(block);
    }
}
```

Adapt the surrounding shape (`wire_blocks`, return type) to match the
existing encoder API.

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-model
```

Expected: both tests pass; existing encode tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/anthropic/encode.rs
git commit -m "feat(model/anthropic): encode ContentBlock::Thinking to wire thinking block

Plain thinking → {type:thinking, thinking, signature}.
Redacted thinking → {type:redacted_thinking, data}.
Discriminated by provider_opaque keys produced by decode.rs.
Per ADR-0019 §5.1."
```

---

## Phase 4 — OpenAI-compat adapter

This is the largest behavior change: introduce a stateful `<think>` tag
parser and an alternate `reasoning_content` field reader.

### Task 12: OpenAI-compat wire — add `reasoning_content` on delta

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/wire.rs`

- [ ] **Step 1: Write the failing test**

Append:

```rust
#[test]
fn deserializes_delta_with_reasoning_content() {
    let json = r#"{"reasoning_content":"I should grep."}"#;
    let d: Delta = serde_json::from_str(json).unwrap(); // replace `Delta` with the actual type name
    assert_eq!(d.reasoning_content.as_deref(), Some("I should grep."));
    assert!(d.content.is_none());
}

#[test]
fn deserializes_delta_without_reasoning_content_stays_compatible() {
    let json = r#"{"content":"hi"}"#;
    let d: Delta = serde_json::from_str(json).unwrap();
    assert_eq!(d.content.as_deref(), Some("hi"));
    assert!(d.reasoning_content.is_none());
}
```

- [ ] **Step 2: Run failing test**

```bash
make test CRATE=cogito-model
```

Expected: fail with `no field reasoning_content`.

- [ ] **Step 3: Add the field**

In the `Delta` struct (or equivalent) within `wire.rs`:

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
```

- [ ] **Step 4: Run tests**

```bash
make test CRATE=cogito-model
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/openai_compat/wire.rs
git commit -m "feat(model/openai_compat): wire delta.reasoning_content (optional)

Used by DeepSeek official API and vLLM --enable-reasoning to expose
reasoning as a separate field from delta.content. Per ADR-0019 §5.3.b."
```

---

### Task 13: OpenAI-compat decode — `reasoning_content` path

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/decode.rs`

- [ ] **Step 1: Write the failing test**

In `crates/cogito-model/tests/openai_compat_replay.rs`:

```rust
#[tokio::test]
async fn replays_separate_reasoning_content_field() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"I should \"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"grep.\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"OK.\"}}]}\n\n\
data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";
    let events = drive_decoder(sse).await; // helper local to this test file
    // Expect: ThinkingDelta x 2, ThinkingBlockCompleted (transition to content),
    // TextDelta, TextBlockCompleted, MessageCompleted.
    let kinds: Vec<&'static str> = events.iter().map(|r| match r.as_ref().unwrap() {
        ModelEvent::ThinkingDelta { .. } => "td",
        ModelEvent::ThinkingBlockCompleted { .. } => "tc",
        ModelEvent::TextDelta { .. } => "xd",
        ModelEvent::TextBlockCompleted { .. } => "xc",
        ModelEvent::MessageCompleted { .. } => "mc",
        _ => "??",
    }).collect();
    assert_eq!(kinds, vec!["td", "td", "tc", "xd", "xc", "mc"]);

    // The ThinkingBlockCompleted carries provider_opaque == None.
    let completed = events.iter().find_map(|r| match r.as_ref().unwrap() {
        ModelEvent::ThinkingBlockCompleted { text, provider_opaque, .. } => {
            Some((text.clone(), provider_opaque.clone()))
        }
        _ => None,
    }).unwrap();
    assert_eq!(completed.0, "I should grep.");
    assert_eq!(completed.1, None);
}
```

- [ ] **Step 2: Run the failing test**

```bash
make test CRATE=cogito-model
```

Expected: fail.

- [ ] **Step 3: Implement the path**

In `decode.rs`, identify where `delta.content` is processed. Mirror the
shape for `delta.reasoning_content`:

```rust
// Pseudocode — adapt to the actual decoder structure.
if let Some(reasoning_chunk) = delta.reasoning_content.as_deref() {
    state.thinking_text.push_str(reasoning_chunk);
    emit(ModelEvent::ThinkingDelta {
        block_index: state.thinking_block_index,
        chunk: reasoning_chunk.to_string(),
    });
}

if let Some(text_chunk) = delta.content.as_deref() {
    // Transition out of thinking: if we have accumulated thinking text
    // and this is the first content chunk, seal the thinking block.
    if !state.thinking_text.is_empty() && !state.thinking_sealed {
        emit(ModelEvent::ThinkingBlockCompleted {
            block_index: state.thinking_block_index,
            text: std::mem::take(&mut state.thinking_text),
            provider_opaque: None,
        });
        state.thinking_sealed = true;
        state.text_block_index = state.thinking_block_index + 1;
    }
    // ... existing text emission logic ...
}
```

On stream end, if `!state.thinking_text.is_empty() && !state.thinking_sealed`,
emit the trailing `ThinkingBlockCompleted` before `MessageCompleted`.

- [ ] **Step 4: Run the test**

```bash
make test CRATE=cogito-model
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/openai_compat/decode.rs crates/cogito-model/tests/openai_compat_replay.rs
git commit -m "feat(model/openai_compat): decode delta.reasoning_content path

reasoning_content chunks emit ThinkingDelta; on transition to
delta.content (or at stream end) emit ThinkingBlockCompleted with
provider_opaque: None. Per ADR-0019 §5.3.b."
```

---

### Task 14: OpenAI-compat decode — `<think>` tag two-state parser

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/decode.rs`

This is the most subtle task in the plan. Read ADR-0019 §5.3.a in full
before starting — the parser must handle `<think>` and `</think>` straddling
chunk boundaries.

- [ ] **Step 1: Write three failing tests covering split-tag scenarios**

```rust
#[tokio::test]
async fn replays_inline_think_tag_whole_chunk() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"<think>I should grep.</think>OK.\"}}]}\n\n\
data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";
    let events = drive_decoder(sse).await;
    let kinds: Vec<&'static str> = events.iter().map(|r| match r.as_ref().unwrap() {
        ModelEvent::ThinkingDelta { .. } => "td",
        ModelEvent::ThinkingBlockCompleted { .. } => "tc",
        ModelEvent::TextDelta { .. } => "xd",
        ModelEvent::TextBlockCompleted { .. } => "xc",
        ModelEvent::MessageCompleted { .. } => "mc",
        _ => "??",
    }).collect();
    assert_eq!(kinds, vec!["td", "tc", "xd", "xc", "mc"]);
}

#[tokio::test]
async fn replays_inline_think_tag_split_across_chunks() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"<thi\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"nk>I should grep.</thi\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"nk>OK.\"}}]}\n\n\
data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";
    let events = drive_decoder(sse).await;
    let completed = events.iter().find_map(|r| match r.as_ref().unwrap() {
        ModelEvent::ThinkingBlockCompleted { text, .. } => Some(text.clone()),
        _ => None,
    }).unwrap();
    assert_eq!(completed, "I should grep.");
    let text = events.iter().find_map(|r| match r.as_ref().unwrap() {
        ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
        _ => None,
    }).unwrap();
    assert_eq!(text, "OK.");
}

#[tokio::test]
async fn replays_unclosed_think_tag_on_stream_end() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"<think>I never closed\"}}]}\n\n\
data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";
    let events = drive_decoder(sse).await;
    let completed = events.iter().find_map(|r| match r.as_ref().unwrap() {
        ModelEvent::ThinkingBlockCompleted { text, .. } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(completed, Some("I never closed".to_string()),
        "unclosed <think> on stream end must still seal as a thinking block");
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-model
```

Expected: all three fail.

- [ ] **Step 3: Implement the two-state parser**

In `decode.rs`, wrap the existing `delta.content` handling in a parser
state machine. Add a struct (file-private) and a method that consumes a
chunk and emits zero or more `ModelEvent`s:

```rust
#[derive(Debug, Default)]
struct ThinkTagParser {
    state: ThinkTagState,
    pending: String,
    accumulated_text: String,
    accumulated_thinking: String,
    thinking_block_index: u32,
    text_block_index: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ThinkTagState {
    #[default]
    Outside,
    Inside,
}

const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

impl ThinkTagParser {
    fn feed(&mut self, chunk: &str, out: &mut Vec<ModelEvent>) {
        self.pending.push_str(chunk);
        loop {
            match self.state {
                ThinkTagState::Outside => {
                    if let Some(pos) = self.pending.find(OPEN_TAG) {
                        let before = self.pending.drain(..pos).collect::<String>();
                        self.pending.drain(..OPEN_TAG.len()); // consume tag
                        if !before.is_empty() {
                            self.accumulated_text.push_str(&before);
                            out.push(ModelEvent::TextDelta {
                                block_index: self.text_block_index,
                                chunk: before,
                            });
                        }
                        self.state = ThinkTagState::Inside;
                    } else {
                        // Keep last 6 chars in pending in case <think> is
                        // split across the next chunk (longest prefix - 1).
                        let safe = self.pending.len().saturating_sub(OPEN_TAG.len() - 1);
                        if safe > 0 {
                            let emitted: String = self.pending.drain(..safe).collect();
                            self.accumulated_text.push_str(&emitted);
                            out.push(ModelEvent::TextDelta {
                                block_index: self.text_block_index,
                                chunk: emitted,
                            });
                        }
                        break;
                    }
                }
                ThinkTagState::Inside => {
                    if let Some(pos) = self.pending.find(CLOSE_TAG) {
                        let before = self.pending.drain(..pos).collect::<String>();
                        self.pending.drain(..CLOSE_TAG.len());
                        if !before.is_empty() {
                            self.accumulated_thinking.push_str(&before);
                            out.push(ModelEvent::ThinkingDelta {
                                block_index: self.thinking_block_index,
                                chunk: before,
                            });
                        }
                        out.push(ModelEvent::ThinkingBlockCompleted {
                            block_index: self.thinking_block_index,
                            text: std::mem::take(&mut self.accumulated_thinking),
                            provider_opaque: None,
                        });
                        self.text_block_index = self.thinking_block_index + 1;
                        self.state = ThinkTagState::Outside;
                    } else {
                        let safe = self.pending.len().saturating_sub(CLOSE_TAG.len() - 1);
                        if safe > 0 {
                            let emitted: String = self.pending.drain(..safe).collect();
                            self.accumulated_thinking.push_str(&emitted);
                            out.push(ModelEvent::ThinkingDelta {
                                block_index: self.thinking_block_index,
                                chunk: emitted,
                            });
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Drain the pending buffer at stream end. Treats any unclosed
    /// `<think>` as best-effort (emit ThinkingBlockCompleted with what
    /// was accumulated); pending text outside a tag is flushed as the
    /// last TextDelta.
    fn finish(&mut self, out: &mut Vec<ModelEvent>) {
        match self.state {
            ThinkTagState::Outside => {
                if !self.pending.is_empty() {
                    let chunk = std::mem::take(&mut self.pending);
                    self.accumulated_text.push_str(&chunk);
                    out.push(ModelEvent::TextDelta {
                        block_index: self.text_block_index,
                        chunk,
                    });
                }
            }
            ThinkTagState::Inside => {
                if !self.pending.is_empty() {
                    let chunk = std::mem::take(&mut self.pending);
                    self.accumulated_thinking.push_str(&chunk);
                    out.push(ModelEvent::ThinkingDelta {
                        block_index: self.thinking_block_index,
                        chunk,
                    });
                }
                out.push(ModelEvent::ThinkingBlockCompleted {
                    block_index: self.thinking_block_index,
                    text: std::mem::take(&mut self.accumulated_thinking),
                    provider_opaque: None,
                });
            }
        }
    }
}
```

Wire `ThinkTagParser` into the existing `delta.content` processing path:
construct one per stream; call `feed(chunk, &mut out)` for each
`delta.content` arrival; on `finish_reason` arrival, call `finish(&mut out)`
followed by the existing `MessageCompleted` emission.

**Important**: this parser runs **only** when `delta.reasoning_content` is
absent for that stream. If the provider emits `reasoning_content` even
once, fall back to the Task 13 path and disable the tag parser for the
rest of the stream (the two flavors are mutually exclusive in practice).

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-model
```

Expected: all three new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/openai_compat/decode.rs crates/cogito-model/tests/openai_compat_replay.rs
git commit -m "feat(model/openai_compat): two-state <think> tag parser

State machine tolerates <think>/</think> split across SSE frames;
on stream end an unclosed <think> still seals as ThinkingBlockCompleted
(best-effort). Mutually exclusive with the reasoning_content path
(Task 13). Per ADR-0019 §5.3.a."
```

---

### Task 15: Provider config — `include_prior_thinking`

**Files:**
- Modify: `crates/cogito-model/src/provider_config.rs`

- [ ] **Step 1: Write the failing test**

In `crates/cogito-model/tests/provider_config.rs` (existing file — append):

```rust
#[test]
fn openai_compat_include_prior_thinking_defaults_false() {
    let toml = r#"
        kind = "openai_compat"
        base_url = "http://localhost:8000/v1"
        api_key_env = "LOCAL_KEY"
    "#;
    let cfg: ProviderConfig = toml::from_str(toml).unwrap();
    match cfg {
        ProviderConfig::OpenAiCompat(c) => {
            assert_eq!(c.include_prior_thinking, false);
        }
        other => panic!("expected OpenAiCompat, got {other:?}"),
    }
}

#[test]
fn openai_compat_include_prior_thinking_honors_explicit_true() {
    let toml = r#"
        kind = "openai_compat"
        base_url = "http://localhost:8000/v1"
        api_key_env = "LOCAL_KEY"
        include_prior_thinking = true
    "#;
    let cfg: ProviderConfig = toml::from_str(toml).unwrap();
    match cfg {
        ProviderConfig::OpenAiCompat(c) => assert!(c.include_prior_thinking),
        other => panic!("expected OpenAiCompat, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-model
```

Expected: fail with `unknown field include_prior_thinking` or `no field`.

- [ ] **Step 3: Add the field**

In `provider_config.rs`, inside the `OpenAiCompat` config struct:

```rust
    /// Whether to re-feed prior-turn `ContentBlock::Thinking` blocks
    /// back into outgoing messages. Most open-source reasoning models
    /// (DeepSeek-R1, QwQ) explicitly drop prior thinking on follow-up
    /// turns; default `false` matches that convention. Set `true` only
    /// if the backend model is documented to handle prior `<think>`
    /// context. See ADR-0019 §5.3.
    #[serde(default)]
    pub include_prior_thinking: bool,
```

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-model
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/provider_config.rs crates/cogito-model/tests/provider_config.rs
git commit -m "feat(model/openai_compat): provider config include_prior_thinking (default false)

Per ADR-0019 §5.3 round-trip note. Default off matches DeepSeek-R1 / QwQ
training conventions; opt in for backends that handle prior thinking."
```

---

### Task 16: OpenAI-compat encode — honor `include_prior_thinking`

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/encode.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn encode_default_drops_prior_thinking() {
    let msg = cogito_protocol::gateway::Message::Assistant {
        content: vec![
            cogito_protocol::content::ContentBlock::Thinking {
                text: "I should grep.".into(),
                provider_opaque: None,
            },
            cogito_protocol::content::ContentBlock::Text { text: "OK.".into() },
        ],
    };
    let wire = encode_assistant_message(&msg, /*include_prior_thinking=*/false).unwrap();
    let json = serde_json::to_value(&wire).unwrap();
    // Outgoing message must only carry "OK." — no <think> tags, no
    // reasoning_content field.
    let s = serde_json::to_string(&json).unwrap();
    assert!(!s.contains("<think>"), "thinking must be dropped: {s}");
    assert!(!s.contains("I should grep."), "thinking text must be absent: {s}");
    assert!(s.contains("OK."), "non-thinking text must remain: {s}");
}

#[test]
fn encode_include_prior_thinking_wraps_in_tags() {
    let msg = cogito_protocol::gateway::Message::Assistant {
        content: vec![
            cogito_protocol::content::ContentBlock::Thinking {
                text: "I should grep.".into(),
                provider_opaque: None,
            },
            cogito_protocol::content::ContentBlock::Text { text: "OK.".into() },
        ],
    };
    let wire = encode_assistant_message(&msg, /*include_prior_thinking=*/true).unwrap();
    let s = serde_json::to_string(&serde_json::to_value(&wire).unwrap()).unwrap();
    assert!(s.contains("<think>I should grep.</think>"), "tags must wrap thinking: {s}");
}
```

Replace `encode_assistant_message` with the actual function signature; the
existing function likely does not take `include_prior_thinking` — add the
parameter and thread it through callers (the `ModelGateway::stream` impl
will get it from its `OpenAiCompatGateway`'s config).

- [ ] **Step 2: Run the failing tests**

```bash
make test CRATE=cogito-model
```

Expected: fail.

- [ ] **Step 3: Implement the encode arm**

In `encode.rs`'s `ContentBlock` match (within the assistant-message encoder):

```rust
ContentBlock::Thinking { text, .. } => {
    if include_prior_thinking {
        // Re-wrap as <think>...</think> inline into the assistant
        // message's content string. Backends that honor reasoning_content
        // would need a different path; this default matches the most
        // common OpenAI-compat serving setups.
        accumulated_text.push_str("<think>");
        accumulated_text.push_str(text);
        accumulated_text.push_str("</think>");
    }
    // else: drop. Per DeepSeek-R1 / QwQ convention.
}
```

Thread `include_prior_thinking` from the `OpenAiCompatGateway`'s config
to the encode entry point (likely a single new arg on the existing fn).

- [ ] **Step 4: Run the tests**

```bash
make test CRATE=cogito-model
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/openai_compat/encode.rs
git commit -m "feat(model/openai_compat): encode honors include_prior_thinking

Default false: ContentBlock::Thinking blocks are dropped from outgoing
messages (matches DeepSeek-R1 / QwQ convention).
include_prior_thinking=true: re-wraps as <think>...</think> inline.
Per ADR-0019 §5.3 round-trip note."
```

---

## Phase 5 — Chaos + docs + roadmap

### Task 17: Extend resume-chaos with thinking-block scenario

**Files:**
- Modify: `crates/cogito-core/tests/resume_chaos.rs`

- [ ] **Step 1: Read existing scenarios**

Open the file. The existing scenarios are `no_tool_short_turn` and
`single_tool_happy_path` (per ROADMAP Sprint 3 closure notes). Identify
the helper functions that build event streams and inject crash points.

- [ ] **Step 2: Add the new scenario**

Add a function `thinking_then_text_then_tool_turn()` (or extend the
generator with a new variant) that produces, in order: `TurnStarted`,
`ThinkingBlockRecorded` (with non-empty `provider_opaque`),
`AssistantMessageAppended`, `ToolUseRecorded`, `ToolResultRecorded`,
`ModelCallCompleted`, `TurnCompleted`. Drive the existing chaos
machinery (PanicAt at each event boundary) and assert the four oracles
hold (prefix-immutable, terminal-equivalent, tool-mapping-equivalent,
final-text-equivalent).

The key oracle to extend: **after resume, replaying the event log must
reconstruct a `Message::Assistant.content` with `Thinking` at index 0,
`Text` at index 1, `ToolUse` at index 2**, regardless of crash point.
H04's projection (Task 8) handles this — the chaos test verifies it
end-to-end.

- [ ] **Step 3: Run the chaos test**

```bash
make chaos
```

Expected: green within the existing 10s budget.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/tests/resume_chaos.rs
git commit -m "test(chaos): cover turn with sealed ThinkingBlockRecorded before text + tool_use

Asserts resume reconstructs ContentBlock::Thinking at index 0 of
Message::Assistant.content for every crash point. Per ADR-0019 §4 +
Sprint follow-on."
```

---

### Task 18: Update component docs

**Files:**
- Modify: `docs/components/H02-step-recorder.md`
- Modify: `docs/components/H04-prompt-composer.md`
- Modify: `docs/components/H06-stream-demux.md`

- [ ] **Step 1: H02 doc — document the thinking buffer**

Find the section that documents the text-block buffer (`current_text_block`
field, `on_text_delta` + `on_text_block_complete` methods) and add a
mirror paragraph for thinking:

> **Thinking block buffer.** Symmetric to the text-block buffer.
> `on_thinking_delta(turn_id, chunk)` appends to the buffer and broadcasts
> `StreamEvent::ThinkingDelta`. `on_thinking_block_complete(provider_opaque)`
> flushes the accumulated text into a single `ThinkingBlockRecorded`
> event, attaching the provider-opaque payload supplied by the gateway's
> `ModelEvent::ThinkingBlockCompleted` (signature for Anthropic,
> encrypted_content for OpenAI Responses, `None` for OpenAI-compat).
> Per ADR-0019 §2-3.

- [ ] **Step 2: H04 doc — extend the projection table**

Add a row to the history-projection table that maps
`EventPayload::ThinkingBlockRecorded` → `ContentBlock::Thinking` and
note: "Block ordering within one assistant message is reconstructed by
envelope `seq` — Anthropic's 'thinking precedes text/tool_use' constraint
is automatically satisfied because providers emit thinking blocks first."

- [ ] **Step 3: H06 doc — extend the routing table**

Document the new `ModelEvent::ThinkingDelta` → `recorder.on_thinking_delta`
and `ThinkingBlockCompleted` → `recorder.on_thinking_block_complete` +
`content.push((block_index, ContentBlock::Thinking { text, provider_opaque }))`
arms.

- [ ] **Step 4: Commit**

```bash
git add docs/components/H02-step-recorder.md docs/components/H04-prompt-composer.md docs/components/H06-stream-demux.md
git commit -m "docs(components): document thinking-block handling in H02/H04/H06

Propagates ADR-0019 §2-4 decisions to the durable component docs per
CLAUDE.md doc-strategy guidance."
```

---

### Task 19: Update AGENTS.md with two inviolable rules

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Add to §"Inviolable design principles"**

Append (or insert into the existing numbered list):

> **N. Thinking content ordering.** Within one assistant turn's
> `Message::Assistant.content` array, `ContentBlock::Thinking` MUST
> precede `Text` and `ToolUse`. The Brain enforces this by walking
> event-log entries in `seq` order in H04 — providers emit thinking
> first, so seq order produces the correct ordering. Reordering or
> dropping `Thinking` blocks invalidates the next-turn signature check
> on Anthropic and the reasoning-item continuity on OpenAI Responses.
> Per ADR-0019 §4.

> **M. Persisted JSONL is append-only and never rewritten.** cogito
> never rewrites already-persisted event log files in place, regardless
> of how the events were originally shaped. This applies to backfilling,
> migration, normalization, or any other server-side rewrite. Old
> sessions with provider-specific quirks (e.g. `<think>…</think>` baked
> into `AssistantMessageAppended.text`) stay byte-for-byte as written.
> New shapes coexist with old shapes in storage; readers handle both.
> Per ADR-0019 §5.3.

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs(agents): inviolable rules for thinking-block order + append-only JSONL

Locks ADR-0019 §4 + §5.3 into the agent operating manual."
```

---

### Task 20: Update jsonl-v1 spec (additive, no filename change)

**Files:**
- Modify: `docs/data-model/jsonl-v1.md`

- [ ] **Step 1: Append a section for `ThinkingBlockRecorded`**

Locate the existing per-event-type sections (one per `EventPayload` variant).
Add a new section after `AssistantMessageAppended`:

```markdown
### `thinking_block_recorded`

Recorded when a reasoning/"thinking" content block is sealed by the
provider. Sibling to `assistant_message_appended` and `tool_use_recorded`
— one event per completed block, ordered by envelope `seq`.

| Field | Type | Required | Description |
|---|---|---|---|
| `text` | `string` | yes | Full reasoning text. May be empty for safety-redacted blocks. |
| `provider_opaque` | `object \| null` | no | Provider-specific round-trip payload. Schema not interpreted by cogito; see provider docs. |

**Provider-opaque shapes observed in production:**

- Anthropic: `{"signature": "<opaque>"}` or `{"redacted_data": "<opaque>"}`.
- OpenAI Responses: `{"item_id": "<id>", "encrypted_content": "<opaque>"}`.
- OpenAI-compat (`<think>` tag / `reasoning_content`): `null`.

Per ADR-0019.
```

- [ ] **Step 2: Confirm the file is still called `jsonl-v1.md`**

No file rename. `SCHEMA_VERSION` stays at 1.

- [ ] **Step 3: Commit**

```bash
git add docs/data-model/jsonl-v1.md
git commit -m "docs(data-model): document thinking_block_recorded event (additive, schema_version 1)

Per ADR-0019 §2 + ADR-0007 additive precedent. Filename unchanged."
```

---

### Task 21: Roadmap entry + cleanup

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Slot ADR-0019 implementation under v0.1**

Insert (under v0.1 Foundation, between Sprint 4 and Sprint 5, as a
sibling sprint — call it Sprint 4.7 to slot between MCP and Async Jobs;
adjust if the user has a different placement preference):

```markdown
#### Sprint 4.7 · Thinking content (ADR-0019) (1.5 day)

**Goal**: first-class representation of model reasoning across protocol,
Brain, and provider adapters without bumping `SCHEMA_VERSION` and without
rewriting persisted JSONL. Closes the gap before Anthropic extended
thinking, OpenAI Responses reasoning items, or OpenAI-compat
`<think>`-tag models are exposed to users.

- [ ] `cogito-protocol`: `ContentBlock::Thinking` + `EventPayload::ThinkingBlockRecorded` + `ModelEvent::ThinkingDelta` / `ThinkingBlockCompleted` + `StreamEvent::ThinkingDelta` (all additive, no SCHEMA_VERSION bump)
- [ ] `cogito-core::harness`: H02 thinking-block buffer + flush; H06 routing; H04 projection
- [ ] `cogito-model::anthropic`: decode thinking_delta + signature_delta + redacted_thinking; encode ContentBlock::Thinking back to wire
- [ ] `cogito-model::openai_compat`: `<think>` two-state SSE parser + `reasoning_content` field reader; `include_prior_thinking` config (default false)
- [ ] Resume-chaos: new scenario covering ThinkingBlockRecorded + AssistantMessageAppended + ToolUseRecorded in seq order
- [ ] Docs: H02/H04/H06 component docs + AGENTS.md inviolable rules + jsonl-v1.md additive entry
- [ ] **ADR-0019**: Reasoning content modeling and event scope (Accepted 2026-05-22)
```

- [ ] **Step 2: Run the full CI gate**

```bash
make ci
```

Expected: green across fmt, clippy, layer-check, test.

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md
git commit -m "docs(roadmap): slot ADR-0019 implementation as Sprint 4.7

Thinking content (ADR-0019) lands between Sprint 4 (MCP) and Sprint 5
(Async Jobs). No SCHEMA_VERSION bump; persisted JSONL is append-only
per ADR-0019 §5.3."
```

---

## Verification — done when

- [ ] `make ci` is green
- [ ] `make chaos` is green and includes the new ThinkingBlockRecorded scenario
- [ ] `docs/schemas/conversation-event-v1.json` carries the new `ContentBlock::Thinking` and `EventPayload::ThinkingBlockRecorded` shapes; CI drift gate passes
- [ ] `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl` includes a `thinking_block_recorded` row
- [ ] An old session JSONL file with `<think>…</think>` baked into `AssistantMessageAppended.text` still replays correctly (no normalization, no rewrite)
- [ ] `SCHEMA_VERSION` is still `1` in `crates/cogito-protocol/src/event.rs`
- [ ] AGENTS.md carries the two new inviolable rules
- [ ] ROADMAP.md Sprint 4.7 entry is present with all checkboxes

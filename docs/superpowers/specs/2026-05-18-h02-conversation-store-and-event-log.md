# Spec: H02 Step Recorder + `ConversationStore` + JSONL Backend + Cross-Language Event Log Contract

> **Status**: Draft (2026-05-18). Implements Sprint 1 of v0.1 Foundation.
> **Predecessor**: `2026-05-18-runtime-h01-execution-model-design.md` (Sprint 0 closure).
> **Successor planning doc**: `docs/superpowers/plans/2026-05-18-sprint-1-h02-jsonl.md` (to be written from this spec).
> **Brainstorming session**: 2026-05-18 (Q1–Q4 in conversation log).

## 0. Scope

This spec covers everything Sprint 1 must deliver to satisfy ROADMAP §"Sprint 1 · H02 Step Recorder + JSONL store":

1. `ConversationEvent` data model in `cogito-protocol` (Q1 outcome).
2. `ConversationStore` trait in `cogito-protocol` (Q2 outcome).
3. `cogito-store-jsonl` minimal backend, scoped to dev/debug use only (Q3 outcome).
4. `cogito-core::harness::step_recorder` skeleton + text-block lifecycle handling (Q4 outcome).
5. A **cross-language storage contract** commitment (ADR-0007) that frames the
   JSONL line format / future Postgres DDL as the cogito public API for non-Rust
   consumers (Q2 SaaS reframe).
6. Tooling: `schemars`-generated JSON Schema for `ConversationEvent`, checked
   into the repo and verified by CI.
7. A fixture file `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
   covering all 9 `EventPayload` variants.
8. Sprint 1 benchmark: `append_throughput` against `cogito-store-jsonl`,
   producing `docs/quality/v0.1-jsonl-baseline.md` as an **informational
   baseline** (not a locked production SLO — see §7).

Out of scope (recorded in §9 to lock the boundary):

- Real `cogito-core::harness::turn_driver` FSM (Sprint 2).
- Real `cogito-core::harness::stream_demux` (H06) (Sprint 2).
- `ConversationCatalog` Rust trait — explicitly deferred to v0.4 ADR-0014
  (TenantContext) (§4.4 explains why).
- Cross-session / cross-tenant query API (covered by external consumer
  services reading storage directly — §4).
- `cogito-store-postgres` (v0.4).
- Production-grade fsync semantics, rotation, multi-replica concerns.
- **Context management mechanism** (compaction, system-prompt injection,
  context-aware tool overrides). The architectural slot is locked by the
  ADR-0006 amendment in PR #6 (new `ContextManaged` FSM state, new H11
  component slot — see `docs/components/H11-context-manage.md`). The
  mechanism — trigger policy, summarization model, exact `EventPayload`
  variants for context lifecycle — is the central deliverable of the
  Context Management initiative (ADR-0008, pending; ROADMAP "Spike ·
  Context Management"). Until ADR-0008 lands, Sprint 2's `ContextManaged`
  state is implemented as a pass-through (immediate transition to
  `PromptBuilt`).

## 1. Background & motivation

### 1.1 Why this is the next sprint

ROADMAP places Sprint 1 immediately after Sprint 0 closure (now merged as
PR #5). Sprint 0 delivered the runtime scaffolding with `todo!()` bodies; the
event log + recorder + persistent backend is the first vertical slice that
runs real I/O.

This sprint is the contract-locking sprint for cogito's most public-facing
asset: the event log. Per ADR-0005 §4 Quality Gates, the event log is governed
by `schema_version` from day one. The brainstorming session that produced this
spec surfaced a major framing correction (Q2 reframe): the event log is not
just an internal serialization, it is the **cross-language contract** with
non-Rust SaaS services (Go/Python/Node) that read the same storage that
cogito writes. This elevates schema design from "internal detail" to
"public API equivalent."

### 1.2 What this sprint does NOT do

The Sprint 1 H02 implementation is intentionally minimal:

- Writes events to a `ConversationStore` trait object — does not yet drive a
  real turn.
- Sprint 2 (Minimal Loop) plugs Sprint 1's recorder into a real H01 Turn
  Driver + H06 Stream Demultiplexer + Anthropic adapter.
- Sprint 3 (Resume Coordinator) consumes `ConversationStore::latest_seq` +
  `replay`, which Sprint 1 lights up.

The recorder's interface contract with H01 / H06 is defined here; the actual
wiring is Sprint 2's work.

### 1.3 Reference implementations consulted

- **Codex Rust** (`/agents/codex/codex-rs/core/src/rollout/`): persistence,
  serde shape, batching policy, file layout.
- **Claude streaming wire format** (`debug/claude.event.log`): content_block
  lifecycle, delta granularity, how the upstream model boundary aligns with
  persistence.
- **Anthropic Messages API streaming docs**: SSE event ordering.

Codex's design informs ~70% of the choices here. Cogito diverges in two
deliberate places, both with rationale recorded:

1. Codex persists `EventMsg` directly into the rollout JSONL — UI events and
   model items live in the same log. Cogito splits them: `StreamEvent` is live
   broadcast (not persisted), `ConversationEvent` is the persisted form
   (post-batch where relevant). See ADR-0006 §7.
2. Codex's RolloutRecorder is per-session-handle. Cogito's `ConversationStore`
   is workspace-wide with `session_id` on every method (Q2b decision, §3.2).

## 2. `ConversationEvent` schema (Q1 outcome)

> **NOTE (amended 2026-05-19)**: `Eq` is NOT derived on
> `ConversationEvent`, `EventPayload`, `ContentBlock`, or `SessionMeta`
> because they transitively contain `serde_json::Value` (no `Eq` impl).
> The implementation derives `PartialEq` only. The spec snippets below
> show `Eq` for completeness; the implementation rationale is recorded
> in each type's doc-comment under `crates/cogito-protocol/src/`.

### 2.1 Envelope shape — adjacent-tag flatten pattern

```rust
// crates/cogito-protocol/src/event.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::ids::{EventId, SessionId, TurnId};

/// One persisted entry in a conversation's event log.
///
/// `ConversationEvent` is the persistent counterpart to [`StreamEvent`]:
/// where `StreamEvent` carries live, may-be-dropped fanout to UI/observability
/// subscribers, `ConversationEvent` is the durable record consumed by
/// resume, replay, and any external consumer reading the storage.
///
/// # Serde representation
///
/// The envelope fields (`schema_version`, `event_id`, `session_id`, `turn_id`,
/// `seq`, `ts`) are at the JSON top level. The payload is **adjacently tagged**
/// with `tag = "type"` and `content = "data"`, flattened into the envelope.
/// A serialized line looks like:
///
/// ```json
/// {"schema_version":1,"event_id":"01HX...","session_id":"01HY...",
///  "turn_id":"01HZ...","seq":42,"ts":"2026-05-18T10:00:00.123Z",
///  "type":"tool_use_recorded",
///  "data":{"call_id":"toolu_abc","tool_name":"read_file","args":{"path":"/tmp/x"}}}
/// ```
///
/// Adjacent tagging (vs internal tagging) sidesteps the serde restriction
/// that bit us in Sprint 0 Task 7: internally-tagged enums refuse newtype
/// variants whose body is a sequence (e.g. `Output(Vec<ContentBlock>)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationEvent {
    /// Schema version of the envelope **and** payload. Bumped together.
    /// See [`SCHEMA_VERSION`] for the current value.
    pub schema_version: u32,

    /// Globally unique, monotonic-per-process event identifier.
    /// Used by `tracing` spans and cross-system correlation. Not used
    /// for ordering within a session — see `seq` for that.
    pub event_id: EventId,

    /// Session this event belongs to.
    pub session_id: SessionId,

    /// Turn this event belongs to. `None` for session-level events such as
    /// [`EventPayload::SessionStarted`].
    pub turn_id: Option<TurnId>,

    /// Monotonic per-session sequence number. The first event in a session
    /// has `seq = 0`. Resume reads `latest_seq` and continues from `seq + 1`.
    pub seq: u64,

    /// Wall-clock timestamp at the moment the recorder serialized the event.
    /// Use only for human-facing display / debugging. Causality lives in
    /// `seq`, not `ts`.
    pub ts: DateTime<Utc>,

    /// Variant-specific payload.
    #[serde(flatten)]
    pub payload: EventPayload,
}

/// Schema version emitted by this build of cogito.
///
/// Per ADR-0005 §4 Quality Gate #2:
/// - 0.x: breaking changes require a migration tool + upgrade runbook.
/// - 1.0+: every future version must read every past version.
pub const SCHEMA_VERSION: u32 = 1;
```

### 2.2 `EventPayload` variants (9 total)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventPayload {
    /// First event of every session. Carries metadata that lets readers
    /// reconstruct context without separate index files. See `SessionMeta`.
    SessionStarted {
        meta: SessionMeta,
    },

    /// A new turn has begun. Carries the user input that triggered it.
    TurnStarted {
        user_input: Vec<ContentBlock>,
    },

    /// One content block of assistant text has been fully emitted by the
    /// model (i.e. `content_block_stop` for a text block has been observed).
    /// The recorder accumulates `text_delta` chunks for the block and writes
    /// **one** event here per block. See §6.1 for the lifecycle.
    AssistantMessageAppended {
        text: String,
    },

    /// The model emitted a tool_use content block. Records the call params
    /// for replay and audit. The tool dispatcher (H08) consumes the same
    /// information independently.
    ToolUseRecorded {
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },

    /// H08 returned a `ToolResult` for a previously recorded call.
    ToolResultRecorded {
        call_id: String,
        result: ToolResult,
    },

    /// The turn paused on an async tool call. The driving Brain task has
    /// exited; the actor is now in `PausedOnJob`. The next event for this
    /// turn will be a `JobCompletedRecorded` (when the job finishes) or a
    /// `TurnFailed` (if the job fails / the actor is cancelled).
    TurnPaused {
        job_id: JobId,
    },

    /// An async job that previously paused this turn has finished.
    /// Recording this event is the prerequisite for transitioning the turn
    /// out of `PausedOnJob` and back into `ModelCalling`.
    JobCompletedRecorded {
        job_id: JobId,
        outcome: JobOutcome,
    },

    /// The turn reached terminal Completed state (model returned
    /// `stop_reason = end_turn` without further tool calls).
    TurnCompleted {
        outcome: TurnOutcome,
    },

    /// The turn ended in failure with a structured reason.
    TurnFailed {
        reason: TurnFailureReason,
    },
}
```

### 2.3 Supporting types: `SessionMeta`, `ContentBlock`

```rust
/// Session-level metadata recorded once at session open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Cogito library version that created this session.
    pub cogito_version: String,
    /// Strategy name (from `HarnessStrategy::name`) selected for this session.
    /// `None` until v0.5 lights up real strategies; default `"default"`.
    pub strategy: Option<String>,
    /// Model identifier intended for this session (e.g. `"claude-sonnet-4-6"`).
    pub model: Option<String>,
    /// Optional consumer-supplied user identifier for the SaaS catalog use
    /// case (§4). Cogito does no auth on this field — it is opaque
    /// pass-through metadata.
    pub user_id: Option<String>,
    /// Optional consumer-supplied tenant identifier. Cogito propagates only;
    /// enforcement is the consumer's responsibility (ADR-0005 §1).
    pub tenant_id: Option<String>,
    /// Opaque consumer-supplied metadata; preserved verbatim for catalog use.
    #[serde(default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One content block as defined by the Anthropic / OpenAI wire formats.
/// In v0.1 we support Text, ToolUse, ToolResult. `Image` lands in v0.2
/// (ADR-0007 storage). `#[non_exhaustive]` keeps additive variants
/// non-breaking under ADR-0005 §5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        result: ToolResult,
    },
}
```

`ToolResult`, `JobOutcome`, `TurnOutcome`, `TurnFailureReason`, `JobId` are
already defined in `cogito-protocol` (landed in Sprint 0); this spec re-uses
them as-is.

### 2.4 `schema_version` and forward-compatibility policy

- `SCHEMA_VERSION = 1` for everything Sprint 1 ships.
- Adding a new `EventPayload` variant during 0.x is **additive**, does NOT
  bump `schema_version`. The `#[non_exhaustive]` attribute is the
  enforcement mechanism.
- Renaming or removing a variant, changing a variant's `data` field set, or
  changing serde rename rules — these are breaking. They require:
  1. Bump `SCHEMA_VERSION` (e.g. to 2).
  2. Ship a migration tool in `crates/cogito-migrate-event-log` (does not
     exist yet; created on first need).
  3. Document in `CHANGELOG.md` and the relevant ADR.
  4. CI runs a v(N-1) writer → vN reader equivalence test (ADR-0005 §4 #2).

The reader contract: **a vN reader MUST accept events with `schema_version <= N`.**
Stub fields default-init (`#[serde(default)]`) so additive 0.x changes do not
require version bumps on readers.

### 2.5 Sample JSONL lines

Each line below ends with `\n`. Long fields are line-wrapped here for
display only.

```jsonl
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0X8","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":null,"seq":0,"ts":"2026-05-18T10:00:00.000Z","type":"session_started","data":{"meta":{"cogito_version":"0.1.0","strategy":"default","model":"claude-sonnet-4-6","user_id":"u_42","tenant_id":null,"extra":{}}}}
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0X9","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":"01J9C0R0K0TURN0TURN0TURN00","seq":1,"ts":"2026-05-18T10:00:00.100Z","type":"turn_started","data":{"user_input":[{"type":"text","data":{"text":"read /tmp/x"}}]}}
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0XA","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":"01J9C0R0K0TURN0TURN0TURN00","seq":2,"ts":"2026-05-18T10:00:00.250Z","type":"assistant_message_appended","data":{"text":"Reading /tmp/x now."}}
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0XB","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":"01J9C0R0K0TURN0TURN0TURN00","seq":3,"ts":"2026-05-18T10:00:00.260Z","type":"tool_use_recorded","data":{"call_id":"toolu_01","tool_name":"read_file","args":{"path":"/tmp/x"}}}
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0XC","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":"01J9C0R0K0TURN0TURN0TURN00","seq":4,"ts":"2026-05-18T10:00:00.280Z","type":"tool_result_recorded","data":{"call_id":"toolu_01","result":{"Output":[{"type":"text","data":{"text":"file contents"}}]}}}
{"schema_version":1,"event_id":"01J9C0R0K3T0X8K3T0X8K3T0XD","session_id":"01J9C0R0K0SESSION0SESSION0","turn_id":"01J9C0R0K0TURN0TURN0TURN00","seq":5,"ts":"2026-05-18T10:00:00.350Z","type":"turn_completed","data":{"outcome":{"Completed":{}}}}
```

(See §4.3 fixture file `sample-v1.jsonl` for the full canonical sample.)

## 3. `ConversationStore` trait (Q2 outcome)

### 3.1 Methods

```rust
// crates/cogito-protocol/src/store.rs

use async_trait::async_trait;
use futures::stream::BoxStream;
use thiserror::Error;

use crate::event::ConversationEvent;
use crate::ids::SessionId;

#[async_trait]
pub trait ConversationStore: Send + Sync + 'static {
    /// Append a single event to the backend.
    ///
    /// # Durability semantics — backend-defined
    ///
    /// Each backend MUST document its durability guarantee in its crate
    /// docs. As of v0.1:
    ///
    /// - `cogito-store-jsonl` (dev/debug): userspace-flushed via
    ///   `tokio::fs::File::flush`, **not fsynced**. Process crash is
    ///   recoverable; power loss may lose recent events.
    /// - `cogito-store-postgres` (v0.4): per-transaction durable.
    ///
    /// Returns `StoreError` on I/O failure or serde failure. On
    /// `StoreError::Io`, the backend's internal state for this session is
    /// considered tainted: callers SHOULD `close(session_id)` and reopen
    /// before further appends.
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError>;

    /// Force any backend-internal buffers for `session_id` to be flushed
    /// to its persistent layer. Equivalent to a no-op for backends with
    /// no internal buffering. JSONL flushes its `tokio::fs::File`.
    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Release backend-side resources held for `session_id` (file handles,
    /// connection slot, etc.). After `close`, subsequent `append` for the
    /// same `session_id` is valid — the backend re-acquires resources.
    /// This is purely a resource-management hint, not a lifecycle event.
    async fn close(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Return the largest `seq` ever appended for `session_id`, or `None`
    /// if no events exist for this session. Used by Sprint 3's H03 Resume
    /// Coordinator to decide the replay starting point.
    ///
    /// Implementations MAY cache this value across calls; they MUST
    /// invalidate on `append`.
    async fn latest_seq(&self, session_id: SessionId)
        -> Result<Option<u64>, StoreError>;

    /// Stream events for `session_id` where `event.seq > from_seq`, in
    /// strict ascending `seq` order (strict greater-than, not greater-or-
    /// equal). `from_seq = 0` reads from the second event onward (the
    /// first event has `seq = 0` and is excluded); to read net-new events
    /// after a resume, pass `from_seq = latest_seq` — i.e. the last
    /// persisted seq, NOT `latest_seq + 1` (that would skip one event).
    ///
    /// The returned stream is single-pass; cloning is the caller's
    /// responsibility if needed.
    fn replay(&self, session_id: SessionId, from_seq: u64)
        -> BoxStream<'_, Result<ConversationEvent, StoreError>>;
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("session {session_id} not found")]
    SessionNotFound { session_id: SessionId },

    #[error("backend io error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("serde error: {source}")]
    Serde {
        #[from]
        source: serde_json::Error,
    },

    #[error("schema version {found} not supported; this build understands <= {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("backend error: {message}")]
    Backend { message: String },
}
```

### 3.2 Ownership model (Q2b decision — workspace-wide)

The Runtime holds **one** `Arc<dyn ConversationStore>` shared by all
`SessionActor`s. Every method takes `session_id` (carried explicitly for
`flush`/`close`/`latest_seq`/`replay`, implicit inside `event.session_id`
for `append`).

Rationale: production backend (Postgres v0.4) is the constraint — one pool
serves all sessions naturally. JSONL backend internally maintains a
`DashMap<SessionId, FileHandle>` to map session-id to file resources.
Per-session ownership (Codex's pattern) doesn't fit Postgres without
contrived wrapper traits.

```rust
// Wire-up sketch in cogito-core::runtime::builder
let store: Arc<dyn ConversationStore> = Arc::new(
    JsonlStore::new(&runtime_config.session_root)?
);
let runtime = Runtime::builder()
    .store(store.clone())
    .build();
```

`SessionActor` clones the `Arc` on spawn; `close(session_id)` is called from
the actor's `Shutdown` handler.

### 3.3 Contract tests

Every backend MUST pass a shared contract test suite living in
`testing/cogito-test-fixtures/src/store_contract.rs`:

```rust
pub async fn run_store_contract<F, Fut, S>(make_store: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = Arc<S>>,
    S: ConversationStore + 'static,
{
    test_append_then_latest_seq(&make_store).await;
    test_append_then_replay_full(&make_store).await;
    test_append_then_replay_from_offset(&make_store).await;
    test_replay_empty_session_returns_empty_stream(&make_store).await;
    test_latest_seq_empty_session_returns_none(&make_store).await;
    test_multiple_sessions_isolated(&make_store).await;
    test_close_then_reappend(&make_store).await;
    test_concurrent_append_two_sessions(&make_store).await;
    test_schema_version_carried_through(&make_store).await;
}
```

Each backend crate has a `tests/contract.rs` integration test that calls
`run_store_contract` with its own factory closure. Sprint 1 ships only
`cogito-store-jsonl` running this suite; Sprint 4 / v0.4 add Postgres.

## 4. Cross-language storage contract (Q2 SaaS reframe — ADR-0007)

### 4.1 The principle

A cogito-using SaaS deployment looks like:

```
┌────────────────────────────────────────────────────────────────────────┐
│ Rust process (consumer service embedding cogito)                       │
│                                                                        │
│   cogito Brain ──── append() ─────► ConversationStore (Rust trait)     │
│                                              │                         │
└──────────────────────────────────────────────┼─────────────────────────┘
                                               │
                                               ▼
                                  ┌─────────────────────────┐
                                  │ Shared storage           │
                                  │  v0.1: JSONL files       │
                                  │  v0.4: PostgreSQL table  │
                                  └─────────────┬───────────┘
                                                │
                              ┌─────────────────┼────────────────┐
                              ▼                 ▼                ▼
                  ┌──────────────────┐ ┌─────────────────┐ ┌─────────────┐
                  │ Go HTTP service  │ │ Python analytics│ │ Node BFF    │
                  │ (catalog query)  │ │ (billing/report)│ │ (websocket) │
                  └──────────────────┘ └─────────────────┘ └──────────────┘
```

External readers cannot consume a Rust trait. They consume the **storage
itself** (JSONL files / Postgres rows / future S3 objects / future Kafka
topics). Therefore:

> **ADR-0007 statement** (to be written as `docs/adr/0007-event-log-as-cross-language-contract.md`):
>
> The `ConversationStore` Rust trait serves Brain's command + single-session
> replay path only. Any cross-session, cross-tenant, user-facing query
> capability is exposed via the **storage-level contract** (v0.1 = JSONL
> line format; v0.4 = Postgres DDL), not via Rust traits. Storage formats
> are SemVer-equivalent public API governed by the `schema_version`
> mechanism in ADR-0005 §4 #2.

### 4.2 What this commits cogito to deliver

| Artifact | Path | Sprint |
|---|---|---|
| `ConversationEvent` Rust types | `crates/cogito-protocol/src/event.rs` | 1 |
| JSON Schema, schemars-generated | `docs/schemas/conversation-event-v1.json` | 1 |
| Human-readable spec | `docs/data-model/jsonl-v1.md` | 1 |
| Canonical fixture | `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl` | 1 |
| ADR-0007 (this principle) | `docs/adr/0007-event-log-as-cross-language-contract.md` | 1 |
| AGENTS.md inviolable rule (catalog) | `AGENTS.md` §"Inviolable design principles" | 1 |
| Postgres DDL + migration file | `crates/cogito-store-postgres/migrations/0001_init.sql` | v0.4 |
| `ConversationCatalog` Rust trait | (deferred — see §4.4) | v0.4 + |

### 4.3 JSON Schema generation

Add `schemars = "0.8"` to workspace deps and derive `JsonSchema` on all
relevant types in `cogito-protocol`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConversationEvent { /* ... */ }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventPayload { /* ... */ }
```

A `just gen-schema` recipe runs a small binary `tools/gen-schema` that
serializes the schema to `docs/schemas/conversation-event-v1.json`. CI runs
`just gen-schema --check` (a flag the tool understands) to verify the
committed file matches the current Rust source — drift fails the build.

```just
gen-schema:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json

gen-schema-check:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json \
        --check
```

The `cogito-gen-schema` package is a new tools-only crate under `tools/`
with `publish = false` and `[[bin]]`-only — not part of the library.

### 4.4 Why `ConversationCatalog` is NOT a v0.1 deliverable

The catalog API for SaaS query services would look like:

```rust
// HYPOTHETICAL — DO NOT IMPLEMENT IN v0.1
async fn list_for_user(&self, user_id: &UserId, page: PageCursor) -> ...;
async fn get_meta(&self, session_id: SessionId) -> ...;
async fn search(&self, q: SearchQuery) -> ...;
```

Problems with shipping this in v0.1:

1. `UserId`, `TenantContext`, `PageCursor`, `SearchQuery` are types
   designed in ADR-0012/0013/0014 at v0.4. Defining them now means churning
   them later.
2. External consumers are typically Go/Python/Node — they cannot consume a
   Rust trait. The actual SaaS catalog will be implemented as direct SQL
   against the v0.4 Postgres schema, not via a Rust trait.
3. v0.1 JSONL backend has no efficient cross-session index; implementing
   `list_for_user` would be a O(N) directory scan — useful for dev but
   misleading to expose as a "trait contract."

**Sprint 1 commits to the principle (via ADR-0007), defers the
implementation.** This is recorded explicitly so future contributors don't
accidentally add catalog methods to `ConversationStore`.

### 4.5 AGENTS.md amendment

Add to `AGENTS.md` §"Inviolable design principles":

> ### 7. `ConversationStore` is Brain's command + single-session replay trait
>
> Methods on `ConversationStore` MUST be scoped to: (a) writing one event,
> (b) reading events for one explicitly-named session. Adding any
> cross-session, cross-tenant, or user-history query method to this trait
> is a design error.
>
> Cross-session / catalog access for external (Go/Python/Node) services is
> served by reading the underlying storage directly (JSONL files in v0.1;
> Postgres tables in v0.4). See ADR-0007 for the principle and ADR-0014
> (v0.4) for the `TenantContext` model.

## 5. `cogito-store-jsonl` implementation (Q3 outcome — simplified)

### 5.1 Scope reset (per user direction 2026-05-18)

JSONL is the **dev/debug** backend for the v0.1–v0.3 development cycle. It
is not a production target. Therefore:

- No fsync, no rotation, no path sharding, no `SyncMode` config knob.
- "Whatever Codex does, simpler if possible."
- Production durability concerns move to `cogito-store-postgres` (v0.4).

### 5.2 File layout

- Root directory: provided by `JsonlStore::new(root: impl AsRef<Path>)`.
- Per-session file: `<root>/<session_id>.jsonl`.
  - `session_id` is a ULID rendered as Crockford base32 (26 chars).
  - Flat layout — no `YYYY/MM/DD` sharding, no hashed buckets. Dev
    environments don't produce 10k+ files.
- First line of every file is the `SessionStarted` event (Q3c).
- No sidecar metadata files. No index files.

### 5.3 Implementation skeleton

```rust
// crates/cogito-store-jsonl/src/lib.rs

#![warn(clippy::pedantic)]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::stream::{self, BoxStream, StreamExt};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use cogito_protocol::{ConversationEvent, ConversationStore, SessionId, StoreError, SCHEMA_VERSION};

/// JSONL backend for `ConversationStore`. **Dev/debug only.** See crate
/// docs for durability semantics and the rationale for not targeting
/// production use.
pub struct JsonlStore {
    root: PathBuf,
    handles: DashMap<SessionId, Arc<Mutex<File>>>,
}

impl JsonlStore {
    /// Create a new JSONL store rooted at `root`. The directory is
    /// created on first append; `new` performs no I/O.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            handles: DashMap::new(),
        }
    }

    fn path_for(&self, sid: &SessionId) -> PathBuf {
        self.root.join(format!("{sid}.jsonl"))
    }

    async fn handle_for(&self, sid: &SessionId) -> Result<Arc<Mutex<File>>, StoreError> {
        if let Some(h) = self.handles.get(sid) {
            return Ok(Arc::clone(&h));
        }
        // Create root if missing on first append.
        tokio::fs::create_dir_all(&self.root).await?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.path_for(sid))
            .await?;
        let arc = Arc::new(Mutex::new(file));
        self.handles.insert(sid.clone(), Arc::clone(&arc));
        Ok(arc)
    }
}

#[async_trait]
impl ConversationStore for JsonlStore {
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError> {
        let handle = self.handle_for(&event.session_id).await?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut f = handle.lock().await;
        f.write_all(&line).await?;
        f.flush().await?; // userspace flush only — see crate-level durability note
        Ok(())
    }

    async fn flush(&self, sid: SessionId) -> Result<(), StoreError> {
        if let Some(h) = self.handles.get(&sid) {
            let mut f = h.lock().await;
            f.flush().await?;
        }
        Ok(())
    }

    async fn close(&self, sid: SessionId) -> Result<(), StoreError> {
        if let Some((_, h)) = self.handles.remove(&sid) {
            let mut f = h.lock().await;
            f.flush().await?;
            // File handle drops when the Arc is dropped after this scope.
        }
        Ok(())
    }

    async fn latest_seq(&self, sid: SessionId) -> Result<Option<u64>, StoreError> {
        let path = self.path_for(&sid);
        if !path.exists() {
            return Ok(None);
        }
        // Read whole file, parse last non-empty line. v0.1 acceptable cost
        // for dev/debug (sessions are small). Sprint 3 may add an in-memory
        // cache if the resume test surfaces a hot path.
        let text = tokio::fs::read_to_string(&path).await?;
        let last = text.lines().rev().find(|l| !l.trim().is_empty());
        match last {
            None => Ok(None),
            Some(line) => {
                let event: ConversationEvent = serde_json::from_str(line)?;
                Ok(Some(event.seq))
            }
        }
    }

    fn replay(
        &self,
        sid: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>> {
        let path = self.path_for(&sid);
        let stream = async_stream::try_stream! {
            let file = match tokio::fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => Err(StoreError::from(e))?,
            };
            let mut lines = BufReader::new(file).lines();
            while let Some(line) = lines.next_line().await? {
                if line.trim().is_empty() {
                    continue;
                }
                let event: ConversationEvent = serde_json::from_str(&line)?;
                if event.schema_version > SCHEMA_VERSION {
                    Err(StoreError::UnsupportedSchemaVersion {
                        found: event.schema_version,
                        supported: SCHEMA_VERSION,
                    })?;
                }
                if event.seq > from_seq {
                    yield event;
                }
            }
        };
        Box::pin(stream)
    }
}
```

### 5.4 What we explicitly skip

| Concern | Decision | Rationale |
|---|---|---|
| `sync_data()` per append | Skip | Codex doesn't either; dev/debug doesn't need it |
| File rotation | Skip | Dev sessions don't grow past meaningful size |
| Path sharding (YYYY/MM/DD or hash buckets) | Skip | <100 files in dev environments |
| `SyncMode` config knob | Skip | Single-mode; v0.4 Postgres covers production needs |
| In-memory `latest_seq` cache | Skip in Sprint 1, revisit in Sprint 3 if benchmarks justify |
| Compaction / archival | Skip | Not a v0.x concern |
| File locking against external readers | Skip | Append-only + flush is enough for dev tail-following |

## 6. H02 step_recorder (Q4 outcome)

### 6.1 Text block lifecycle (the only "batching" we do)

Per Q4 brainstorming: Codex and Claude Code both batch by **wire-protocol
content_block boundary**, not by timer or character threshold. Cogito
follows the same model.

```rust
// crates/cogito-core/src/harness/step_recorder.rs

use std::sync::Arc;

use cogito_protocol::{
    ConversationEvent, ConversationStore, EventPayload, EventId, SessionId,
    StreamEvent, TurnId, SCHEMA_VERSION,
};
use chrono::Utc;
use tokio::sync::broadcast;

/// H02 Step Recorder.
///
/// Owns the live mapping from H01 / H06 events into the dual streams:
/// - **Persisted**: `ConversationEvent` to `ConversationStore`.
/// - **Live broadcast**: `StreamEvent` to subscribers.
///
/// Text-block lifecycle:
/// 1. `on_text_delta` accumulates chunks for the current content block
///    AND broadcasts each chunk as `StreamEvent::TextDelta`. Nothing is
///    persisted yet.
/// 2. `on_text_block_complete` (triggered by H06 on `content_block_stop`
///    for a text block) writes one `AssistantMessageAppended` carrying the
///    full block text.
///
/// On crash mid-block: the recorder dies with the actor (no cross-turn
/// state per ADR-0006 §3). The accumulated text is lost. On resume, the
/// last `seq` is before the unfinished `AssistantMessageAppended`, so the
/// turn re-runs from `ModelCalling`. The model re-streams. This is correct
/// because partial assistant outputs are not part of the canonical
/// conversation history.
pub struct StepRecorder {
    store: Arc<dyn ConversationStore>,
    events_tx: broadcast::Sender<StreamEvent>,
    session_id: SessionId,
    seq_counter: u64,
    current_text_block: Option<TextBlockBuf>,
}

struct TextBlockBuf {
    turn_id: TurnId,
    text: String,
}

impl StepRecorder {
    pub fn new(
        store: Arc<dyn ConversationStore>,
        events_tx: broadcast::Sender<StreamEvent>,
        session_id: SessionId,
        seq_start: u64,
    ) -> Self {
        Self {
            store,
            events_tx,
            session_id,
            seq_counter: seq_start,
            current_text_block: None,
        }
    }

    pub async fn record_session_started(&mut self, meta: SessionMeta)
        -> Result<(), StoreError>
    {
        self.append_payload(None, EventPayload::SessionStarted { meta }).await
    }

    pub async fn record_turn_started(&mut self, turn_id: TurnId, user_input: Vec<ContentBlock>)
        -> Result<(), StoreError>
    {
        let _ = self.events_tx.send(StreamEvent::TurnStarted);
        self.append_payload(Some(turn_id), EventPayload::TurnStarted { user_input }).await
    }

    pub fn on_text_delta(&mut self, turn_id: TurnId, chunk: String) {
        let _ = self.events_tx.send(StreamEvent::TextDelta { chunk: chunk.clone() });
        self.current_text_block
            .get_or_insert_with(|| TextBlockBuf { turn_id, text: String::new() })
            .text
            .push_str(&chunk);
    }

    pub async fn on_text_block_complete(&mut self) -> Result<(), StoreError> {
        if let Some(buf) = self.current_text_block.take() {
            self.append_payload(
                Some(buf.turn_id),
                EventPayload::AssistantMessageAppended { text: buf.text },
            ).await?;
        }
        Ok(())
    }

    // … record_tool_use, record_tool_result, record_turn_paused,
    //   record_job_completed, record_turn_completed, record_turn_failed …

    async fn append_payload(
        &mut self,
        turn_id: Option<TurnId>,
        payload: EventPayload,
    ) -> Result<(), StoreError> {
        let event = ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: self.session_id.clone(),
            turn_id,
            seq: self.seq_counter,
            ts: Utc::now(),
            payload,
        };
        self.store.append(&event).await?;
        self.seq_counter = self.seq_counter.saturating_add(1);
        Ok(())
    }
}
```

### 6.2 Flow-through for non-text events

`record_tool_use` / `record_tool_result` / `record_turn_*` / `record_job_*` —
each is a single `append_payload(...)` call. No batching. They also emit the
corresponding `StreamEvent` (e.g. `ToolDispatchStarted`/`ToolDispatchEnded`)
where one exists.

### 6.3 AGENTS.md amendment for the text-batching rule

Replace the current text-delta exception:

```diff
 ### 2. H02 Step Recorder writes events immediately

-No batching. No buffering across components. The only exception is
-`text_delta` events, which may be batched for ≤200ms or ≤500 chars,
-then flushed.
+No batching. No buffering across components. `StreamEvent::TextDelta`
+is live-only (never persisted by H02). Persistence happens at the
+wire-protocol content_block boundary: when H06 emits
+`text_block_complete`, H02 writes one `AssistantMessageAppended`
+carrying the full block text. This matches Codex and Claude Code,
+both of which align persistence with content_block boundaries.
+No timer-based or size-based batching exists.
```

The corresponding `docs/components/H02-step-recorder.md` gets a new section
"Text block lifecycle" that explains the same with the lifecycle diagram.

## 7. Sprint 1 benchmark — `append_throughput` (informational baseline)

ADR-0005 §3 requires "Sprint 1 must include a benchmark suite that measures
the first metric to lock its real number." This sprint delivers the
benchmark, but with revised positioning:

> Sprint 1's benchmark establishes a **JSONL dev-grade baseline**, not a
> production SLO. The provisional P99 < 5 ms target remains provisional
> until `cogito-store-postgres` lands in v0.4 and runs the same benchmark
> under production-realistic conditions.

### 7.1 Benchmark contents

`crates/cogito-store-jsonl/benches/append_throughput.rs` (criterion):

- **Setup**: temp dir, fresh `JsonlStore`, one session, pre-built
  `ConversationEvent` (a `ToolUseRecorded` with ~200 bytes args).
- **Measure**: latency per `append` call; throughput over 10k iterations.
- **Report**: P50, P99, P99.9 latency; events/sec throughput.
- **Output**: criterion writes `target/criterion/`. A `just bench-baseline`
  recipe runs the bench and renders `docs/quality/v0.1-jsonl-baseline.md`
  from the criterion output. Manual review at first; CI gating deferred.

### 7.2 What the baseline file looks like

```markdown
# v0.1 JSONL Append Baseline (informational)

> Measured against `cogito-store-jsonl` on $(uname -a) at $(date).
> This is a **dev-grade backend baseline**, not a production SLO. The
> ADR-0005 §3 P99 < 5 ms target is locked at v0.4 against
> `cogito-store-postgres` under production-realistic load.

| Metric | Value |
|---|---|
| Events appended | 10000 |
| P50 latency | N µs |
| P99 latency | N µs |
| P99.9 latency | N µs |
| Mean throughput | N events/sec |
| Event payload size (median) | N bytes |
```

### 7.3 ADR-0005 §3 footnote

Plan 2 includes a task to add a footnote to the ADR-0005 §3 SLO table:

```diff
 | P99 step record write latency | < 5 ms | H02 + `ConversationStore` impl |
+
+† JSONL backend baseline is informational only (see
+`docs/quality/v0.1-jsonl-baseline.md`); production SLO is locked against
+`cogito-store-postgres` at v0.4.
```

## 8. Docs & inviolable rules — all updates

| Doc | Update |
|---|---|
| `AGENTS.md` §3 (text batching) | Per §6.3 |
| `AGENTS.md` §"Inviolable design principles" | Add rule #7 from §4.5 |
| `ARCHITECTURE.md` §Workspace layout | No change |
| `ARCHITECTURE.md` §"Component contracts" table | Add `ConversationStore`, `ConversationEvent`, `EventPayload` |
| `ROADMAP.md` Sprint 1 checklist | Mark items completed when Plan 2 lands |
| `docs/components/H02-step-recorder.md` | Add "Text block lifecycle" section; link to ADR-0007 |
| `docs/adr/0005-production-scope-and-quality-gates.md` | Add §3 footnote (per §7.3) |
| `docs/adr/0007-event-log-as-cross-language-contract.md` | New ADR per §4 |
| `docs/adr/README.md` | Add ADR-0007 to index |
| `docs/data-model/jsonl-v1.md` | New, human-readable schema spec |
| `docs/schemas/conversation-event-v1.json` | New, schemars-generated |
| `docs/quality/v0.1-jsonl-baseline.md` | New, criterion-rendered baseline |
| `CHANGELOG.md` | First v0.1 entry: schema_version=1 frozen, ConversationStore trait stable for 0.x |

## 9. Out of scope (locked for clarity)

Items deliberately excluded from Sprint 1, recorded here so future
contributors do not slip them in unprompted:

- **Real H01 Turn Driver FSM body** — Sprint 2.
- **Real H06 Stream Demultiplexer** — Sprint 2. Sprint 1 mocks the
  H06→H02 interface in unit tests.
- **Real H08 Tool Dispatcher** — Sprint 2. Sprint 1's H02 receives mock
  `ToolUseRecorded` / `ToolResultRecorded` events from tests.
- **Async jobs** — Sprint 4. Sprint 1's H02 has `record_turn_paused` and
  `record_job_completed` for schema completeness, but no real wiring.
- **Anthropic / OpenAI model adapters** — Sprint 2 / 5.
- **Resume Coordinator (H03)** — Sprint 3. Sprint 1 exposes
  `latest_seq` + `replay` so Sprint 3 has nothing to add to the trait.
- **Postgres backend** — v0.4.
- **`ConversationCatalog` Rust trait** — Deferred indefinitely (per §4.4).
  External SaaS catalog services read storage directly.
- **`TenantContext` propagation** — v0.4 (ADR-0014). Sprint 1's
  `SessionMeta.tenant_id` is an opaque pass-through field, not enforced.
- **`Redactor` trait for secret redaction** — v0.2 default impl.
- **Production-grade fsync, rotation, multi-replica** — JSONL is dev-only,
  these belong to `cogito-store-postgres`.
- **Hook pipeline (H09) integration** — Sprint 6.
- **TUI** — Sprint 6.

## 10. Cross-references

- ADR-0001: workspace layout
- ADR-0002: event sourcing
- ADR-0003: state-machine Turn Driver
- ADR-0004: Brain / Hands / Session boundaries
- ADR-0005: production scope + quality gates (§3 SLO, §4 schema_version, §5 compatibility)
- ADR-0006: Runtime + H01 execution model
- ADR-0007: event log as cross-language contract (to be written under this spec)
- AGENTS.md: §2 (immediate write rule), §3 (state in Conversation Service), new §7 (catalog scope)
- ROADMAP.md: Sprint 1 checklist
- `docs/components/H02-step-recorder.md`: implementation note added under this spec
- Codex Rust references:
  - `codex-rs/protocol/src/protocol.rs:1664` — `RolloutLine`
  - `codex-rs/protocol/src/protocol.rs:1606` — `RolloutItem`
  - `codex-rs/core/src/rollout/recorder.rs:469` — `JsonlWriter` (flush-only)
  - `codex-rs/core/src/rollout/policy.rs` — delta non-persistence policy
- Brainstorming source: 2026-05-18 conversation, Q1–Q4

## 11. Open questions deliberately left for Plan 2

- Whether `cogito-gen-schema` lives under `tools/` (separate workspace
  member) or as a `[[bin]]` inside `cogito-protocol`. Cost trade-off
  resolved during Plan 2 task breakdown.
- Whether `EventId::new()` uses `ulid::Ulid::new()` directly or wraps a
  process-local monotonic generator. Defer until Plan 2 task on `EventId`
  type.
- Exact contract test names — listed in §3.3, finalized during Plan 2.

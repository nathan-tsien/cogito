# Sprint 11 — Subagent (S2 minimal) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a synchronous `delegate(role, input) → output` tool that runs a bounded subtask in a fresh, isolated child session and returns its final text to the parent agent.

**Architecture:** A `BrainSpawner` trait (cogito-protocol) is injected into every tool's `ExecCtx`; the `Runtime` implements it. `DelegateToolProvider` (cogito-core::runtime::subagent) reads `ctx.brain_spawner` and calls `run_to_completion`, which opens a child top-level session, drives it to a terminal turn via the broadcast stream, and replays the child log for the final assistant text. Parent↔child linkage is recorded child-side as typed `SessionMeta` fields. All protocol changes are additive (no `SCHEMA_VERSION` bump).

**Tech Stack:** Rust 2024, `async_trait`, `tokio`, `serde`/`schemars`, `thiserror`, `cargo nextest`. Read `docs/adr/0011-subagent-execution-model.md` and `docs/superpowers/specs/2026-05-30-sprint-11-subagent-minimal-design.md` first.

**Commands:** `make fmt`, `make fix CRATE=<c>`, `make test CRATE=<c>`, `make gen-schema`, `make ci`. Be patient with cargo; never kill by PID.

---

## File map

- **Create** `crates/cogito-protocol/src/subagent.rs` — `BrainSpawner`, `DelegateRequest`, `SpawnError`.
- **Modify** `crates/cogito-protocol/src/exec_ctx.rs` — add `call_id`, `subagent_depth`, `brain_spawner`; hand-write `Debug`.
- **Modify** `crates/cogito-protocol/src/session.rs` — add `parent_session_id`, `parent_call_id`, `subagent_depth`.
- **Modify** `crates/cogito-protocol/src/stream.rs` — add `subagent_call_id` to forwarded variants.
- **Modify** `crates/cogito-protocol/src/lib.rs` — module + re-exports.
- **Create** `crates/cogito-core/src/runtime/subagent.rs` — `DelegateToolProvider`.
- **Modify** `crates/cogito-core/src/runtime/mod.rs` — export the module.
- **Modify** `crates/cogito-core/src/harness/dispatcher.rs` — set `ctx.call_id` per call.
- **Modify** `crates/cogito-core/src/runtime/session_loop.rs` — `SessionState.subagent_depth`, `SessionDeps.brain_spawner`, `ExecCtx` population, meta-override session-started.
- **Modify** `crates/cogito-core/src/runtime/builder.rs` — `impl BrainSpawner for Runtime`, `strategy_registry` field/setter, `open_inner` factoring.
- **Modify** `crates/cogito-cli/src/chat.rs` — register `delegate`, pass strategy registry, read `max_subagent_depth`.
- **Create** `crates/cogito-core/tests/subagent_delegate.rs` — acceptance + depth integration tests.
- **Modify** docs: ADR done; `ARCHITECTURE.md`, `ROADMAP.md`, `CHANGELOG.md`, `docs/schemas/conversation-event-v1.json`.

Convention reminder: all code comments in English; clippy denies `unwrap_used`/`expect_used`/`panic` (use `?`, `match`, `unwrap_or`); workspace deps via `{ workspace = true }`.

---

## Task 1: `BrainSpawner` trait + request/error types (protocol)

**Files:**
- Create: `crates/cogito-protocol/src/subagent.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Write the failing test** — append to the new file's bottom (write the whole file in Step 3; the test asserts object-safety and a mock impl).

The test lives in Step 3's file body (`mod tests`). Its intent: a `dyn BrainSpawner` is constructible and `DelegateRequest` builds.

- [ ] **Step 2: Create `subagent.rs`**

```rust
//! Subagent execution seam (ADR-0011, v0.2 S2 minimal).
//!
//! `BrainSpawner` is the layer-rule seam (ADR-0004): Hands cannot import
//! Runtime, so the ability to spawn a child Brain is a Protocol trait that
//! `cogito-core::runtime` implements and injects into every tool via
//! [`crate::ExecCtx::brain_spawner`]. v0.2 ships a single synchronous
//! `run_to_completion`; v0.3 grows the spawn/wait/cancel lifecycle
//! additively.

use crate::ids::SessionId;

/// Request to run a child agent to completion. `#[non_exhaustive]` so v0.3
/// can add fields (e.g. `handed_tools`) without breaking call sites.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DelegateRequest {
    /// Strategy name to resolve into the child's `HarnessStrategy` (role).
    pub role: String,
    /// The child's first user message — the only parent→child channel.
    pub input: String,
    /// Parent session id, recorded child-side for linkage.
    pub parent_session_id: SessionId,
    /// The `delegate` tool-call id in the parent turn, recorded child-side.
    pub parent_call_id: String,
    /// Parent's subagent depth; the child opens at `parent_depth + 1`.
    pub parent_depth: u32,
}

impl DelegateRequest {
    /// Build a request. Convenience for call sites and tests.
    #[must_use]
    pub fn new(
        role: impl Into<String>,
        input: impl Into<String>,
        parent_session_id: SessionId,
        parent_call_id: impl Into<String>,
        parent_depth: u32,
    ) -> Self {
        Self {
            role: role.into(),
            input: input.into(),
            parent_session_id,
            parent_call_id: parent_call_id.into(),
            parent_depth,
        }
    }
}

/// Failure modes of [`BrainSpawner::run_to_completion`]. The `delegate`
/// tool maps every variant to a `ToolResult::Error` (Inviolable #5).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpawnError {
    /// `role` did not resolve to a known strategy.
    #[error("unknown subagent role `{role}`")]
    UnknownRole {
        /// The unresolved role name.
        role: String,
    },
    /// Opening the child session failed (store/runtime error).
    #[error("failed to open subagent session: {reason}")]
    OpenFailed {
        /// Human-readable cause.
        reason: String,
    },
    /// The child turn ended in a terminal failure.
    #[error("subagent failed: {reason}")]
    ChildFailed {
        /// Human-readable rendering of the child `TurnFailureReason`.
        reason: String,
    },
}

/// The layer-rule seam that lets a tool spawn a child Brain. Implemented by
/// `cogito-core::runtime::Runtime`; injected as `Arc<dyn BrainSpawner>` via
/// `ExecCtx`. Brain and tools see only this trait.
#[async_trait::async_trait]
pub trait BrainSpawner: Send + Sync {
    /// Run a child agent to completion synchronously and return its final
    /// assistant text. The child is an independent top-level session; only
    /// the returned string crosses back to the caller.
    async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct MockSpawner;

    #[async_trait::async_trait]
    impl BrainSpawner for MockSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Ok(format!("ran {} with {}", req.role, req.input))
        }
    }

    #[tokio::test]
    async fn object_safe_and_invocable() {
        let spawner: Arc<dyn BrainSpawner> = Arc::new(MockSpawner);
        let req = DelegateRequest::new("reviewer", "check this", SessionId::new(), "c1", 0);
        let out = spawner.run_to_completion(req).await.expect("mock ok");
        assert_eq!(out, "ran reviewer with check this");
    }
}
```

- [ ] **Step 3: Wire module + re-exports** in `crates/cogito-protocol/src/lib.rs`

Add `pub mod subagent;` near the other `pub mod` lines (after `pub mod stream;`), and add to the re-export block:

```rust
pub use subagent::{BrainSpawner, DelegateRequest, SpawnError};
```

- [ ] **Step 4: Run tests**

Run: `make test CRATE=cogito-protocol`
Expected: PASS (new `object_safe_and_invocable` green; `async_trait` already a workspace dep — confirm `async_trait = { workspace = true }` is in `crates/cogito-protocol/Cargo.toml`; it is, since `ToolProvider` uses it).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/subagent.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): add BrainSpawner trait + DelegateRequest + SpawnError"
```

---

## Task 2: `ExecCtx` gains `call_id`, `subagent_depth`, `brain_spawner`

**Files:**
- Modify: `crates/cogito-protocol/src/exec_ctx.rs`
- Test: `crates/cogito-protocol/tests/exec_ctx.rs` (existing)

`brain_spawner` is `Option<Arc<dyn BrainSpawner>>`, which is not `Debug`, so the `#[derive(Debug)]` must become a hand-written impl.

- [ ] **Step 1: Write the failing test** — append to `crates/cogito-protocol/tests/exec_ctx.rs`

```rust
#[test]
fn open_ended_defaults_new_fields() {
    use cogito_protocol::ExecCtx;
    use cogito_protocol::ids::{SessionId, TurnId};
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    assert_eq!(ctx.subagent_depth, 0);
    assert!(ctx.call_id.is_none());
    assert!(ctx.brain_spawner.is_none());
    // Debug must not panic and must not try to print the spawner.
    let s = format!("{ctx:?}");
    assert!(s.contains("ExecCtx"));
}
```

(If `TurnId::new()` does not exist, use the same constructor the file already uses for `TurnId`; check the top of `exec_ctx.rs` test file.)

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-protocol`
Expected: FAIL — `no field subagent_depth` / `call_id` / `brain_spawner`.

- [ ] **Step 3: Edit `exec_ctx.rs`** — replace the struct definition and `open_ended`, and add a manual `Debug`.

Replace `use std::time::Instant;` block top with:

```rust
use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::ids::{SessionId, TurnId};
use crate::subagent::BrainSpawner;
```

Replace the `#[derive(Debug, Clone)] pub struct ExecCtx { ... }` with:

```rust
/// Per-invocation execution context. Tools and hooks receive this by value
/// and decide whether to honor `deadline` / `cancel`.
///
/// `Debug` is hand-written because `brain_spawner` holds a trait object that
/// is not `Debug`; the impl prints only whether a spawner is present.
#[derive(Clone)]
pub struct ExecCtx {
    /// Identifies the current session for correlation in logs and metrics.
    pub session_id: SessionId,
    /// Identifies the current turn within the session.
    pub turn_id: TurnId,
    /// The tool-call id for the current dispatch, set by H08 before
    /// `ToolProvider::invoke`. `None` outside a tool dispatch.
    pub call_id: Option<String>,
    /// Absolute wall-clock deadline. `None` means "no deadline".
    pub deadline: Option<Instant>,
    /// Cooperative cancellation token.
    pub cancel: CancellationToken,
    /// Subagent nesting depth of the current session (0 = top-level).
    /// `delegate` opens a child at `subagent_depth + 1`.
    pub subagent_depth: u32,
    /// Recursive Brain spawner (ADR-0011). `Some` when the Runtime wired a
    /// `BrainSpawner`; `None` otherwise (the `delegate` tool then errors).
    pub brain_spawner: Option<Arc<dyn BrainSpawner>>,
}

impl std::fmt::Debug for ExecCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecCtx")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("call_id", &self.call_id)
            .field("deadline", &self.deadline)
            .field("cancel", &self.cancel)
            .field("subagent_depth", &self.subagent_depth)
            .field("brain_spawner", &self.brain_spawner.is_some())
            .finish()
    }
}
```

Replace `open_ended` body to default the new fields:

```rust
    #[must_use]
    pub fn open_ended(session_id: SessionId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            turn_id,
            call_id: None,
            deadline: None,
            cancel: CancellationToken::new(),
            subagent_depth: 0,
            brain_spawner: None,
        }
    }
```

Also fix the stale module doc comment: change the line `//! a clone to each tool / hook call. v0.1 fields are minimal; v0.2 adds` + `//! \`storage: Arc<dyn StorageSystem>\` and v0.4 adds \`tenant\`.` to:

```rust
//! a clone to each tool / hook call. v0.2 adds `brain_spawner` +
//! `subagent_depth` + `call_id` (ADR-0011); storage moved to v0.5 and
//! `tenant` lands in v0.4.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make test CRATE=cogito-protocol`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/exec_ctx.rs crates/cogito-protocol/tests/exec_ctx.rs
git commit -m "feat(protocol): ExecCtx gains call_id, subagent_depth, brain_spawner"
```

---

## Task 3: `SessionMeta` parent↔child linkage fields

**Files:**
- Modify: `crates/cogito-protocol/src/session.rs`

- [ ] **Step 1: Write the failing test** — add to the `mod tests` in `session.rs`

```rust
    #[test]
    fn subagent_meta_roundtrips_and_defaults() -> serde_json::Result<()> {
        // Default top-level: new fields absent / zero.
        let base = SessionMeta {
            cogito_version: "0.2.0".into(),
            ..Default::default()
        };
        assert_eq!(base.subagent_depth, 0);
        assert!(base.parent_session_id.is_none());
        let json = serde_json::to_string(&base)?;
        // depth 0 + None parents are omitted.
        assert_eq!(json, r#"{"cogito_version":"0.2.0"}"#);

        // Child: fields populated and round-trip.
        let child = SessionMeta {
            cogito_version: "0.2.0".into(),
            strategy: Some("reviewer".into()),
            parent_session_id: Some(crate::ids::SessionId::new()),
            parent_call_id: Some("c1".into()),
            subagent_depth: 1,
            ..Default::default()
        };
        let back: SessionMeta = serde_json::from_str(&serde_json::to_string(&child)?)?;
        assert_eq!(back, child);
        Ok(())
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-protocol`
Expected: FAIL — unknown fields `parent_session_id` etc.

- [ ] **Step 3: Add the fields** — in `crates/cogito-protocol/src/session.rs`, inside `pub struct SessionMeta`, after `tenant_id`:

```rust
    /// Parent session id when this session is a subagent (ADR-0011).
    /// `Some` ⇒ this is a delegated child; `None` ⇒ top-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<crate::ids::SessionId>,

    /// The parent turn's `delegate` tool-call id that spawned this child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,

    /// Subagent nesting depth (0 = top-level, 1 = first delegate, ...).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub subagent_depth: u32,
```

Add a free helper near the bottom of the module (before `#[cfg(test)]`):

```rust
/// serde skip predicate: omit `subagent_depth` when it is the 0 default.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}
```

(`is_zero` keeps top-level session JSONL byte-identical to v0.1.)

- [ ] **Step 4: Run test to verify it passes**

Run: `make test CRATE=cogito-protocol`
Expected: PASS (including the existing `unknown_fields_in_json_do_not_panic`).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/session.rs
git commit -m "feat(protocol): SessionMeta parent_session_id/parent_call_id/subagent_depth"
```

---

## Task 4: `StreamEvent.subagent_call_id` attribution field

**Files:**
- Modify: `crates/cogito-protocol/src/stream.rs`
- Test: `crates/cogito-protocol/tests/stream_event.rs` (existing)

Add the attribution field to the variants the observability bridge forwards: `TurnStarted`, `TurnCompleted`, `TurnFailed`, `TextDelta`. (Unit variants `TurnStarted`/`TurnCompleted` become struct variants with one optional field.)

- [ ] **Step 1: Write the failing test** — append to `crates/cogito-protocol/tests/stream_event.rs`

```rust
#[test]
fn text_delta_carries_optional_subagent_call_id() -> serde_json::Result<()> {
    use cogito_protocol::stream::StreamEvent;
    // Absent by default: not serialized.
    let bare = StreamEvent::TextDelta { chunk: "hi".into(), subagent_call_id: None };
    let json = serde_json::to_string(&bare)?;
    assert!(!json.contains("subagent_call_id"), "omitted when None: {json}");
    // Present when tagged.
    let tagged = StreamEvent::TextDelta { chunk: "hi".into(), subagent_call_id: Some("c1".into()) };
    let back: StreamEvent = serde_json::from_str(&serde_json::to_string(&tagged)?)?;
    assert_eq!(back, tagged);
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-protocol`
Expected: FAIL — missing field `subagent_call_id`.

- [ ] **Step 3: Edit `stream.rs`** — change the four variants. For the two currently-unit variants:

```rust
    /// A new turn has begun.
    TurnStarted {
        /// Set when this event is forwarded from a subagent's stream,
        /// naming the parent `delegate` call. `None` for the parent's own
        /// turns. (ADR-0011 observability bridge.)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },
```

```rust
    /// The turn reached terminal Completed state.
    TurnCompleted {
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },
```

For `TurnFailed` add the field alongside `reason`; for `TextDelta` add it alongside `chunk`:

```rust
    TurnFailed {
        /// Human-readable description of the failure.
        reason: String,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },
```

```rust
    TextDelta {
        /// The text chunk emitted by the model.
        chunk: String,
        /// See `TurnStarted::subagent_call_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_call_id: Option<String>,
    },
```

- [ ] **Step 4: Fix all construction/match sites** — the compiler will list them. Search and update:

Run: `rg -n "StreamEvent::TurnStarted|StreamEvent::TurnCompleted|StreamEvent::TurnFailed|StreamEvent::TextDelta" crates/`

For each producer of `TurnStarted`/`TurnCompleted` add `subagent_call_id: None`; for `TurnFailed`/`TextDelta` add `subagent_call_id: None` next to existing fields. For each `match` arm that binds these (e.g. in `cogito-cli`/`cogito-tui` renderers, `step_recorder.rs`), add `, subagent_call_id: _` (or bind and ignore). Primary producers: `crates/cogito-core/src/harness/step_recorder.rs` (TextDelta, terminal events) and the session loop's terminal broadcasts.

- [ ] **Step 5: Run tests + build**

Run: `make test CRATE=cogito-protocol && make test CRATE=cogito-core`
Expected: PASS. Then `cargo build -p cogito-cli -p cogito-tui` clean (no missing-field errors).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(protocol): StreamEvent.subagent_call_id for subagent attribution"
```

---

## Task 5: Regenerate the conversation-event JSON Schema

**Files:**
- Modify: `docs/schemas/conversation-event-v1.json` (generated)

`SessionMeta` is part of `ConversationEvent` (the `SessionStarted` payload), so its new fields must land in the schema artifact, or the CI drift gate (`make gen-schema-check`) fails.

- [ ] **Step 1: Regenerate**

Run: `make gen-schema`
Expected: `docs/schemas/conversation-event-v1.json` updated with the new optional `SessionMeta` properties.

- [ ] **Step 2: Verify the drift gate passes**

Run: `make gen-schema-check`
Expected: exit 0 (no drift).

- [ ] **Step 3: Confirm no `SCHEMA_VERSION` bump**

Run: `rg -n "SCHEMA_VERSION" crates/cogito-protocol/src/event.rs`
Expected: unchanged value (additive per ADR-0007/0019).

- [ ] **Step 4: Commit**

```bash
git add docs/schemas/conversation-event-v1.json
git commit -m "chore(schema): regenerate for additive SessionMeta subagent fields"
```

---

## Task 6: `DelegateToolProvider` (core) + unit tests

**Files:**
- Create: `crates/cogito-core/src/runtime/subagent.rs`
- Modify: `crates/cogito-core/src/runtime/mod.rs`

- [ ] **Step 1: Create `subagent.rs` with the provider + a failing test**

```rust
//! `DelegateToolProvider` — the `delegate(role, input) → output` tool
//! (ADR-0011, v0.2 S2 minimal). Reads `ExecCtx.brain_spawner` and runs a
//! child agent to completion synchronously. No `Runtime` reference is held;
//! the spawner arrives per-call via `ExecCtx`.

use cogito_protocol::subagent::{DelegateRequest, SpawnError};
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use cogito_protocol::ExecCtx;

/// Tool name exposed to the model.
pub const DELEGATE_TOOL_NAME: &str = "delegate";

/// The `delegate` tool. Construct with [`DelegateToolProvider::new`].
pub struct DelegateToolProvider {
    /// Maximum subagent nesting depth (inclusive guard). Default 3.
    max_depth: u32,
}

impl DelegateToolProvider {
    /// Build with an explicit max depth.
    #[must_use]
    pub fn new(max_depth: u32) -> Self {
        Self { max_depth }
    }
}

impl Default for DelegateToolProvider {
    fn default() -> Self {
        Self { max_depth: 3 }
    }
}

#[derive(serde::Deserialize)]
struct DelegateArgs {
    role: String,
    input: String,
}

fn error(kind: ToolErrorKind, message: String) -> InvokeOutcome {
    InvokeOutcome::Sync(ToolResult::Error { kind, message, retryable: false })
}

#[async_trait::async_trait]
impl ToolProvider for DelegateToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: DELEGATE_TOOL_NAME.to_string(),
            description: "Delegate a self-contained subtask to a child agent \
                identified by `role` (a strategy name). The child starts with \
                a fresh context and sees NONE of this conversation, so pack \
                every file path, fact, and decision it needs into `input`. \
                Returns the child's final message as text."
                .to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "description": "Strategy name to run as." },
                    "input": { "type": "string", "description": "Self-contained task for the child." }
                },
                "required": ["role", "input"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        if name != DELEGATE_TOOL_NAME {
            return error(ToolErrorKind::InvocationFailed, format!("unknown tool `{name}`"));
        }
        let parsed: DelegateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return error(ToolErrorKind::InvalidArgs, format!("invalid delegate args: {e}")),
        };
        if ctx.subagent_depth >= self.max_depth {
            return error(
                ToolErrorKind::InvocationFailed,
                format!(
                    "subagent depth limit reached (depth {} >= max {})",
                    ctx.subagent_depth, self.max_depth
                ),
            );
        }
        let Some(spawner) = ctx.brain_spawner.clone() else {
            return error(
                ToolErrorKind::InvocationFailed,
                "subagent delegation is not available (no BrainSpawner wired)".to_string(),
            );
        };
        let call_id = ctx.call_id.clone().unwrap_or_default();
        let req = DelegateRequest::new(
            parsed.role,
            parsed.input,
            ctx.session_id,
            call_id,
            ctx.subagent_depth,
        );
        match spawner.run_to_completion(req).await {
            Ok(text) => InvokeOutcome::Sync(ToolResult::text(text)),
            Err(e) => map_spawn_error(&e),
        }
    }
}

fn map_spawn_error(e: &SpawnError) -> InvokeOutcome {
    error(ToolErrorKind::InvocationFailed, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::subagent::BrainSpawner;
    use std::sync::Arc;

    struct OkSpawner;
    #[async_trait::async_trait]
    impl BrainSpawner for OkSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Ok(format!("done:{}", req.role))
        }
    }
    struct UnknownRoleSpawner;
    #[async_trait::async_trait]
    impl BrainSpawner for UnknownRoleSpawner {
        async fn run_to_completion(&self, req: DelegateRequest) -> Result<String, SpawnError> {
            Err(SpawnError::UnknownRole { role: req.role })
        }
    }

    fn ctx_with(depth: u32, spawner: Option<Arc<dyn BrainSpawner>>) -> ExecCtx {
        let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
        c.subagent_depth = depth;
        c.brain_spawner = spawner;
        c.call_id = Some("c1".into());
        c
    }

    fn args(role: &str, input: &str) -> serde_json::Value {
        serde_json::json!({ "role": role, "input": input })
    }

    #[tokio::test]
    async fn happy_path_returns_child_text() {
        let p = DelegateToolProvider::new(3);
        let out = p.invoke("delegate", args("reviewer", "x"), ctx_with(0, Some(Arc::new(OkSpawner)))).await;
        match out {
            InvokeOutcome::Sync(ToolResult::Output(v)) => {
                assert_eq!(v, vec![serde_json::Value::String("done:reviewer".into())]);
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn depth_guard_blocks_at_max() {
        let p = DelegateToolProvider::new(3);
        let out = p.invoke("delegate", args("r", "x"), ctx_with(3, Some(Arc::new(OkSpawner)))).await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { kind: ToolErrorKind::InvocationFailed, .. })));
    }

    #[tokio::test]
    async fn missing_spawner_errors() {
        let p = DelegateToolProvider::new(3);
        let out = p.invoke("delegate", args("r", "x"), ctx_with(0, None)).await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { .. })));
    }

    #[tokio::test]
    async fn bad_args_are_invalid_args() {
        let p = DelegateToolProvider::new(3);
        let out = p.invoke("delegate", serde_json::json!({ "role": "r" }), ctx_with(0, Some(Arc::new(OkSpawner)))).await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { kind: ToolErrorKind::InvalidArgs, .. })));
    }

    #[tokio::test]
    async fn unknown_role_maps_to_error() {
        let p = DelegateToolProvider::new(3);
        let out = p.invoke("delegate", args("nope", "x"), ctx_with(0, Some(Arc::new(UnknownRoleSpawner)))).await;
        assert!(matches!(out, InvokeOutcome::Sync(ToolResult::Error { .. })));
    }
}
```

- [ ] **Step 2: Export the module** — in `crates/cogito-core/src/runtime/mod.rs`, add:

```rust
pub mod subagent;
pub use subagent::{DelegateToolProvider, DELEGATE_TOOL_NAME};
```

- [ ] **Step 3: Run tests**

Run: `make test CRATE=cogito-core`
Expected: PASS — five `subagent::tests` green.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/src/runtime/subagent.rs crates/cogito-core/src/runtime/mod.rs
git commit -m "feat(core): DelegateToolProvider with depth guard + spawner-via-ExecCtx"
```

---

## Task 7: Dispatcher sets `ctx.call_id`; session loop threads spawner + depth

**Files:**
- Modify: `crates/cogito-core/src/harness/dispatcher.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs`

This makes `ctx.call_id` populated for `invoke`, and gives the per-turn `ExecCtx` a `brain_spawner` + `subagent_depth`.

- [ ] **Step 1: Dispatcher — set call_id before invoke.** In `crates/cogito-core/src/harness/dispatcher.rs`, replace the invoke block (around lines 75-80):

```rust
    let name = inv.name.clone();
    let call_id = inv.call_id.clone();
    let args = inv.args.clone();
    let mut ctx = ctx;
    ctx.call_id = Some(call_id.clone());
    let caught = AssertUnwindSafe(provider.invoke(&name, args, ctx))
        .catch_unwind()
        .await;
```

- [ ] **Step 2: `SessionState` gains `subagent_depth`; `SessionDeps` gains `brain_spawner`.** In `crates/cogito-core/src/runtime/session_loop.rs`:

In `pub(super) struct SessionState` add:
```rust
    /// Subagent nesting depth of this session (0 = top-level). Read from
    /// the seq=0 `SessionMeta` at open; flowed into each turn's `ExecCtx`.
    pub(super) subagent_depth: u32,
```

In `pub(super) struct SessionDeps` add (alongside `model`, `tools`, `job_mgr`):
```rust
    /// Recursive Brain spawner injected per-turn into `ExecCtx`. `None`
    /// when the Runtime had no spawner (subagent disabled).
    pub(super) brain_spawner: Option<std::sync::Arc<dyn cogito_protocol::subagent::BrainSpawner>>,
```

- [ ] **Step 3: Populate `ExecCtx` in `spawn_turn_driver`.** Replace the `let exec_ctx = ExecCtx { ... };` block (session_loop.rs:463) with:

```rust
    let exec_ctx = ExecCtx {
        session_id: state.session_id,
        turn_id,
        call_id: None,
        deadline: None,
        cancel: new_token,
        subagent_depth: state.subagent_depth,
        brain_spawner: deps.brain_spawner.clone(),
    };
```

- [ ] **Step 4: Run build to find SessionState/SessionDeps construction gaps**

Run: `cargo build -p cogito-core 2>&1 | head -40`
Expected: errors at the two literal construction sites (in `builder.rs::open_session` for `SessionState`/`SessionDeps`). Those are fixed in Task 8 — for now confirm the only errors are missing-field on those two literals.

- [ ] **Step 5: Commit (after Task 8 makes it compile)** — defer the commit; Task 8 completes the wiring.

---

## Task 8: `Runtime: impl BrainSpawner`, `strategy_registry`, child-open path

**Files:**
- Modify: `crates/cogito-core/src/runtime/builder.rs`

- [ ] **Step 1: Add the `strategy_registry` field + setter.** In `struct Runtime` add:

```rust
    /// Optional strategy registry, used by the subagent spawner to resolve
    /// a `delegate` role into a child `HarnessStrategy`. `None` ⇒ delegate
    /// returns `SpawnError::UnknownRole`.
    strategy_registry: Option<Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>>,
```

In `struct RuntimeBuilder` add `strategy_registry: Option<Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>>,` and a setter:

```rust
    /// Inject a strategy registry so the subagent `delegate` tool can
    /// resolve roles. Optional — without it, `delegate` errors on any role.
    #[must_use]
    pub fn strategy_registry(
        mut self,
        registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>,
    ) -> Self {
        self.strategy_registry = Some(registry);
        self
    }
```

In `build()` add `strategy_registry: self.strategy_registry,` to the `Runtime { ... }` literal.

- [ ] **Step 2: Factor `open_session` into `open_inner`.** Rename the current method body to a private helper that accepts the per-session strategy, an optional `SessionMeta` override, the depth, and a `register` flag. Public `open_session` delegates:

```rust
    pub async fn open_session(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        let strategy = self.strategy.clone();
        self.open_inner(id, mode, strategy, None, 0, true).await
    }
```

Change the existing method signature to:

```rust
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn open_inner(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
        strategy: HarnessStrategy,
        meta_override: Option<cogito_protocol::SessionMeta>,
        subagent_depth: u32,
        register: bool,
    ) -> Result<SessionHandle, RuntimeError> {
```

Inside, make these edits to the moved body:
- Use the parameter `strategy` instead of `self.strategy` everywhere it appears (`record_session_started`, `context_pipeline` build, `SessionState.strategy`).
- Replace the `record_session_started(&recorder, id, &self.strategy)` call with a meta-aware variant:

```rust
        if !session_exists {
            let meta = meta_override.unwrap_or_else(|| cogito_protocol::SessionMeta {
                cogito_version: env!("CARGO_PKG_VERSION").into(),
                strategy: Some(strategy.name.clone()),
                model: Some(strategy.model_params.model.clone()),
                ..Default::default()
            });
            let mut rec = recorder.lock().await;
            if let Err(e) = rec.record_session_started(meta).await {
                tracing::error!(session_id = %id, error = %e, "failed to record SessionStarted");
            }
        }
```

(Drop the `record_session_started` import from session_loop if now unused, or keep the helper for top-level callers — simplest: delete the import line and the now-dead helper usage; the helper fn can stay.)

- Set `SessionState.subagent_depth: subagent_depth`.
- Build `SessionDeps.brain_spawner` via the `RuntimeSpawner` newtype defined in Step 3 (see Step 4 for the exact line).
- Guard the registry insert/contains with `register`:

```rust
        if register && self.sessions.contains_key(&id) {
            return Err(RuntimeError::SessionAlreadyOpen { id });
        }
```
and at the end:
```rust
        if register {
            self.sessions.insert(id, handle.clone());
        }
```

- [ ] **Step 3: Implement the spawner via a `RuntimeSpawner` newtype.** The trait method needs `&Arc<Runtime>` (for `open_inner`), so wrap the owning Arc in a newtype and implement `BrainSpawner` on it; the injected trait object is a single `Arc<RuntimeSpawner>`. Add at the bottom of `builder.rs`:

```rust
/// Owns an `Arc<Runtime>` so `run_to_completion` has the Arc that
/// `open_inner` needs (and so a spawned child can itself delegate).
pub(crate) struct RuntimeSpawner(pub(crate) Arc<Runtime>);

#[async_trait::async_trait]
impl cogito_protocol::subagent::BrainSpawner for RuntimeSpawner {
    async fn run_to_completion(
        &self,
        req: cogito_protocol::subagent::DelegateRequest,
    ) -> Result<String, cogito_protocol::subagent::SpawnError> {
        use cogito_protocol::stream::StreamEvent;
        use cogito_protocol::subagent::SpawnError;
        use futures::TryStreamExt as _;

        let rt = &self.0; // &Arc<Runtime>

        // 1. Resolve the role -> child strategy.
        let registry = rt
            .strategy_registry
            .as_ref()
            .ok_or_else(|| SpawnError::UnknownRole { role: req.role.clone() })?;
        let strategy = registry
            .get(&req.role)
            .map_err(|_| SpawnError::UnknownRole { role: req.role.clone() })?;

        // 2. Child SessionMeta (linkage recorded child-side only). Clone
        //    parent_call_id so the optional bridge (Task 9) can still use it.
        let child_id = SessionId::new();
        let meta = cogito_protocol::SessionMeta {
            cogito_version: env!("CARGO_PKG_VERSION").into(),
            strategy: Some(strategy.name.clone()),
            model: Some(strategy.model_params.model.clone()),
            parent_session_id: Some(req.parent_session_id),
            parent_call_id: Some(req.parent_call_id.clone()),
            subagent_depth: req.parent_depth + 1,
            ..Default::default()
        };

        // 3. Open the child as an unregistered top-level session.
        let child = rt
            .open_inner(child_id, OpenMode::New, strategy, Some(meta), req.parent_depth + 1, false)
            .await
            .map_err(|e| SpawnError::OpenFailed { reason: e.to_string() })?;

        // 4. Drive to a terminal turn via the broadcast stream.
        let mut rx = child.subscribe();
        child
            .submit_user_text(req.input)
            .await
            .map_err(|e| SpawnError::OpenFailed { reason: e.to_string() })?;
        let mut failure: Option<String> = None;
        loop {
            match rx.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => break,
                Ok(StreamEvent::TurnFailed { reason, .. }) => { failure = Some(reason); break; }
                Ok(_) => {} // intermediate (paused/resumed/deltas) — keep waiting
                Err(_) => break, // lagged/closed — fall through to log replay
            }
        }

        // 5. Tear the child actor down.
        let _ = child.shutdown(std::time::Duration::from_secs(5)).await;
        if let Some(reason) = failure {
            return Err(SpawnError::ChildFailed { reason });
        }

        // 6. Extract the final assistant text from the child log.
        let events: Vec<cogito_protocol::ConversationEvent> = rt
            .store
            .replay(child_id, 0)
            .try_collect()
            .await
            .map_err(|e| SpawnError::OpenFailed { reason: e.to_string() })?;
        Ok(last_assistant_text(&events).unwrap_or_default())
    }
}

/// Walk events newest-first; return the last non-empty assistant message
/// text. `EventPayload::AssistantMessageAppended { text }` is the flat-text
/// shape verified in `crates/cogito-protocol/src/event.rs:92`.
fn last_assistant_text(events: &[cogito_protocol::ConversationEvent]) -> Option<String> {
    use cogito_protocol::event::EventPayload;
    events.iter().rev().find_map(|ev| match &ev.payload {
        EventPayload::AssistantMessageAppended { text } if !text.is_empty() => Some(text.clone()),
        _ => None,
    })
}
```

- [ ] **Step 4: Inject `RuntimeSpawner` into per-turn deps.** This resolves Task 7's `SessionDeps.brain_spawner` field. In `open_inner` (which is `self: &Arc<Self>`), build the deps spawner:

```rust
    brain_spawner: Some(
        Arc::new(RuntimeSpawner(Arc::clone(self)))
            as Arc<dyn cogito_protocol::subagent::BrainSpawner>
    ),
```

Every session (top-level and child) therefore carries a spawner, so a child can itself `delegate` up to the depth limit. No `clone_arc`, no `impl … for Arc<Runtime>` — the newtype holds the Arc.

- [ ] **Step 5: Build + run core tests**

Run: `make test CRATE=cogito-core`
Expected: PASS (existing runtime/session tests unaffected; `SessionState`/`SessionDeps` literals now compile).

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/harness/dispatcher.rs crates/cogito-core/src/runtime/session_loop.rs crates/cogito-core/src/runtime/builder.rs
git commit -m "feat(core): Runtime impl BrainSpawner + strategy_registry + child-open path"
```

---

## Task 9: Observability bridge (forward child stream to parent) — OPTIONAL/ISOLATED

**Files:**
- Modify: `crates/cogito-core/src/runtime/builder.rs`

This task is self-contained and may be dropped (then child progress is visible only via the child's own JSONL). It re-emits the child's `StreamEvent`s onto the parent's broadcast tagged with the `delegate` call id.

- [ ] **Step 1: Look up the parent broadcast sender.** Inside `run_to_completion`, before the drive loop, capture the parent sender if the parent session is registered:

```rust
        // `rt` is `&Arc<Runtime>` from Step 3 (`let rt = &self.0;`).
        // `req.parent_call_id` is still available — Step 3's `meta` cloned it.
        let parent_tx = rt
            .sessions
            .get(&req.parent_session_id)
            .map(|h| h.shared.events_tx.clone());
        let bridge_call_id = req.parent_call_id.clone();
```

- [ ] **Step 2: Tag + forward in the drive loop.** Replace the `Ok(_) => {}` arm and add tagging on terminal events:

```rust
                Ok(ev) => {
                    if let Some(tx) = &parent_tx {
                        let _ = tx.send(tag_subagent(ev.clone(), &bridge_call_id));
                    }
                    if matches!(ev, StreamEvent::TurnCompleted { .. }) { break; }
                    if let StreamEvent::TurnFailed { reason, .. } = &ev {
                        failure = Some(reason.clone());
                        break;
                    }
                }
```

(Replace the earlier explicit `TurnCompleted`/`TurnFailed` arms with this single `Ok(ev)` arm to avoid double-handling.)

- [ ] **Step 3: Add the tagging helper** near `last_assistant_text`:

```rust
/// Stamp `subagent_call_id` onto the forwarded events that carry it; pass
/// other variants through unchanged. Broadcast-only — never persisted.
fn tag_subagent(ev: cogito_protocol::stream::StreamEvent, call_id: &str) -> cogito_protocol::stream::StreamEvent {
    use cogito_protocol::stream::StreamEvent as S;
    match ev {
        S::TextDelta { chunk, .. } => S::TextDelta { chunk, subagent_call_id: Some(call_id.to_string()) },
        S::TurnStarted { .. } => S::TurnStarted { subagent_call_id: Some(call_id.to_string()) },
        S::TurnCompleted { .. } => S::TurnCompleted { subagent_call_id: Some(call_id.to_string()) },
        S::TurnFailed { reason, .. } => S::TurnFailed { reason, subagent_call_id: Some(call_id.to_string()) },
        other => other,
    }
}
```

- [ ] **Step 4: Run core tests**

Run: `make test CRATE=cogito-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/runtime/builder.rs
git commit -m "feat(core): live observability bridge forwards child stream tagged with call id"
```

---

## Task 10: CLI wiring (`cogito chat`)

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs`

- [ ] **Step 1: Register the `delegate` provider in the composite.** In the tool-composition fn (around line 462-530), after building `builtin`/`run_tests`/`bash` and before the `CompositeToolProvider::new(vec![...], NamingPolicy::Strict)` call, add:

```rust
    use cogito_core::runtime::DelegateToolProvider;
    let max_depth = cfg
        .tools
        .as_ref()
        .and_then(|t| t.max_subagent_depth)
        .unwrap_or(3);
    let delegate: Arc<dyn ToolProvider> = Arc::new(DelegateToolProvider::new(max_depth));
```

and include `delegate` in the local composite vec:

```rust
        CompositeToolProvider::new(vec![builtin, run_tests, bash, delegate], NamingPolicy::Strict)
```

(If `cfg.tools` / `max_subagent_depth` does not exist, default to `3` inline and add the config field per Step 2; otherwise skip Step 2.)

- [ ] **Step 2 (if needed): Config field.** In `crates/cogito-config/src/types.rs`, add `max_subagent_depth: Option<u32>` to the `[tools]` config struct (mirror an existing optional field; `#[serde(default)]`). Skip if a suitable knob already exists.

- [ ] **Step 3: Pass the strategy registry to the Runtime.** Where the `Runtime::builder()` chain is built (around line 608), the CLI already has the `registry` it used for `resolve_strategy`. Add:

```rust
        .strategy_registry(registry.clone())
```

to the builder chain (registry is `Arc<dyn StrategyRegistry>` or wrap with `Arc::new` / `Arc::clone` to match its existing type).

- [ ] **Step 4: Build + smoke**

Run: `cargo build -p cogito-cli`
Expected: clean. `delegate` now appears in the tool surface.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-cli/src/chat.rs crates/cogito-config/src/types.rs
git commit -m "feat(cli): wire delegate tool + strategy registry into chat runtime"
```

---

## Task 11: Integration test (acceptance) + depth E2E

**Files:**
- Create: `crates/cogito-core/tests/subagent_delegate.rs`

Uses a shared `MockModelGateway` scripted in global FIFO order (every `stream()` call across parent and child sessions pops the next script) and an in-test `StrategyRegistry`. Mirrors the harness in `crates/cogito-core/tests/runtime_submit.rs`. JSONL layout assumption: `JsonlStore` writes `<root>/sessions/<session_id>.jsonl`; if the implementer finds a flat or different layout, adjust the scan in `find_child_meta`.

- [ ] **Step 1: Write the file with shared helpers + the acceptance test**

```rust
//! Sprint 11 acceptance + depth integration tests for `delegate`.

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{DelegateToolProvider, OpenMode, Runtime};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::SessionMeta;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolProvider, ToolResult};
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, CompositeToolProvider, NamingPolicy, ReadFile};
use futures::StreamExt as _;

/// In-test registry: every requested role resolves to a mock-model strategy
/// named after the role, allowing all tools (so `delegate` is surfaced).
struct TestRegistry;
impl StrategyRegistry for TestRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        let mut s = HarnessStrategy::default_with_model("mock");
        s.name = name.to_string();
        s.allowed_tools = ToolFilter::All;
        Ok(s)
    }
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
}

/// One scripted tool_use turn emitting `delegate{role, input}` then ToolUse stop.
fn script_delegate_call(mock: &MockModelGateway, role: &str, input: &str) {
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted { block_index: 0, call_id: "c1".into(), tool_name: "delegate".into() },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "delegate".into(),
            args: serde_json::json!({ "role": role, "input": input }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 1, output_tokens: 1 },
        },
    ]);
}

/// One scripted text turn emitting `text` then EndTurn.
fn script_text(mock: &MockModelGateway, text: &str) {
    mock.push_reply(vec![
        ModelEvent::TextDelta { block_index: 0, chunk: text.into() },
        ModelEvent::TextBlockCompleted { block_index: 0, text: text.into() },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 1, output_tokens: 1 },
        },
    ]);
}

fn build_runtime(
    store: &Arc<JsonlStore>,
    mock: Arc<MockModelGateway>,
    max_depth: u32,
) -> Arc<Runtime> {
    let builtin: Arc<dyn ToolProvider> =
        Arc::new(BuiltinToolProvider::builder().with_tool(Arc::new(ReadFile)).build());
    let delegate: Arc<dyn ToolProvider> = Arc::new(DelegateToolProvider::new(max_depth));
    let tools: Arc<dyn ToolProvider> =
        Arc::new(CompositeToolProvider::new(vec![builtin, delegate], NamingPolicy::Strict));
    let mut parent_strategy = HarnessStrategy::default_with_model("mock");
    parent_strategy.allowed_tools = ToolFilter::All;
    Runtime::builder()
        .store(Arc::clone(store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(tools)
        .strategy(parent_strategy)
        .strategy_registry(Arc::new(TestRegistry) as Arc<dyn StrategyRegistry>)
        .build()
        .expect("runtime builds")
}

/// Wait for the PARENT's own terminal turn (untagged; subagent-forwarded
/// terminal events carry `Some(call_id)` and are ignored).
async fn wait_parent_done(rx: &mut tokio::sync::broadcast::Receiver<StreamEvent>) -> bool {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Ok(StreamEvent::TurnCompleted { subagent_call_id: None }) => return true,
                Ok(StreamEvent::TurnFailed { subagent_call_id: None, .. }) => return false,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

async fn replay_all(store: &Arc<JsonlStore>, id: SessionId) -> Vec<cogito_protocol::ConversationEvent> {
    let mut s = store.replay(id, 0);
    let mut out = Vec::new();
    while let Some(ev) = s.next().await {
        out.push(ev.expect("event decodes"));
    }
    out
}

/// Scan `<root>/sessions/*.jsonl`, skipping `parent_id`, and return the first
/// child session's `SessionMeta` (its seq=0 `SessionStarted`).
fn find_child_meta(root: &std::path::Path, parent_id: SessionId) -> Option<SessionMeta> {
    let dir = root.join("sessions");
    for entry in std::fs::read_dir(&dir).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(parent_id.to_string().as_str()) {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let first = content.lines().next()?;
        let ev: cogito_protocol::ConversationEvent = serde_json::from_str(first).ok()?;
        if let EventPayload::SessionStarted { meta } = ev.payload {
            return Some(meta);
        }
    }
    None
}

/// Scan every session log for a `ToolResultRecorded` Error whose message
/// mentions the depth limit.
fn any_depth_error(root: &std::path::Path) -> bool {
    let dir = root.join("sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else { return false };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for line in content.lines() {
            if let Ok(ev) = serde_json::from_str::<cogito_protocol::ConversationEvent>(line) {
                if let EventPayload::ToolResultRecorded { result: ToolResult::Error { message, .. }, .. } = ev.payload {
                    if message.contains("depth") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[tokio::test]
async fn delegate_runs_child_and_returns_final_text() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    // Global FIFO order: parent#1 (delegate) -> child (CHILD-DONE) -> parent#2.
    script_delegate_call(&mock, "reviewer", "go");
    script_text(&mock, "CHILD-DONE");
    script_text(&mock, "parent done");

    let runtime = build_runtime(&store, mock, 3);
    let parent_id = SessionId::new();
    let handle = runtime.open_session(parent_id, OpenMode::New).await?;
    let mut rx = handle.subscribe();
    handle.submit_user_text("please review").await?;
    assert!(wait_parent_done(&mut rx).await, "parent turn did not complete");
    let _ = handle.shutdown(Duration::from_secs(5)).await;

    // The parent received the child's verbatim final text as the tool result.
    let parent_events = replay_all(&store, parent_id).await;
    let result = parent_events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { call_id, result } if call_id == "c1" => Some(result.clone()),
            _ => None,
        })
        .expect("delegate ToolResult recorded in parent log");
    match result {
        ToolResult::Output(v) => assert_eq!(v, vec![serde_json::Value::String("CHILD-DONE".into())]),
        other => panic!("expected Output, got {other:?}"),
    }

    // A separate child session exists, linked child-side via SessionMeta.
    let child_meta = find_child_meta(tmp.path(), parent_id).expect("child session file");
    assert_eq!(child_meta.parent_session_id, Some(parent_id));
    assert_eq!(child_meta.parent_call_id.as_deref(), Some("c1"));
    assert_eq!(child_meta.subagent_depth, 1);
    assert_eq!(child_meta.strategy.as_deref(), Some("reviewer"));
    Ok(())
}
```

- [ ] **Step 2: Append the depth E2E test** to the same file

```rust
#[tokio::test]
async fn delegate_recursion_stops_at_max_depth() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    // max_depth = 2. Recursion: parent(0) -> child(1) -> grandchild(2)=BLOCKED.
    // Global FIFO order across all sessions:
    script_delegate_call(&mock, "looper", "x"); // parent#1  (depth 0, spawns child)
    script_delegate_call(&mock, "looper", "x"); // child#1   (depth 1, spawns grandchild)
    script_delegate_call(&mock, "looper", "x"); // grandchild#1 (depth 2 -> ToolResult::Error)
    script_text(&mock, "gc-stop"); // grandchild#2 (ends)
    script_text(&mock, "c-stop"); //  child#2      (ends)
    script_text(&mock, "p-stop"); //  parent#2     (ends)

    let runtime = build_runtime(&store, mock, 2);
    let parent_id = SessionId::new();
    let handle = runtime.open_session(parent_id, OpenMode::New).await?;
    let mut rx = handle.subscribe();
    handle.submit_user_text("recurse").await?;
    // Terminates (no infinite recursion) within the timeout.
    assert!(wait_parent_done(&mut rx).await, "recursion did not terminate");
    let _ = handle.shutdown(Duration::from_secs(5)).await;

    // Some level hit the depth guard.
    assert!(any_depth_error(tmp.path()), "expected a depth-limit ToolResult::Error in some session log");
    Ok(())
}
```

> Note: only the **registered** top-level parent receives forwarded child
> events (children open with `register = false`), so `wait_parent_done`
> only ever returns on the parent's own untagged terminal event. If
> `default_with_model` does not default `max_turns` high enough for the
> tool→re-call loop, raise it in `build_runtime`'s `parent_strategy`.

- [ ] **Step 3: Run the tests**

Run: `make test CRATE=cogito-core`
Expected: PASS (both new integration tests green).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/tests/subagent_delegate.rs
git commit -m "test(core): subagent delegate acceptance + depth-limit integration tests"
```

---

## Task 12: Docs, roadmap, changelog, architecture

**Files:**
- Modify: `ARCHITECTURE.md`, `ROADMAP.md`, `CHANGELOG.md`, `docs/components/H08-tool-dispatcher.md`

- [ ] **Step 1: ARCHITECTURE.md** — in the trait-contract table, update the `ExecCtx` row (now adds `brain_spawner` + `subagent_depth` + `call_id` in v0.2; storage v0.5) and the `BrainSpawner` row (v0.2 shipped, sync run-to-completion). In §"Subagent layer", add a short "v0.2 minimal (shipped)" note pointing at ADR-0011 and the spec.

- [ ] **Step 2: ROADMAP.md** — tick the Sprint 11 checklist boxes that are now done (ADR-0011 amendment, no-new-crate module, BrainSpawner trait, Runtime impl, DelegateToolProvider, strategy YAML role mapping, integration test). Leave any genuinely-unshipped item unchecked with a note.

- [ ] **Step 3: CHANGELOG.md** — add an entry under the v0.2 / Unreleased section listing the public-API additions: `BrainSpawner`/`DelegateRequest`/`SpawnError`, `ExecCtx` fields, `SessionMeta` fields, `StreamEvent.subagent_call_id`, `DelegateToolProvider`, `RuntimeBuilder::strategy_registry`.

- [ ] **Step 4: H08 doc** — note that `delegate` is an `AlwaysSync` tool and that the dispatcher now populates `ExecCtx.call_id` before `invoke`.

- [ ] **Step 5: Commit**

```bash
git add ARCHITECTURE.md ROADMAP.md CHANGELOG.md docs/components/H08-tool-dispatcher.md
git commit -m "docs(sprint-11): reconcile ARCHITECTURE/ROADMAP/CHANGELOG/H08 with shipped subagent"
```

---

## Task 13: Final gate

- [ ] **Step 1: Full local CI**

Run: `make ci`
Expected: fmt-check + clippy (with `-Dwarnings`) + layer-check + test all green. The layer-check must confirm `cogito-core::harness` still imports only `cogito-protocol` (the spawner reaches Brain only via `ExecCtx`).

- [ ] **Step 2: Chaos sanity (optional for v0.2)**

Run: `make chaos`
Expected: existing scenarios still pass. (A dedicated `subagent_*` chaos scenario is a v0.3 item per the spec — do not add one here.)

- [ ] **Step 3: Push the branch**

```bash
git push github feat/sprint-11-subagent
```

(Push to `github` only; the user syncs GitLab manually.)

---

## Self-review notes (for the implementer)

- **One fact to verify against source before coding**: whether a `[tools]` config struct with `max_subagent_depth` already exists (Task 10) — fallback is a hard-coded `3`. (The assistant-text shape is confirmed: `EventPayload::AssistantMessageAppended { text }`, flat string, at `event.rs:92`.)
- **The `Arc<Runtime>` self-type** in `BrainSpawner` is the one non-mechanical wiring decision — resolved by the `RuntimeSpawner(Arc<Runtime>)` newtype (Task 8 Step 3): the trait is implemented on the newtype, which owns the Arc the method needs, and the injected trait object is `Arc<RuntimeSpawner>`. No `impl … for Arc<Runtime>`, no `clone_arc`.
- **Task 9 is droppable** — if scope tightens, skip it; the core delegate works without the observability bridge (child progress then lives only in the child JSONL). If dropped, `wait_parent_done` still works because the parent's own terminal event is untagged regardless.
- **No `#[ignore]`, no `unwrap`/`expect`/`panic` in non-test code** (clippy denies them; tests may use `expect`).

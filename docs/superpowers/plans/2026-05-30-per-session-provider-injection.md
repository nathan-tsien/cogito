# Per-session Provider Injection (ADR-0028) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a single `Runtime` give each session its own tool/skill/strategy surface at open time, and let that surface change mid-session (effective next turn), with resume re-supplying the surface.

**Architecture:** Add a `SessionSpec` value type carrying optional per-session provider overrides (`None` → fall back to the Runtime build-time default). `Runtime::open_session_with(id, mode, spec)` becomes the real entry; `open_session` delegates with an empty spec. Per-session providers become *mutable session state*: a new `SessionCommand::UpdateSession(spec)` swaps the live Arcs, picked up at the next turn boundary because `TurnDeps` is rebuilt per turn. The Brain (H01–H11) is untouched. Provider identity is never persisted; on resume the caller passes the current spec again.

**Tech Stack:** Rust 2024, tokio actor model, `cargo nextest`. Crates touched: `cogito-core` only.

**Plan scope:** Plan 1 of 2 for Sprint 12. Plan 2 (plugin loader, ADR-0021) builds on `open_session_with` / `update_session` delivered here. This plan ends green and testable on its own.

**Authoritative design:** [ADR-0028](../../adr/0028-per-session-provider-injection.md) + [Sprint 12 spec](../specs/2026-05-30-sprint-12-saas-session-plugin-design.md) §3.

**Ground-truth notes (verified against source 2026-05-30):**
- `SessionCommand` lives in `crates/cogito-core/src/runtime/types.rs`, is `#[non_exhaustive]`, derives `Debug`, and its variants are `Trigger` / `JobCompleted` / `InternalCancel` / `Shutdown` / `CancelJob` / `SnapshotInFlight`.
- `open_inner` signature today: `(self: &Arc<Self>, id, mode, strategy, meta_override, subagent_depth, register)`. Two call sites: `Runtime::open_session` (builder.rs ~line 84) and `RuntimeSpawner::run_to_completion` (builder.rs ~line 543).
- `run_session(mut state, mut mailbox_rx, mailbox_tx, deps, initial_events)` — no `resume_decision` param. `deps` is currently immutable.
- `SessionHandle` talks to the actor via `self.shared.mailbox_tx`; the error variant is `SessionError::SessionClosed { session_id }`.
- Tests are integration tests under `crates/cogito-core/tests/` (there is no `builder_tests.rs`). `runtime_submit.rs` is the canonical pattern (real `MockModelGateway` + `JsonlStore` + `BuiltinToolProvider`).
- `EventPayload::SessionStarted { meta, strategy_name }` is at seq 0; `store.replay(id, 0)` yields **seq > 0 only**, so seq-0 meta is not assertable via `replay`. Tests therefore assert behavior (turn completes) rather than reading back `SessionMeta`.

---

## File structure

| File | Responsibility | Action |
|---|---|---|
| `crates/cogito-core/src/runtime/session_spec.rs` | `SessionSpec` value type + manual `Debug` + derived `Default`/`Clone` | Create |
| `crates/cogito-core/src/runtime/mod.rs` | Declare + export `SessionSpec` | Modify |
| `crates/cogito-core/src/runtime/types.rs` | `SessionCommand::UpdateSession` variant | Modify |
| `crates/cogito-core/src/runtime/builder.rs` | `open_session_with`; `open_session` delegates; `open_inner` consumes spec; meta stamping; fix spawner call site | Modify |
| `crates/cogito-core/src/runtime/session_loop.rs` | `run_session(mut deps)`; intercept `UpdateSession`; `apply_session_update` | Modify |
| `crates/cogito-core/src/runtime/handle.rs` | `SessionHandle::update_session` | Modify |
| `crates/cogito-core/tests/per_session_injection.rs` | New integration tests | Create |
| `crates/cogito-core/tests/resume_chaos.rs` | New `session_spec_mutated_then_resume` scenario | Modify |

---

## Task 1: `SessionSpec` value type

**Files:**
- Create: `crates/cogito-core/src/runtime/session_spec.rs`
- Modify: `crates/cogito-core/src/runtime/mod.rs`

- [ ] **Step 1: Create the `SessionSpec` type**

Create `crates/cogito-core/src/runtime/session_spec.rs`:

```rust
//! Per-session provider overrides.
//!
//! A `SessionSpec` carries optional overrides applied to one session at
//! open time (`Runtime::open_session_with`) or mid-session
//! (`SessionHandle::update_session`). A `None` field falls back to the
//! Runtime's build-time default provider. See ADR-0028.

use std::sync::Arc;

use cogito_protocol::skill::SkillProvider;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::tool::ToolProvider;

/// Optional per-session provider overrides.
///
/// `Default` is all-`None`, which makes `open_session_with(id, mode,
/// SessionSpec::default())` behave exactly like the legacy
/// `open_session(id, mode)`.
#[derive(Default, Clone)]
pub struct SessionSpec {
    /// Per-session tool provider. `None` → Runtime default.
    pub tools: Option<Arc<dyn ToolProvider>>,
    /// Per-session skill provider. `None` → Runtime default.
    pub skills: Option<Arc<dyn SkillProvider>>,
    /// Per-session strategy. `None` → Runtime default.
    pub strategy: Option<HarnessStrategy>,
    /// Tenant identity stamped into `SessionMeta` at open. Ignored by
    /// `update_session` (identity is fixed at open time).
    pub tenant_id: Option<String>,
    /// User identity stamped into `SessionMeta` at open. Ignored by
    /// `update_session`.
    pub user_id: Option<String>,
}

impl std::fmt::Debug for SessionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionSpec")
            .field("tools", &self.tools.is_some())
            .field("skills", &self.skills.is_some())
            .field("strategy", &self.strategy.as_ref().map(|s| &s.name))
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .finish()
    }
}
```

- [ ] **Step 2: Declare and export the module**

In `crates/cogito-core/src/runtime/mod.rs`, add `mod session_spec;` after the existing `pub mod ...;` block (it does not need to be public; the type is re-exported below), and add the re-export next to the existing `pub use types::...;` line:

```rust
mod session_spec;
```

```rust
pub use session_spec::SessionSpec;
```

- [ ] **Step 3: Verify it compiles**

Run: `make test CRATE=cogito-core`
Expected: builds clean; existing tests still PASS (nothing references `SessionSpec` yet, but the module must compile).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/src/runtime/session_spec.rs crates/cogito-core/src/runtime/mod.rs
git commit -m "feat(core): add SessionSpec per-session provider override type (ADR-0028)"
```

---

## Task 2: `open_session_with` + spec-driven `open_inner`

**Files:**
- Modify: `crates/cogito-core/src/runtime/builder.rs`
- Test: `crates/cogito-core/tests/per_session_injection.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-core/tests/per_session_injection.rs`. The test opens a session with a `SessionSpec` carrying a per-session tool provider and a per-session strategy, then drives one turn to completion — proving the spec is accepted and used to build the session.

```rust
//! ADR-0028: per-session provider injection via open_session_with /
//! update_session. Patterned on tests/runtime_submit.rs (real
//! MockModelGateway + JsonlStore + BuiltinToolProvider).

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, SessionSpec, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::ToolProvider;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};

fn end_turn_reply() -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { block_index: 0, chunk: "ack".into() },
        ModelEvent::TextBlockCompleted { block_index: 0, text: "ack".into() },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 1, output_tokens: 1 },
        },
    ]
}

fn builtin_tools() -> Arc<dyn ToolProvider> {
    Arc::new(BuiltinToolProvider::builder().with_tool(Arc::new(ReadFile)).build())
}

async fn await_turn_completed(handle: &cogito_core::runtime::SessionHandle) -> bool {
    let mut events = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[tokio::test]
async fn open_session_with_uses_injected_providers() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    // Runtime default providers (a baseline tool set + default strategy).
    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    // Per-session overrides: a distinct strategy + a per-session tool set.
    let mut per_session_strategy = HarnessStrategy::default_with_model("mock");
    per_session_strategy.name = "tenant-acme".into();
    let spec = SessionSpec {
        tools: Some(builtin_tools()),
        strategy: Some(per_session_strategy),
        tenant_id: Some("acme".into()),
        ..Default::default()
    };

    let sid = SessionId::new();
    let handle = runtime.open_session_with(sid, OpenMode::New, spec).await?;
    handle.submit_user_text("hello").await?;

    assert!(await_turn_completed(&handle).await, "turn did not complete");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}
```

> `HarnessStrategy::name` is a public field (it is read as `strategy.name.clone()` in `builder.rs`). `default_with_model` is the constructor used across the existing test suite.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-core open_session_with_uses_injected_providers`
Expected: FAIL to compile — `open_session_with` and `SessionSpec` not found.

- [ ] **Step 3: Add `open_session_with` and make `open_session` delegate**

In `crates/cogito-core/src/runtime/builder.rs`, add the import next to the other `super::` imports:

```rust
use super::session_spec::SessionSpec;
```

Replace the existing `open_session` method body so it delegates, and add `open_session_with` directly after it:

```rust
    pub async fn open_session(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        self.open_session_with(id, mode, SessionSpec::default()).await
    }

    /// Open (or resume) a session with per-session provider overrides.
    ///
    /// Each `Some` field of `spec` replaces the corresponding Runtime
    /// default for this session only. `tenant_id` / `user_id` are stamped
    /// into the session's `SessionMeta`. See ADR-0028.
    ///
    /// # Errors
    /// Same as [`Runtime::open_session`].
    pub async fn open_session_with(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
        spec: SessionSpec,
    ) -> Result<SessionHandle, RuntimeError> {
        let strategy = spec.strategy.clone().unwrap_or_else(|| self.strategy.clone());
        self.open_inner(id, mode, strategy, spec, None, 0, true).await
    }
```

Keep the existing doc comment block above `open_session` (it documents `OpenMode` semantics and errors).

- [ ] **Step 4: Thread `spec` through `open_inner`**

Change `open_inner`'s signature to add `spec` immediately after `strategy`:

```rust
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn open_inner(
        self: &Arc<Self>,
        id: SessionId,
        mode: OpenMode,
        strategy: HarnessStrategy,
        spec: SessionSpec,
        meta_override: Option<cogito_protocol::SessionMeta>,
        subagent_depth: u32,
        register: bool,
    ) -> Result<SessionHandle, RuntimeError> {
```

Resolve per-session providers. Insert these two lines just before the `// Build metrics first` comment block:

```rust
        // Per-session provider resolution: spec field wins, else Runtime default.
        let session_tools = spec.tools.clone().unwrap_or_else(|| Arc::clone(&self.tools));
        let session_skills = spec.skills.clone().or_else(|| self.skills.clone());
```

Stamp tenant/user into the derived meta. Replace the existing `let meta = meta_override.unwrap_or_else(...)` block with:

```rust
            let meta = meta_override.unwrap_or_else(|| cogito_protocol::SessionMeta {
                cogito_version: env!("CARGO_PKG_VERSION").into(),
                strategy: Some(strategy.name.clone()),
                model: Some(strategy.model_params.model.clone()),
                tenant_id: spec.tenant_id.clone(),
                user_id: spec.user_id.clone(),
                ..Default::default()
            });
```

In the `SessionState { ... }` initializer, replace `skills: self.skills.clone(),` with:

```rust
            skills: session_skills,
```

In the `SessionDeps { ... }` initializer, replace `tools: Arc::clone(&self.tools),` with:

```rust
            tools: session_tools,
```

- [ ] **Step 5: Fix the subagent call site**

`RuntimeSpawner::run_to_completion` calls `open_inner(child_id, OpenMode::New, strategy, Some(meta), req.parent_depth + 1, false)`. Insert `SessionSpec::default()` in the new `spec` position (after `strategy`):

```rust
            .open_inner(
                child_id,
                OpenMode::New,
                strategy,
                SessionSpec::default(),
                Some(meta),
                req.parent_depth + 1,
                false,
            )
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo nextest run -p cogito-core open_session_with_uses_injected_providers`
Expected: PASS.

- [ ] **Step 7: Verify the legacy path is unbroken**

Run: `cargo nextest run -p cogito-core --test runtime_submit --test session_e2e --test subagent_delegate`
Expected: all PASS (delegation + the spawner fix preserve old behavior).

- [ ] **Step 8: Commit**

```bash
git add crates/cogito-core/src/runtime/builder.rs crates/cogito-core/tests/per_session_injection.rs
git commit -m "feat(core): Runtime::open_session_with injects per-session providers (ADR-0028)"
```

---

## Task 3: Mutable providers via `update_session`

**Files:**
- Modify: `crates/cogito-core/src/runtime/types.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs`
- Modify: `crates/cogito-core/src/runtime/handle.rs`
- Test: `crates/cogito-core/tests/per_session_injection.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-core/tests/per_session_injection.rs`:

```rust
#[tokio::test]
async fn update_session_then_turn_completes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(end_turn_reply());

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(mock)
        .tools(builtin_tools())
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let sid = SessionId::new();
    let handle = runtime.open_session(sid, OpenMode::New).await?;

    // Swap the tool provider mid-session (no turn in flight yet).
    let spec = SessionSpec { tools: Some(builtin_tools()), ..Default::default() };
    handle.update_session(spec).await?;

    // The next turn must still complete with the swapped provider.
    handle.submit_user_text("hi").await?;
    assert!(await_turn_completed(&handle).await, "turn did not complete after update");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-core update_session_then_turn_completes`
Expected: FAIL to compile — `SessionHandle::update_session` does not exist.

- [ ] **Step 3: Add the `UpdateSession` command variant**

In `crates/cogito-core/src/runtime/types.rs`, add a variant to the `#[non_exhaustive]` `SessionCommand` enum (after `Trigger`). `SessionSpec` has a manual `Debug` impl, so the enum's `#[derive(Debug)]` still holds:

```rust
    /// Replace one or more per-session providers; effective next turn.
    /// See ADR-0028. `tenant_id` / `user_id` on the spec are ignored
    /// (session identity is fixed at open time).
    UpdateSession(crate::runtime::SessionSpec),
```

- [ ] **Step 4: Make `deps` mutable and intercept the command in the loop**

In `crates/cogito-core/src/runtime/session_loop.rs`:

Add the import near the other `super::`/`crate::` imports:

```rust
use crate::runtime::SessionSpec;
```

Change the `run_session` parameter `deps: SessionDeps` to `mut deps: SessionDeps`:

```rust
pub(super) async fn run_session(
    mut state: SessionState,
    mut mailbox_rx: mpsc::Receiver<SessionCommand>,
    mailbox_tx: mpsc::Sender<SessionCommand>,
    mut deps: SessionDeps,
    initial_events: Vec<cogito_protocol::ConversationEvent>,
) -> ShutdownOutcome {
```

In the mailbox loop, replace Arm 2 (`cmd = mailbox_rx.recv() => { ... }`) with a version that intercepts `UpdateSession` before delegating to `handle_command` (which only takes `&deps`):

```rust
            // Arm 2: caller commands.
            cmd = mailbox_rx.recv() => {
                let Some(cmd) = cmd else { break; };
                if let SessionCommand::UpdateSession(spec) = cmd {
                    apply_session_update(&mut state, &mut deps, spec);
                } else {
                    let outcome_opt = handle_command(&mut state, cmd, &mailbox_tx, &deps).await;
                    if let Some(outcome) = outcome_opt {
                        return outcome;
                    }
                }
            }
```

Add the free function near `spawn_turn_driver` (it swaps the live Arcs; the next `spawn_turn_driver` rebuilds `TurnDeps` from these, so the change lands at the next turn boundary):

```rust
/// Apply a mid-session provider swap (ADR-0028). Replaces only the
/// provided Arcs; `tenant_id` / `user_id` are intentionally not changed
/// (session identity is fixed at open). Effective at the next turn
/// boundary because `spawn_turn_driver` rebuilds `TurnDeps` from these.
fn apply_session_update(state: &mut SessionState, deps: &mut SessionDeps, spec: SessionSpec) {
    if let Some(tools) = spec.tools {
        deps.tools = tools;
    }
    if let Some(skills) = spec.skills {
        state.skills = Some(skills);
    }
    if let Some(strategy) = spec.strategy {
        state.strategy = strategy;
    }
}
```

> `handle_command` keeps its `&SessionDeps` signature — `UpdateSession` never reaches it. The `if let ... = cmd { } else { handle_command(cmd) }` form does not move `cmd` in the `else` branch, so passing `cmd` to `handle_command` there is valid.

- [ ] **Step 5: Add `SessionHandle::update_session`**

In `crates/cogito-core/src/runtime/handle.rs`, add a method inside `impl SessionHandle` (next to `submit`):

```rust
    /// Replace one or more of this session's providers. Each `Some` field
    /// of `spec` swaps the corresponding live provider; the change takes
    /// effect at the next turn boundary. `tenant_id` / `user_id` are
    /// ignored (identity is fixed at open time). See ADR-0028.
    ///
    /// # Errors
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn update_session(
        &self,
        spec: crate::runtime::SessionSpec,
    ) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::UpdateSession(spec))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }
```

`SessionCommand` is already imported in `handle.rs` (via `use super::types::{SessionCommand, ...}`).

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo nextest run -p cogito-core update_session_then_turn_completes`
Expected: PASS.

- [ ] **Step 7: Run the whole crate to catch fallout**

Run: `make test CRATE=cogito-core`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/cogito-core/src/runtime/types.rs crates/cogito-core/src/runtime/session_loop.rs crates/cogito-core/src/runtime/handle.rs crates/cogito-core/tests/per_session_injection.rs
git commit -m "feat(core): SessionHandle::update_session swaps providers mid-session (ADR-0028)"
```

---

## Task 4: Resume-chaos scenario `session_spec_mutated_then_resume`

**Files:**
- Test: `crates/cogito-core/tests/resume_chaos.rs`

Proves a session whose surface changed mid-run resumes correctly when the caller re-supplies the mutated spec.

- [ ] **Step 1: Read the existing chaos harness to match its shape**

Run: `grep -n "fn \|scenario\|oracle\|open_session\|PanicAt\|NotifyAt\|SessionSpec\|build" crates/cogito-core/tests/resume_chaos.rs`
Expected: lists the existing scenario fns, the crash-injection mechanism (`PanicAt`-style), the runtime builder helper, and the assertion helper that runs the four oracles. Read the closest existing scenario (`single_tool_happy_path` or `strategy_with_tool_filter`) in full before writing the new one.

- [ ] **Step 2: Add the `SessionSpec` import**

At the top of `crates/cogito-core/tests/resume_chaos.rs`, add (matching how the file already imports `OpenMode` / `Runtime` from `cogito_core::runtime`):

```rust
use cogito_core::runtime::SessionSpec;
```

- [ ] **Step 3: Write the new scenario**

Model it on the closest existing scenario. Reuse the file's existing runtime-builder, drive, crash-injection, and four-oracle assertion helpers verbatim — the only new wiring is two `SessionSpec` values and the `update_session` call between turns. The scenario:

1. Open with spec A (tool provider exposing one tool, e.g. `alpha`).
2. Drive one full turn that exercises spec A.
3. `update_session` to spec B (tool provider exposing `alpha` + a second tool `beta`).
4. Drive a turn whose scripted model output uses `beta`.
5. Inject a crash at each event boundary of that second turn.
6. Resume with `open_session_with(id, OpenMode::Resume, spec_b.clone())`.
7. Assert all four oracles (prefix-immutable / terminal-equivalent / tool-mapping-equivalent / final-text-equivalent).

```rust
#[tokio::test]
async fn session_spec_mutated_then_resume() {
    // spec_a / spec_b built from the file's existing scripted tool-provider
    // fixture; spec_b adds one tool. (Use the same provider type the other
    // scenarios use; do not introduce a new mock.)
    let spec_a = SessionSpec { tools: Some(/* provider with alpha */), ..Default::default() };
    let spec_b = SessionSpec { tools: Some(/* provider with alpha + beta */), ..Default::default() };

    for boundary in /* the file's existing boundary enumerator */ {
        // Pre-crash: open with A, run turn 1, update to B, run turn 2 to the crash point.
        // Resume: open_session_with(id, OpenMode::Resume, spec_b.clone()).
        // Then assert the four oracles via the existing helper.
    }
}
```

> If a per-scenario scripted tool provider does not already exist in the file, the closest existing scenario shows how tools are scripted; extend that fixture to expose a second tool rather than authoring a new mock. Keep the crash-injection and oracle plumbing identical to the sibling scenario.

- [ ] **Step 4: Run the scenario**

Run: `cargo nextest run -p cogito-core session_spec_mutated_then_resume`
Expected: PASS at every boundary.

- [ ] **Step 5: Run the full chaos suite**

Run: `make chaos`
Expected: all scenarios PASS, including the new one; total time within the suite's existing budget.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/tests/resume_chaos.rs
git commit -m "test(core): resume-chaos session_spec_mutated_then_resume (ADR-0028)"
```

---

## Task 5: Final verification + CI gate

**Files:** none (verification only)

- [ ] **Step 1: Format + lint**

Run: `make fmt && make fix CRATE=cogito-core`
Expected: no diffs after fmt; clippy clean (the new non-test code uses no `unwrap`/`expect`/`panic`).

- [ ] **Step 2: Full crate test**

Run: `make test CRATE=cogito-core`
Expected: all PASS.

- [ ] **Step 3: Workspace CI**

Run: `make ci`
Expected: fmt-check + clippy + layer-check + test green. Layer-check confirms `cogito-core` gained no forbidden import (this plan adds none).

- [ ] **Step 4: Commit any fmt-only changes**

```bash
git add -A
git commit -m "chore(core): fmt/clippy after ADR-0028 per-session injection" || echo "nothing to commit"
```

---

## Self-review notes (for the executor)

- **Spec coverage:** ADR-0028 §1 → Task 1; §2 (`open_session_with`) → Task 2; §3 (mutable, next-turn) → Task 3; §5 (resume re-supplies spec) → Task 4; §6 (Brain unchanged) → Task 5 layer-check + no harness file touched; §7 four change sites → `open_session_with` + `open_inner` (Task 2), `SessionState`/`SessionDeps` construction (Task 2), `UpdateSession` interception (Task 3).
- **Verified identifiers:** `SessionCommand` in `types.rs` (`#[non_exhaustive]`, `Debug`); `open_inner` arg order `(id, mode, strategy, spec, meta_override, subagent_depth, register)` at both call sites; `run_session(mut deps)`; `SessionError::SessionClosed { session_id }`; `EventPayload::SessionStarted { meta, strategy_name }` at seq 0 (so tests assert turn completion, not seq-0 readback).
- **Deliberately out of scope:** plugin loader (Plan 2); diagnostics-only `SessionMeta.extra` capture; deep "different surface used" assertion (covered by the chaos tool-mapping oracle, Task 4).
```

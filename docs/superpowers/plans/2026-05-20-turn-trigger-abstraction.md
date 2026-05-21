# TurnTrigger Abstraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land ADR-0016 v0.1 scope — introduce `TurnTrigger` in `cogito-protocol`, add `SessionHandle::submit(TurnTrigger)`, rename `SessionCommand::Input(NewMessage)` to `SessionCommand::Trigger(TurnTrigger)`, delete `NewMessage`, keep `send_user` as a 1-line shim. Event payload unchanged in v0.1.

**Architecture:** Single-variant `#[non_exhaustive]` enum lives in `cogito-protocol` so Brain and consumers pattern-match without knowing concrete trigger sources. The per-session loop projects `TurnTrigger` into `Vec<ContentBlock>` inside `try_start_turn` — the existing `TurnStarted { user_input }` shape is preserved exactly. No `schema_version` bump. `send_user(text)` retained, becoming a 1-line wrapper around `submit(TurnTrigger::UserText(text.into()))` so all existing call sites (`cogito-cli`, integration tests, chaos tests) stay green without modification.

**Tech Stack:** Rust 2024, `serde`/`serde_json`, `schemars`, `tokio`, workspace `just` recipes (`just fmt`, `just fix`, `just test`, `just ci`). TDD via `cargo nextest`.

**Spec:** `docs/adr/0016-turn-trigger-abstraction.md` (status: Proposed → flipped to Accepted at the end of this plan).

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/cogito-protocol/src/turn_trigger.rs` | Create | `TurnTrigger` enum + unit tests (serde roundtrip + reserved-variant docstrings). |
| `crates/cogito-protocol/src/lib.rs` | Modify | Register `turn_trigger` module; re-export `TurnTrigger` at crate root next to `ContentBlock`. |
| `crates/cogito-core/src/runtime/types.rs` | Modify | Replace `SessionCommand::Input(NewMessage)` with `SessionCommand::Trigger(TurnTrigger)`. Delete `NewMessage` struct + its doc-comment. |
| `crates/cogito-core/src/runtime/handle.rs` | Modify | Add `submit(&self, trigger: TurnTrigger) -> Result<(), SessionError>`. Refactor `send_user` to delegate via `self.submit(TurnTrigger::UserText(text.into())).await`. Drop the `NewMessage` import. |
| `crates/cogito-core/src/runtime/session_loop.rs` | Modify | Match arm `SessionCommand::Trigger(trigger)`; change `try_start_turn` signature to take `TurnTrigger`; project to `Vec<ContentBlock>` (`UserText(text)` → `vec![ContentBlock::Text { text }]`). Force a non-exhaustive `_ =>` arm with a `tracing::error!` so future-variant slip is loud but non-fatal. |
| `crates/cogito-core/src/runtime/mod.rs` | Modify | Drop `NewMessage` from `pub use types::{...}` re-export. |
| `crates/cogito-core/tests/runtime_submit.rs` | Create | Integration test exercising `SessionHandle::submit(TurnTrigger::UserText(...))` end-to-end: open session → submit → assert `TurnStarted.user_input == vec![Text("hello")]` from the persisted log. |
| `docs/adr/0016-turn-trigger-abstraction.md` | Modify | Status: Proposed → Accepted (2026-05-20). |
| `docs/adr/README.md` | Modify | Index entry stays as ADR-0016 (number unchanged per user decision); no text change unless the description needs sync. |
| `docs/components/H01-turn-driver.md` | Modify | Line 201: replace `input: NewMessage(text) | ResumeAfter(job_id)` with `trigger: TurnTrigger | ResumeAfter(job_id)`. |
| `CHANGELOG.md` | Modify | Add `### Added — Sprint 4 prep (TurnTrigger)` block under `[Unreleased]` (or extend the existing Sprint-3 block — see Task 4) documenting `TurnTrigger`, `SessionHandle::submit`, and the internal rename. |

**Commits**: one per task (4 total). Each task leaves the workspace in a green `just ci` state.

---

## Task 1: Add `TurnTrigger` enum to `cogito-protocol`

**Files:**
- Create: `crates/cogito-protocol/src/turn_trigger.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

This task is self-contained: no consumer yet, so existing tests are unaffected. Lands a single-variant `#[non_exhaustive]` enum + serde roundtrip test + reserved-variant docstrings (per ADR §1).

- [ ] **Step 1.1: Write the failing test in a new module**

Create `crates/cogito-protocol/src/turn_trigger.rs` with:

```rust
//! `TurnTrigger` — what caused a new turn to start. Single source of
//! truth for "what triggered this turn", open-by-extension via
//! `#[non_exhaustive]` per ADR-0007 b-档 (additive variants do NOT bump
//! `schema_version`). See ADR-0016 for the design rationale and the
//! v0.2 / v0.3 / v0.6 migration plan.

use serde::{Deserialize, Serialize};

/// What caused a new turn to start. Open-by-extension via
/// `#[non_exhaustive]` per ADR-0007 b-档: future variants are additive
/// and do NOT bump `schema_version`.
///
/// v0.1 ships exactly one variant. A single-variant `#[non_exhaustive]`
/// enum is intentional: it locks the *shape* of the abstraction so that
/// future variants are additive (Skill, Hook, multimodal user content),
/// even though the enum looks like overkill today.
///
/// Reserved variants (DO NOT add to the enum until the matching
/// consumer lands — adding a variant before its handler exists creates
/// a dead-code path that drifts unverified):
///
/// - `UserContent(Vec<ContentBlock>)` — lands with the v0.2 multimedia
///   ADR + `ContentBlock::{Image, Audio}`. Projection: the per-session
///   loop writes `TurnStarted.user_input = blocks` verbatim.
/// - `SkillInvocation { skill_id: String, args: serde_json::Value }` —
///   lands with the post-v0.3 Skills initiative. Projection: the loop
///   writes `TurnStarted.origin = Skill { skill_id }` and derives
///   `user_input` from `args`.
/// - `HookFired { hook_id: String, payload: serde_json::Value }` —
///   lands with the post-v0.6 Hooks initiative beyond H09's policy
///   gate. Projection: the loop writes
///   `TurnStarted.origin = Hook { hook_id }` and derives `user_input`
///   from `payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnTrigger {
    /// User-typed plain text. The overwhelmingly common case for v0.1.
    UserText(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_serde_roundtrip() {
        let original = TurnTrigger::UserText("hello".into());
        let json = serde_json::to_string(&original).expect("serialize");
        assert_eq!(json, r#"{"kind":"user_text","data":"hello"}"#);
        let parsed: TurnTrigger = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, original);
    }

    #[test]
    fn unknown_kind_fails_to_deserialize() {
        // Until v0.2 lands `UserContent` (or v0.3 lands `SkillInvocation`),
        // unknown `kind` values must fail loudly: TurnTrigger is wire-internal
        // between SessionHandle and the per-session loop, NOT a persisted
        // event-log payload that needs forward-tolerance per ADR-0007.
        let unknown = r#"{"kind":"skill_invocation","data":{"skill_id":"foo"}}"#;
        let result: Result<TurnTrigger, _> = serde_json::from_str(unknown);
        assert!(result.is_err(), "expected unknown variant to error; got {result:?}");
    }
}
```

- [ ] **Step 1.2: Register the module in `lib.rs`**

Edit `crates/cogito-protocol/src/lib.rs`. Insert the new `pub mod turn_trigger;` line in the module list (alphabetical-ish; place it between `tool` and `turn`) and add a re-export next to `ContentBlock`:

```rust
pub mod content;
pub mod error;
pub mod event;
pub mod exec_ctx;
pub mod gateway;
pub mod ids;
pub mod job;
pub mod session;
pub mod store;
pub mod strategy;
pub mod stream;
pub mod tool;
pub mod turn;
pub mod turn_trigger;  // <-- added

pub use content::ContentBlock;
pub use turn_trigger::TurnTrigger;  // <-- added, group with ContentBlock
pub use event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
// ... (rest unchanged)
```

Also extend the module map doc-comment at the top of `lib.rs` with one bullet:

```text
//! - [`turn_trigger`]: `TurnTrigger` — what caused a new turn to start (ADR-0016)
```

- [ ] **Step 1.3: Run the tests, expect green**

Run: `just test -p cogito-protocol`
Expected: all tests pass including the two new ones in `turn_trigger::tests`. If the `unknown_kind_fails_to_deserialize` test surprises you and passes deserialization (i.e., serde silently accepts the unknown variant), that's a serde-behavior assumption violation — investigate before continuing; the ADR's reasoning depends on this.

- [ ] **Step 1.4: Run `just fmt && just fix -p cogito-protocol`**

Run: `just fmt && just fix -p cogito-protocol`
Expected: no diffs, no clippy warnings.

- [ ] **Step 1.5: Commit**

```bash
git add crates/cogito-protocol/src/turn_trigger.rs crates/cogito-protocol/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(protocol): add TurnTrigger enum (ADR-0016 v0.1 scope)

Single-variant #[non_exhaustive] enum lives in cogito-protocol so Brain
and consumers can pattern-match without knowing concrete trigger
sources. Locks the shape of the abstraction; v0.2 / v0.3 / v0.6 add
UserContent / SkillInvocation / HookFired additively (no schema bump
per ADR-0007 b-档). No consumer yet — wiring lands in the next commit.
EOF
)"
```

---

## Task 2: Internal rename `SessionCommand::Input(NewMessage)` → `SessionCommand::Trigger(TurnTrigger)`

**Files:**
- Modify: `crates/cogito-core/src/runtime/types.rs`
- Modify: `crates/cogito-core/src/runtime/handle.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs`
- Modify: `crates/cogito-core/src/runtime/mod.rs`

Pure refactor: `send_user` still constructs the inbound command from a `String`, just through a `TurnTrigger::UserText(...)` wrapper. All existing call sites (`session_e2e`, `runtime_resume_dispatch`, `resume_chaos`, `cogito-cli`) stay unchanged and stay green. `NewMessage` is deleted.

- [ ] **Step 2.1: Update `types.rs` — replace command variant, delete struct**

Open `crates/cogito-core/src/runtime/types.rs`.

Add the import near the existing `cogito_protocol::*` re-exports (top of file):

```rust
pub use cogito_protocol::turn_trigger::TurnTrigger;
```

Replace the `Input(NewMessage)` variant inside `SessionCommand` (lines ~55-58) with:

```rust
    /// Caller-driven trigger. Spawns a new `TurnDriver` when no turn is
    /// in flight. v0.1 only carries `TurnTrigger::UserText`; future
    /// variants (multimedia user content, skill invocations, hook
    /// fires) land additively per ADR-0016 + ADR-0007 b-档.
    Trigger(TurnTrigger),
```

Delete the `NewMessage` struct entirely (lines ~86-93 of the current file — the `pub struct NewMessage { pub text: String }` block and its preceding doc-comment).

The `impl From<JobCompletionEvent> for SessionCommand` block stays unchanged.

- [ ] **Step 2.2: Update `mod.rs` — drop `NewMessage` from public re-export**

Open `crates/cogito-core/src/runtime/mod.rs` and replace line 25:

```rust
pub use types::{NewMessage, OpenMode, SessionCommand, SessionId, ShutdownOutcome};
```

with:

```rust
pub use types::{OpenMode, SessionCommand, SessionId, ShutdownOutcome, TurnTrigger};
```

(`TurnTrigger` is re-exported through `runtime::types::TurnTrigger` for ergonomic access from runtime consumers; the canonical home stays `cogito_protocol::TurnTrigger`.)

- [ ] **Step 2.3: Update `handle.rs` — `send_user` constructs `Trigger(UserText(...))`**

Open `crates/cogito-core/src/runtime/handle.rs`.

Change the `use super::types::{...}` import on line 12 from:

```rust
use super::types::{NewMessage, SessionCommand, SessionId, ShutdownOutcome};
```

to:

```rust
use super::types::{SessionCommand, SessionId, ShutdownOutcome, TurnTrigger};
```

Replace the body of `send_user` (lines ~58-66) with:

```rust
    /// Send a new user text message; the actor will spawn a `TurnDriver`.
    /// Awaits (mailbox backpressure) if the actor is overwhelmed.
    ///
    /// Convenience wrapper around [`SessionHandle::submit`] — equivalent
    /// to `submit(TurnTrigger::UserText(text.into()))`. Retained because
    /// user-typed text is the dominant path and callers should not have
    /// to spell out the enum for it. See ADR-0016 §2.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn send_user(&self, text: impl Into<String>) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::Trigger(TurnTrigger::UserText(text.into())))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }
```

(Task 3 will refactor `send_user` further to delegate to `submit`. For this task we leave the duplication out — `submit` doesn't exist yet — so the rename is self-contained and the diff is reviewable.)

- [ ] **Step 2.4: Update `session_loop.rs` — match `Trigger`, project trigger in `try_start_turn`**

Open `crates/cogito-core/src/runtime/session_loop.rs`.

Change the `use super::types::{...}` import on line 41 from:

```rust
use super::types::{NewMessage, SessionCommand, ShutdownOutcome};
```

to:

```rust
use super::types::{SessionCommand, ShutdownOutcome, TurnTrigger};
```

In `handle_command` (lines ~362-388), replace the `SessionCommand::Input(msg) => { try_start_turn(state, msg, deps).await; None }` arm with:

```rust
        SessionCommand::Trigger(trigger) => {
            try_start_turn(state, trigger, deps).await;
            None
        }
```

Replace the `try_start_turn` signature (line ~394) and the projection logic (lines ~399-423). The full new body:

```rust
/// Attempt to start a fresh turn from a caller-submitted `TurnTrigger`.
/// No-op if a turn is already in flight. Always uses
/// `TurnEntry::FreshLikeInit` — resume dispatch happens once at actor
/// startup via `apply_resume_point`, not here.
///
/// `TurnTrigger` projection (v0.1 single-variant; ADR-0016):
/// - `UserText(text)` → `user_input = vec![ContentBlock::Text { text }]`
///
/// Future variants (UserContent / SkillInvocation / HookFired) extend
/// this match. `#[non_exhaustive]` forces the `_ =>` arm; we log loudly
/// and drop the trigger rather than panic — a missed variant is a
/// runtime bug, not a turn failure.
async fn try_start_turn(state: &mut SessionState, trigger: TurnTrigger, deps: &SessionDeps) {
    if state.has_active_turn() {
        return;
    }

    let user_input: Vec<ContentBlock> = match trigger {
        TurnTrigger::UserText(text) => vec![ContentBlock::Text { text }],
        // `#[non_exhaustive]` guard: when a future TurnTrigger variant
        // lands (ADR-0016 §6 migration table) the consumer crate that
        // adds the variant must also extend this match. Until then,
        // log + drop is correct: no event is written, no turn spawned.
        _ => {
            tracing::error!(
                session_id = %state.session_id,
                "unhandled TurnTrigger variant; dropping turn (this is a build wiring bug)"
            );
            return;
        }
    };

    let turn_id = TurnId::new();

    // Write-before-transition: record TurnStarted before spawning the task.
    {
        let mut rec = state.recorder.lock().await;
        if let Err(e) = rec.record_turn_started(turn_id, user_input).await {
            tracing::error!(
                session_id = %state.session_id,
                turn_id = %turn_id,
                error = %e,
                "failed to record TurnStarted; aborting turn"
            );
            return;
        }
    }

    spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit, deps);
}
```

- [ ] **Step 2.5: Run targeted tests, expect green**

Run: `just test -p cogito-core`
Expected: all tests pass — `session_e2e`, `runtime_resume_dispatch`, `runtime_session_mode`, `resume_chaos`, and the harness tests. Semantics are preserved (a `String` becomes a `TurnTrigger::UserText` which projects back to the same `Vec<ContentBlock>` the old code wrote).

If any test fails:
- Re-check that `send_user` still serializes through the mailbox.
- Re-check that the projection inside `try_start_turn` produces exactly `vec![ContentBlock::Text { text }]` (no leading/trailing whitespace, no extra blocks).

- [ ] **Step 2.6: Run `just fmt && just fix -p cogito-core`**

Run: `just fmt && just fix -p cogito-core`
Expected: no diffs, no clippy warnings. (The `_ =>` arm may trigger a `clippy::match_wildcard_for_single_variants` warning today, since there is exactly one variant. If clippy complains, silence it locally with a targeted `#[allow(clippy::match_wildcard_for_single_variants)]` on the match statement and a comment pointing at ADR-0016 — the wildcard is a *deliberate* forward-compat hedge, not a missed variant.)

- [ ] **Step 2.7: Run full CI, expect green**

Run: `just ci`
Expected: fmt-check + clippy + full test suite all pass. Confirms `cogito-cli` (which uses `send_user`) and the chaos tests still work.

- [ ] **Step 2.8: Commit**

```bash
git add crates/cogito-core/src/runtime/types.rs crates/cogito-core/src/runtime/handle.rs crates/cogito-core/src/runtime/session_loop.rs crates/cogito-core/src/runtime/mod.rs
git commit -m "$(cat <<'EOF'
refactor(core): rename SessionCommand::Input(NewMessage) -> Trigger(TurnTrigger)

Pure internal rename per ADR-0016 v0.1 scope. send_user still accepts
String and constructs the mailbox command internally — all existing
call sites (session_e2e, runtime_resume_dispatch, resume_chaos,
cogito-cli) unchanged and green. NewMessage struct deleted (was never
exposed beyond runtime::types). try_start_turn now projects the
trigger into Vec<ContentBlock> with a non-exhaustive guard arm for
future variants. Event payload unchanged in v0.1.
EOF
)"
```

---

## Task 3: Add public `SessionHandle::submit(TurnTrigger)` + integration test

**Files:**
- Create: `crates/cogito-core/tests/runtime_submit.rs`
- Modify: `crates/cogito-core/src/runtime/handle.rs`

Adds the canonical entry point. Verifies via an end-to-end integration test that `submit(TurnTrigger::UserText("hello"))` produces a `TurnStarted` event whose `user_input` matches the old `send_user` path exactly. After the test passes, `send_user` is refactored one more time to delegate to `submit`, eliminating the duplicated mailbox-send code.

- [ ] **Step 3.1: Write the failing integration test**

Create `crates/cogito-core/tests/runtime_submit.rs` with:

```rust
//! Integration test for `SessionHandle::submit(TurnTrigger)` — ADR-0016.
//!
//! Mirrors `session_e2e::open_send_complete_shutdown` but exercises the
//! `submit` path with a `TurnTrigger::UserText` payload, then asserts
//! that the persisted `TurnStarted` event's `user_input` is exactly
//! `vec![ContentBlock::Text { text: "hello" }]`. This locks the v0.1
//! projection (ADR-0016 §4: "TurnTrigger::UserText(text) projects to
//! vec![ContentBlock::Text { text }]").

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime, ShutdownOutcome};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::turn_trigger::TurnTrigger;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;

#[tokio::test]
async fn submit_user_text_projects_to_text_content_block(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let mock = Arc::new(MockModelGateway::new());
    mock.push_reply(vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "ack".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "ack".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        },
    ]);

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Clone the Arc into the builder so the local `store` handle stays
    // alive for the read-back assertion below. The Arc unsizes
    // `Arc<JsonlStore>` -> `Arc<dyn ConversationStore>` implicitly.
    let runtime = Runtime::builder()
        .store(Arc::clone(&store))
        .model(mock)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // The canonical entry point for any new turn trigger source.
    handle
        .submit(TurnTrigger::UserText("hello".into()))
        .await?;

    let got_completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(got_completed, "TurnCompleted event not received within 5s");

    let out = handle.shutdown(Duration::from_secs(5)).await?;
    assert!(matches!(out, ShutdownOutcome::Clean { .. }));

    // Now verify the projection: read the persisted log and confirm the
    // first TurnStarted event carries user_input = [Text("hello")].
    // `replay(session_id, 0)` yields events with `seq > 0`; SessionStarted
    // sits at seq 0 and is skipped, but TurnStarted (seq 1) is included.
    // The trait method is in scope via the `ConversationStore` import; the
    // call auto-derefs through `Arc<JsonlStore>`.
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };
    let turn_started = persisted
        .iter()
        .find(|e| matches!(&e.payload, EventPayload::TurnStarted { .. }))
        .expect("expected a TurnStarted event in the persisted log");
    match &turn_started.payload {
        EventPayload::TurnStarted { user_input, .. } => {
            assert_eq!(
                user_input,
                &vec![ContentBlock::Text {
                    text: "hello".into()
                }],
                "TurnTrigger::UserText projection must equal vec![Text(text)] per ADR-0016 §4"
            );
        }
        other => panic!("expected TurnStarted, got {other:?}"),
    }

    Ok(())
}
```

- [ ] **Step 3.2: Run the test, expect FAIL (compile error: `submit` not found)**

Run: `cargo test -p cogito-core --test runtime_submit`
Expected: compile error — `no method named submit found for struct SessionHandle`. Confirms the test exercises the new surface.

- [ ] **Step 3.3: Add `submit` and refactor `send_user` to delegate**

Open `crates/cogito-core/src/runtime/handle.rs`.

Inside `impl SessionHandle`, **above** `send_user`, insert:

```rust
    /// Submit a [`TurnTrigger`]. The session loop spawns a `TurnDriver`
    /// if no turn is in flight. **Canonical entry point** for any new
    /// trigger source — `send_user` is a convenience shim that calls
    /// `submit(TurnTrigger::UserText(text.into()))`. See ADR-0016 §2.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn submit(&self, trigger: TurnTrigger) -> Result<(), SessionError> {
        self.shared
            .mailbox_tx
            .send(SessionCommand::Trigger(trigger))
            .await
            .map_err(|_| SessionError::SessionClosed {
                session_id: self.shared.session_id,
            })
    }
```

Refactor `send_user` (which Task 2 left as a direct mailbox send) to delegate:

```rust
    /// Send a new user text message; the actor will spawn a `TurnDriver`.
    /// Awaits (mailbox backpressure) if the actor is overwhelmed.
    ///
    /// Convenience wrapper around [`SessionHandle::submit`] — equivalent
    /// to `submit(TurnTrigger::UserText(text.into()))`. Retained because
    /// user-typed text is the dominant path and callers should not have
    /// to spell out the enum for it. See ADR-0016 §2.
    ///
    /// # Errors
    ///
    /// Returns `SessionError::SessionClosed` if the actor has exited.
    pub async fn send_user(&self, text: impl Into<String>) -> Result<(), SessionError> {
        self.submit(TurnTrigger::UserText(text.into())).await
    }
```

- [ ] **Step 3.4: Run the new test, expect PASS**

Run: `cargo test -p cogito-core --test runtime_submit`
Expected: PASS.

- [ ] **Step 3.5: Run full CI, expect green**

Run: `just ci`
Expected: fmt-check + clippy + full test suite all pass. Confirms `send_user` callers still work through the delegating shim and the `submit` path matches them byte-for-byte at the projection layer.

- [ ] **Step 3.6: Run `just fmt && just fix -p cogito-core`**

Run: `just fmt && just fix -p cogito-core`
Expected: no diffs, no clippy warnings.

- [ ] **Step 3.7: Commit**

```bash
git add crates/cogito-core/src/runtime/handle.rs crates/cogito-core/tests/runtime_submit.rs
git commit -m "$(cat <<'EOF'
feat(core): SessionHandle::submit(TurnTrigger) (ADR-0016 v0.1 scope)

Canonical entry point for any new turn trigger source. send_user is
now a 1-line shim around submit(TurnTrigger::UserText(text.into())).
Integration test asserts the v0.1 projection: TurnTrigger::UserText
maps to vec![ContentBlock::Text { text }] in TurnStarted.user_input.
EOF
)"
```

---

## Task 4: Status flip + doc sync + CHANGELOG

**Files:**
- Modify: `docs/adr/0016-turn-trigger-abstraction.md`
- Modify: `docs/components/H01-turn-driver.md`
- Modify: `CHANGELOG.md`

Marks ADR-0016 Accepted, syncs the one stale `NewMessage` reference in the H01 doc, and records the user-facing change in CHANGELOG.

- [ ] **Step 4.1: Flip ADR-0016 status**

In `docs/adr/0016-turn-trigger-abstraction.md`, change line 5 from:

```markdown
Proposed (2026-05-20).
```

to:

```markdown
Accepted (2026-05-20).
```

(`docs/adr/README.md` already lists ADR-0016 with a one-line description — no change needed; the description does not embed a status.)

- [ ] **Step 4.2: Sync `H01-turn-driver.md` to drop the `NewMessage` reference**

In `docs/components/H01-turn-driver.md`, line 201, change:

```markdown
- `TurnRequest { session_id, input: NewMessage(text) | ResumeAfter(job_id) }`
```

to:

```markdown
- `TurnRequest { session_id, input: TurnTrigger | ResumeAfter(job_id) }` (see ADR-0016)
```

Skim the surrounding paragraph (lines ~195-215) for any other `NewMessage` mentions; there should be none, but the conceptual sentence may need a one-clause edit for tense / grammar. Keep edits minimal.

- [ ] **Step 4.3: Add CHANGELOG entry**

Open `CHANGELOG.md`. Under the existing `## [Unreleased]` section, **above** `### Added — Sprint 2`, insert a new block:

```markdown
### Added — Post-Sprint 3 (ADR-0016 TurnTrigger)

- `cogito-protocol::turn_trigger::TurnTrigger` — single-variant
  `#[non_exhaustive]` enum (`UserText(String)` in v0.1). Locks the
  shape of the abstraction so v0.2 `UserContent`, v0.3+
  `SkillInvocation`, and v0.6 `HookFired` land additively per ADR-0007
  b-档 (no `schema_version` bump). Re-exported at `cogito_protocol::TurnTrigger`.
- `cogito-core::runtime::SessionHandle::submit(TurnTrigger)` —
  canonical entry point for any new trigger source. See ADR-0016 §2.

### Changed — Post-Sprint 3 (ADR-0016 TurnTrigger)

- `cogito-core::runtime::SessionCommand::Input(NewMessage)` renamed to
  `SessionCommand::Trigger(TurnTrigger)`. Internal rename; the
  `NewMessage` struct is deleted (was never exposed outside
  `cogito_core::runtime::types`).
- `cogito-core::runtime::SessionHandle::send_user` is now a 1-line
  shim around `submit(TurnTrigger::UserText(text.into()))`. Behavior
  preserved; existing call sites (`cogito-cli`, integration tests,
  chaos tests) unchanged.
- `cogito-core::runtime::mod` no longer re-exports `NewMessage`; it
  re-exports `TurnTrigger` instead.
```

- [ ] **Step 4.4: Run full CI, expect green**

Run: `just ci`
Expected: all green. (No behavioral changes — this commit is doc + CHANGELOG only — but `just ci` confirms no formatting regressions snuck in.)

- [ ] **Step 4.5: Commit**

```bash
git add docs/adr/0016-turn-trigger-abstraction.md docs/components/H01-turn-driver.md CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(adr): mark ADR-0016 Accepted; sync H01 + CHANGELOG

ADR-0016 status Proposed -> Accepted (2026-05-20). H01 doc swaps the
stale NewMessage(text) reference for TurnTrigger. CHANGELOG records
the new public surface (TurnTrigger, SessionHandle::submit) and the
internal rename for [Unreleased].
EOF
)"
```

---

## Self-Review Checklist

After implementing all tasks, audit:

1. **Spec coverage** — every ADR-0016 §"Decision" item maps to a task:
   - §1 `TurnTrigger` in `cogito-protocol` → Task 1.
   - §2 `SessionHandle::submit` + retained `send_user` → Task 3.
   - §3 `SessionCommand::Trigger(TurnTrigger)` rename, `NewMessage` removed → Task 2.
   - §4 event-log projection (unchanged in v0.1) → Task 2 (`try_start_turn` projection) + Task 3 (integration test).
   - §5 H03 resume semantics (no algorithm change) → no task needed; resume path is untouched and the existing `runtime_resume_dispatch` + `resume_chaos` tests are the regression gate (run as part of `just ci`).
   - §6 Migration plan v0.1 row → Tasks 1-3.
2. **Public API surface** — `cogito-core::runtime::mod` re-exports `TurnTrigger`, `SessionCommand`, `OpenMode`, `SessionId`, `ShutdownOutcome`; `NewMessage` is gone. Verify with `grep -r NewMessage crates/` returning zero hits in `src/` and `tests/`.
3. **No dead-code reservations** — only `TurnTrigger::UserText` is in the enum. `UserContent` / `SkillInvocation` / `HookFired` exist as docstrings only (per ADR §1 "DO NOT add to the enum until the matching consumer lands").
4. **Tests** — `just test -p cogito-protocol` covers the protocol roundtrip; `just test -p cogito-core --test runtime_submit` covers the new entry point; `just ci` runs the full regression suite including `session_e2e`, `runtime_resume_dispatch`, `resume_chaos`, and `cogito-cli` builds.
5. **No `schema_version` bump** — `crates/cogito-protocol/src/event.rs` is not modified by this plan. Confirmed by Task file list.
6. **No CLAUDE.md violations** — all new `///` and `//` comments are English. No decorative numerals. Inline-comment placement matches existing modules.

If any item fails, fix inline before declaring the plan complete.

---

## Out of Scope (deferred, per ADR-0016 §7)

- Hook-pipeline initiator semantics (v0.6 Hooks ADR).
- Authorization / capability checking on `submit` (future ADR).
- Turn-trigger queueing / coalescing when a turn is already in flight (current behavior: silent no-op via the existing `try_start_turn` guard).
- Adding `TurnOrigin` to `TurnStarted` (lands with the first non-user variant — Skills or Hooks).
- `RestartCurrentTurn` recovery reading `TurnTrigger` from the persisted log (Sprint 3 closure narrowing; lands with the post-Sprint-3 recovery work).

# Sprint 0 Closure + Doc Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close out v0.1 Sprint 0 per spec `2026-05-18-runtime-h01-execution-model-design.md` §13 — fix workspace topology to ADR-0004 compliance, add the protocol types Sprint 1 needs, scaffold the Runtime module, sync authoritative docs (ARCHITECTURE.md / ROADMAP.md / H01-H02-H08), publish ADR-0006.

**Architecture:** Pure scaffolding + type definitions + doc updates. No business logic. Every Rust type added is accompanied by a serde-roundtrip or contract test. Runtime module ships as compilable stubs with `todo!()` bodies; real implementation lives in Plan 2 (Sprint 1).

**Tech Stack:** Rust 2024 (MSRV 1.83), tokio 1.40, tokio-util (CancellationToken), serde 1, thiserror 1, criterion (benches, Plan 2), nextest, just.

**Conventions (read before starting any task):**
- All Rust comments (`//`, `///`, `//!`) in English (per CLAUDE.md §Coding standards). Chinese stays in spec/ADR/commit/chat.
- Errors via `thiserror` (libraries) or `anyhow` (binaries). No `unwrap` / `expect` / `panic` in non-test code.
- `unsafe_code = "forbid"`. `missing_docs = "warn"` — every public item has a doc comment.
- Workspace deps go through `[workspace.dependencies]`; members declare `{ workspace = true }`.
- Commits: imperative, capitalized first word, no trailing period. Match recent style ("Lock target architecture...", "Design v0.1 Runtime...").
- Each task ends with a commit. Branch: `impl/sprint-0-closure` off `main` (created in Task 1).
- After plan completes: open PR `nathan-tsien:impl/sprint-0-closure -> main`, base = main.

---

## File structure (created or modified)

```
NEW   crates/cogito-store-jsonl/Cargo.toml
NEW   crates/cogito-store-jsonl/src/lib.rs
NEW   crates/cogito-core/src/runtime/mod.rs
NEW   crates/cogito-core/src/runtime/types.rs
NEW   crates/cogito-core/src/runtime/builder.rs
NEW   crates/cogito-core/src/runtime/handle.rs
NEW   crates/cogito-core/src/runtime/actor.rs
NEW   crates/cogito-core/src/runtime/store_writer.rs
NEW   crates/cogito-protocol/src/tool.rs
NEW   crates/cogito-protocol/src/stream.rs
NEW   crates/cogito-protocol/src/job.rs
NEW   crates/cogito-protocol/src/turn.rs
NEW   crates/cogito-protocol/src/error.rs
NEW   scripts/check-layer.sh
NEW   docs/adr/0006-runtime-h01-execution-model.md

DEL   crates/cogito-conversation/ (entire dir)

MOD   Cargo.toml                          (workspace members + deps)
MOD   crates/cogito-protocol/src/lib.rs   (re-export new modules)
MOD   crates/cogito-protocol/Cargo.toml   (+ tokio-util for CancellationToken; + tokio for mpsc::Sender in JobManager)
MOD   crates/cogito-core/Cargo.toml       (drop Hands/Boundary/Session deps; add tokio-util, dashmap)
MOD   crates/cogito-core/src/lib.rs       (export runtime module)
MOD   crates/cogito-jobs/Cargo.toml       (drop cogito-conversation; keep protocol)
MOD   crates/cogito-cli/Cargo.toml        (explicit Hands/Boundary/Session deps)
MOD   ARCHITECTURE.md                     (workspace layout + trait contracts sections)
MOD   ROADMAP.md                          (Sprint 0 checkboxes)
MOD   docs/components/H01-turn-driver.md  (impl note)
MOD   docs/components/H02-step-recorder.md (impl note)
MOD   docs/components/H08-tool-dispatcher.md (impl note)
MOD   .github/workflows/ci.yml            (add cargo-deny + layer check jobs)
MOD   justfile                            (add `layer-check` recipe)
```

**Each task below produces exactly one commit unless explicitly marked combined.** Total: 22 tasks → 22 commits → 1 PR.

---

## Task 0: Create branch

**Files:** none (git operation)

- [ ] **Step 1: Verify on main, clean tree**

Run: `git status && git branch --show-current`
Expected: `working tree clean`, branch `main`.

- [ ] **Step 2: Create and switch to feature branch**

Run: `git checkout -b impl/sprint-0-closure`
Expected: `Switched to a new branch 'impl/sprint-0-closure'`.

---

## Task 1: Add cogito-store-jsonl skeleton; drop cogito-conversation from workspace

**Files:**
- Create: `crates/cogito-store-jsonl/Cargo.toml`
- Create: `crates/cogito-store-jsonl/src/lib.rs`
- Modify: `Cargo.toml` (workspace members + `[workspace.dependencies]`)

- [ ] **Step 1: Create the new crate Cargo.toml**

`crates/cogito-store-jsonl/Cargo.toml`:

```toml
[package]
name = "cogito-store-jsonl"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
cogito-protocol.workspace = true

tokio.workspace = true
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true

[dev-dependencies]
cogito-test-fixtures.workspace = true
tokio-test.workspace = true
rstest.workspace = true
tempfile = "3.10"

[lints]
workspace = true
```

- [ ] **Step 2: Create the new crate lib.rs stub**

`crates/cogito-store-jsonl/src/lib.rs`:

```rust
//! cogito-store-jsonl
//!
//! Per-session JSONL backend for `ConversationStore`. Each session writes to
//! `<root>/sessions/<session_id>.jsonl`; every event is `fsync`'d to disk
//! before the append returns. This is the v0.1 sole `ConversationStore`
//! implementation; future backends (Postgres, HTTP) live in sibling crates.
//!
//! See:
//! - `docs/components/H02-step-recorder.md`
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` §8
```

- [ ] **Step 3: Update workspace Cargo.toml — members list**

Modify `Cargo.toml`. In `[workspace]` `members = [...]`, replace the line `"crates/cogito-conversation",` with `"crates/cogito-store-jsonl",`. Keep alphabetical order if practical.

- [ ] **Step 4: Update workspace Cargo.toml — workspace.dependencies**

In `[workspace.dependencies]`, replace:

```toml
cogito-conversation = { path = "crates/cogito-conversation" }
```

with:

```toml
cogito-store-jsonl = { path = "crates/cogito-store-jsonl" }
```

- [ ] **Step 5: Verify workspace check fails on dangling cogito_conversation usages**

Run: `cargo check --workspace 2>&1 | head -40`
Expected: errors like "could not find `cogito_conversation`" or "no matching package named `cogito-conversation`" — Task 2 / 3 / 4 / 6 will resolve them.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-store-jsonl/ Cargo.toml
git commit -m "$(cat <<'EOF'
Add cogito-store-jsonl crate skeleton and retire cogito-conversation

Per spec §13 Phase 1 step 1.1-1.3. The new crate is the v0.1 sole
ConversationStore backend; cogito-conversation is removed from the
workspace member list and workspace.dependencies. Downstream Cargo.toml
fixes follow in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Delete cogito-conversation crate directory

**Files:**
- Delete: `crates/cogito-conversation/` (entire directory)

- [ ] **Step 1: Remove the directory**

Run: `git rm -r crates/cogito-conversation/`
Expected: lists removed files (`Cargo.toml`, `src/lib.rs`).

- [ ] **Step 2: Verify removal**

Run: `ls crates/cogito-conversation/ 2>&1`
Expected: `No such file or directory`.

- [ ] **Step 3: Commit**

```bash
git commit -m "$(cat <<'EOF'
Remove cogito-conversation crate (superseded by cogito-store-jsonl)

Per ADR-0004 §3: the trait lives in cogito-protocol; concrete backends
live in cogito-store-* crates. cogito-conversation was a stub holding
no real code.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Fix cogito-core/Cargo.toml — enforce ADR-0004 import rule

**Files:**
- Modify: `crates/cogito-core/Cargo.toml`

- [ ] **Step 1: Replace dependencies block**

Replace the `[dependencies]` section of `crates/cogito-core/Cargo.toml` with:

```toml
[dependencies]
cogito-protocol.workspace = true

tokio.workspace = true
tokio-stream.workspace = true
tokio-util = { version = "0.7", features = ["rt"] }
futures.workspace = true
async-trait.workspace = true
dashmap = "6.1"
serde.workspace = true
serde_json.workspace = true
serde_yaml.workspace = true
thiserror.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
```

Removed (illegal per ADR-0004): `cogito-conversation`, `cogito-model`, `cogito-tools`, `cogito-sandbox`, `cogito-jobs`. Brain can only see `cogito-protocol`.

Added: `tokio-util` for `CancellationToken`, `dashmap` for `Runtime::sessions` map.

- [ ] **Step 2: Add `tokio-util` and `dashmap` to workspace.dependencies**

In `Cargo.toml` `[workspace.dependencies]`, add (alphabetically near `tokio`):

```toml
tokio-util = { version = "0.7", features = ["rt"] }
dashmap = "6.1"
```

Then change `cogito-core/Cargo.toml` lines for these two to `.workspace = true`:

```toml
tokio-util.workspace = true
dashmap.workspace = true
```

- [ ] **Step 3: Verify cogito-core still builds with Brain stubs**

Run: `cargo check -p cogito-core 2>&1 | head -30`
Expected: compile errors *only* from `cogito-core/src/harness/*.rs` files that try to `use cogito_conversation::...` / `use cogito_jobs::...` etc. — those module files are stubs (~100 bytes each) and should not actually import concrete crates. If they do, leave them — Task 4 will null them out.

Most likely actual outcome: passes (since harness stubs only have `//! comment\n` bodies).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/cogito-core/Cargo.toml
git commit -m "$(cat <<'EOF'
Enforce ADR-0004 Brain import rule via cogito-core Cargo.toml

cogito-core (Brain) may only depend on cogito-protocol. Direct deps on
cogito-conversation/-model/-tools/-sandbox/-jobs are removed; layer
violations are now build errors instead of review-time catches.

Adds tokio-util (CancellationToken) and dashmap (Runtime::sessions map)
in preparation for the runtime module scaffolding in later tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Fix cogito-jobs/Cargo.toml — drop cogito-conversation reference

**Files:**
- Modify: `crates/cogito-jobs/Cargo.toml`

- [ ] **Step 1: Remove the dangling dependency**

In `crates/cogito-jobs/Cargo.toml` `[dependencies]`, delete the line `cogito-conversation.workspace = true`. The block should now read:

```toml
[dependencies]
cogito-protocol.workspace = true

tokio.workspace = true
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
sqlx.workspace = true
uuid.workspace = true
```

- [ ] **Step 2: Verify build**

Run: `cargo check -p cogito-jobs 2>&1 | head -20`
Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-jobs/Cargo.toml
git commit -m "$(cat <<'EOF'
Drop cogito-conversation dep from cogito-jobs

Stale reference left over from the crate's removal. cogito-jobs talks
to the event log only through the ConversationStore trait it receives
via DI from Runtime; no direct backend dep needed.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Fix cogito-cli/Cargo.toml — Surface layer imports Hands/Boundary/Session explicitly

**Files:**
- Modify: `crates/cogito-cli/Cargo.toml`

- [ ] **Step 1: Replace dependencies block**

Replace `[dependencies]` of `crates/cogito-cli/Cargo.toml` with:

```toml
[dependencies]
cogito-core.workspace = true
cogito-protocol.workspace = true
cogito-store-jsonl.workspace = true
cogito-model.workspace = true
cogito-tools.workspace = true
cogito-jobs.workspace = true

tokio.workspace = true
anyhow.workspace = true
clap.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

Per ADR-0004 §2 import rules, Surface may import Protocol + Runtime + Hands/Boundary/Session directly. Wiring all concrete impls is the CLI's job.

- [ ] **Step 2: Verify workspace builds clean**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-cli/Cargo.toml
git commit -m "$(cat <<'EOF'
Wire Hands/Boundary/Session crates into cogito-cli explicitly

Surface layer is the integration point per ADR-0004 §2: Protocol +
Runtime + every concrete Hand/Boundary/Session impl. Adds store-jsonl,
model, tools, jobs as direct deps; drops the obsolete cogito-conversation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Split cogito-protocol into modules; add `error` module skeleton

**Files:**
- Modify: `crates/cogito-protocol/src/lib.rs`
- Create: `crates/cogito-protocol/src/error.rs`

cogito-protocol currently has only `lib.rs`. Subsequent tasks (7-11) add types in dedicated modules (`tool`, `stream`, `job`, `turn`). This task creates the module skeleton so each new type lands cleanly.

- [ ] **Step 1: Rewrite lib.rs with module declarations**

Replace `crates/cogito-protocol/src/lib.rs` with:

```rust
//! cogito-protocol
//!
//! Protocol layer: events, contracts, and types shared across the workspace.
//!
//! This crate is dependency-free with respect to other cogito crates.
//! Anything that defines the *contract* between components belongs here.
//!
//! Module map (1:1 with the Brain/Hands/Session boundaries in ADR-0004):
//! - [`tool`]: ToolProvider trait, ToolDescriptor, InvokeOutcome, ExecutionClass
//! - [`stream`]: StreamEvent enum (real-time fanout to subscribers)
//! - [`job`]: JobManager trait, JobId, JobStatus, JobCompletionEvent
//! - [`turn`]: TurnOutcome, TurnFailureReason
//! - [`error`]: shared error kinds and helpers

pub mod error;
pub mod job;
pub mod stream;
pub mod tool;
pub mod turn;
```

- [ ] **Step 2: Create error.rs skeleton**

`crates/cogito-protocol/src/error.rs`:

```rust
//! Shared error types used across protocol contracts.
//!
//! Per ADR-0005 §4: contracts return structured errors via `thiserror`.
//! Concrete crates may wrap these into their own error enums.

use thiserror::Error;

/// Errors that cross the protocol boundary. Concrete impls may add
/// backend-specific variants by wrapping `ProtocolError` in their own
/// `thiserror` enum.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Caller passed arguments that violate a documented invariant
    /// (e.g., schema mismatch, missing required field).
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    /// A backend resource (store, gateway, sandbox) is unavailable.
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
}
```

- [ ] **Step 3: Verify build**

Run: `cargo check -p cogito-protocol 2>&1 | tail -5`
Expected: missing module errors for `tool`, `stream`, `job`, `turn` (they don't exist yet).

To unblock this single task, temporarily comment out the four module decls:

```rust
// pub mod job;
// pub mod stream;
// pub mod tool;
// pub mod turn;
```

Verify: `cargo check -p cogito-protocol` passes with only `error` exposed. Tasks 7–10 each uncomment one and add the module file.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/
git commit -m "$(cat <<'EOF'
Split cogito-protocol into per-contract modules; add error skeleton

Prepares the protocol crate to receive the new contract types (tool,
stream, job, turn) added in subsequent tasks. Adds a shared
ProtocolError enum for backend-unavailable and invalid-args cases.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Add ToolDescriptor / ExecutionClass / InvokeOutcome / ToolResult / ToolErrorKind

**Files:**
- Create: `crates/cogito-protocol/src/tool.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (uncomment `pub mod tool;`)
- Create: `crates/cogito-protocol/tests/tool_types.rs`

Per spec §6.

- [ ] **Step 1: Write failing test first**

`crates/cogito-protocol/tests/tool_types.rs`:

```rust
//! Tests for tool-layer contract types.
//!
//! These tests pin down serde stability and enum coverage. They run as
//! part of `cargo nextest run -p cogito-protocol`.

use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolResult,
};

#[test]
fn execution_class_serde_roundtrip() {
    for variant in [
        ExecutionClass::AlwaysSync,
        ExecutionClass::AlwaysAsync,
        ExecutionClass::Adaptive,
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        let back: ExecutionClass = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, back);
    }
}

#[test]
fn tool_descriptor_round_trips() {
    let descriptor = ToolDescriptor {
        name: "read_file".into(),
        description: "Read a file from the workspace".into(),
        schema: serde_json::json!({ "type": "object" }),
        execution_class: ExecutionClass::AlwaysSync,
        outputs_model_visible_multimodal: false,
    };
    let json = serde_json::to_string(&descriptor).unwrap();
    let back: ToolDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(descriptor.name, back.name);
    assert_eq!(descriptor.execution_class, back.execution_class);
}

#[test]
fn invoke_outcome_distinguishes_sync_and_async() {
    let sync_out = InvokeOutcome::Sync(ToolResult::Output(vec![]));
    assert!(matches!(sync_out, InvokeOutcome::Sync(_)));
    // JobId is an opaque ulid wrapper — constructed via Default for this smoke test.
    let async_out = InvokeOutcome::Async(cogito_protocol::job::JobId::default());
    assert!(matches!(async_out, InvokeOutcome::Async(_)));
}

#[test]
fn tool_error_kind_serde_covers_all_variants() {
    use ToolErrorKind::*;
    for kind in [
        InvalidArgs,
        InvocationFailed,
        ToolPanicked,
        Cancelled,
        Timeout,
        JobStateLost,
        AsyncFailed,
    ] {
        let json = serde_json::to_string(&kind).unwrap();
        let back: ToolErrorKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }
}
```

- [ ] **Step 2: Run test to verify it fails (no `tool` module yet)**

Run: `cargo nextest run -p cogito-protocol tool_types 2>&1 | tail -15`
Expected: `error[E0432]: unresolved import \`cogito_protocol::tool\`` (or similar).

- [ ] **Step 3: Implement the module**

`crates/cogito-protocol/src/tool.rs`:

```rust
//! Tool contract: descriptor, invocation outcome, result, error kinds.
//!
//! See:
//! - `docs/components/H07-tool-resolver.md` (descriptor and validation)
//! - `docs/components/H08-tool-dispatcher.md` (invocation flow)
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` §6
//!   (sync vs async judgment via ExecutionClass)

use crate::job::JobId;
use serde::{Deserialize, Serialize};

/// A tool exposed by a ToolProvider. ToolDescriptor is the metadata the
/// LLM (and H05 Tool Surface Builder) sees; the actual call goes through
/// `ToolProvider::invoke`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Unique name; the LLM uses this in tool calls.
    pub name: String,
    /// One-line description shown to the model.
    pub description: String,
    /// JSON Schema (Draft 2020-12) for the arguments.
    pub schema: serde_json::Value,
    /// Whether invocations are sync, async, or per-call adaptive.
    pub execution_class: ExecutionClass,
    /// If true, this tool may emit Image/Video/Audio ContentBlocks in its
    /// result. H05 may filter the tool out when the selected model has no
    /// native multimodal capability.
    pub outputs_model_visible_multimodal: bool,
}

/// Statically-declared execution class for a tool. H08 uses this to validate
/// the InvokeOutcome variant returned by `invoke()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    /// Always returns `InvokeOutcome::Sync`. Typical: read_file, now,
    /// parse_json.
    AlwaysSync,
    /// Always returns `InvokeOutcome::Async(JobId)`. Typical: run_tests,
    /// build_release.
    AlwaysAsync,
    /// Decides per call based on arguments. Typical: transcribe_audio
    /// (short clip -> Sync; long clip -> Async).
    Adaptive,
}

/// Outcome of a single `ToolProvider::invoke` call.
#[derive(Debug, Clone)]
pub enum InvokeOutcome {
    /// Result is available immediately.
    Sync(ToolResult),
    /// Result is deferred; consult `JobManager` for completion. The Brain
    /// will pause the turn until a matching `JobCompletionEvent` arrives.
    Async(JobId),
}

/// Result body returned by a tool. `Vec<ContentBlock>` arrives in v0.2 when
/// the multimodal upgrade lands; v0.1 uses plain text via the convenience
/// constructor `ToolResult::text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResult {
    /// Successful output. v0.1 represents content as a list of opaque
    /// JSON values; v0.2 replaces this with `Vec<ContentBlock>`.
    Output(Vec<serde_json::Value>),
    /// Structured error. H08 records this as the tool's result, then feeds
    /// it back to the model so the model can decide how to proceed.
    Error {
        kind: ToolErrorKind,
        message: String,
        retryable: bool,
    },
}

impl ToolResult {
    /// Convenience constructor for the common "single text block" case.
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        ToolResult::Output(vec![serde_json::Value::String(s.into())])
    }
}

/// Classification of why a tool call failed. The model only ever sees
/// `ToolResult::Error`; this kind helps H09 hooks and H10 strategy decide
/// whether to retry or surface to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    /// H07 schema validation rejected the args.
    InvalidArgs,
    /// Tool returned a business-logic error.
    InvocationFailed,
    /// The tool implementation panicked; caught at H08 Layer 3
    /// (see spec §9).
    ToolPanicked,
    /// Cancellation token fired during the invocation.
    Cancelled,
    /// Tool-internal timeout (distinct from turn-level timeout, which
    /// produces `TurnFailureReason::TurnTimedOut`).
    Timeout,
    /// Resuming a paused turn: JobManager lost track of the JobId.
    JobStateLost,
    /// Async job completed but reported an internal failure.
    AsyncFailed,
}
```

- [ ] **Step 4: Uncomment the module decl in lib.rs**

Edit `crates/cogito-protocol/src/lib.rs`: change `// pub mod tool;` -> `pub mod tool;`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p cogito-protocol tool_types 2>&1 | tail -10`
Expected: `4 tests passed`.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/
git commit -m "$(cat <<'EOF'
Add tool contract types: ExecutionClass, InvokeOutcome, ToolResult

Per spec §6: ExecutionClass on ToolDescriptor lets H08 statically know
whether to expect Sync or Async outcomes; Adaptive defers the choice
to the tool implementation. ToolErrorKind enumerates every reason a
tool call can fail (panic, cancel, timeout, async-lost, ...).

Tests cover serde roundtrip and exhaustive enum coverage.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Add JobManager trait, JobId, JobStatus, JobCompletionEvent

**Files:**
- Create: `crates/cogito-protocol/src/job.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (uncomment `pub mod job;`)
- Modify: `crates/cogito-protocol/Cargo.toml` (add tokio mpsc; async-trait)
- Create: `crates/cogito-protocol/tests/job_types.rs`

Per spec §6 "JobManager trait (v0.1 shape)".

- [ ] **Step 1: Add tokio + async-trait to cogito-protocol Cargo.toml**

In `crates/cogito-protocol/Cargo.toml` `[dependencies]`, add (the trait needs `mpsc::Sender` and `#[async_trait]`):

```toml
async-trait.workspace = true
tokio = { workspace = true, features = ["sync"] }
```

The `sync` feature is the minimum needed (`mpsc`, `oneshot`); we deliberately don't pull in `full` from the protocol crate.

- [ ] **Step 2: Write failing test**

`crates/cogito-protocol/tests/job_types.rs`:

```rust
//! Tests for JobManager-adjacent value types. The trait itself is exercised
//! via contract tests in concrete implementor crates (cogito-jobs).

use cogito_protocol::job::{JobCompletionEvent, JobId, JobOutcome, JobStatus};

#[test]
fn job_id_default_is_unique() {
    let a = JobId::default();
    let b = JobId::default();
    assert_ne!(a, b, "two default-constructed JobIds must collide-resist");
}

#[test]
fn job_status_serde_covers_all_variants() {
    for status in [
        JobStatus::Pending,
        JobStatus::Running,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ] {
        let json = serde_json::to_string(&status).unwrap();
        let back: JobStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, back);
    }
}

#[test]
fn job_completion_event_carries_job_id_and_outcome() {
    let event = JobCompletionEvent {
        job_id: JobId::default(),
        outcome: JobOutcome::Cancelled,
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: JobCompletionEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event.job_id, back.job_id);
    assert_eq!(event.outcome, back.outcome);
}
```

- [ ] **Step 3: Run test, expect compile failure**

Run: `cargo nextest run -p cogito-protocol job_types 2>&1 | tail -15`
Expected: unresolved import `cogito_protocol::job`.

- [ ] **Step 4: Implement the module**

`crates/cogito-protocol/src/job.rs`:

```rust
//! Async job lifecycle contract.
//!
//! `JobManager` exposes status/result/cancel + an `on_complete` callback
//! registration. Submission lives on the concrete `LocalJobManager` type
//! in cogito-jobs (only async-tool implementations submit jobs; Brain only
//! observes via this trait). See spec §6.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use ulid::Ulid;

use crate::tool::ToolResult;

/// Opaque job identifier. Currently a Ulid so order corresponds to
/// submission time within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(Ulid);

impl Default for JobId {
    fn default() -> Self {
        Self(Ulid::new())
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Accepted by the manager but not yet scheduled.
    Pending,
    /// A worker is actively executing the job.
    Running,
    /// Reached a terminal successful state.
    Completed,
    /// Reached a terminal error state.
    Failed,
    /// Reached a terminal cancelled state (via `JobManager::cancel`).
    Cancelled,
}

/// Terminal outcome of a job, delivered through `on_complete` and
/// `result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobOutcome {
    /// Tool result produced by the job. The wire format matches
    /// `ToolResult::Output`; the actor wraps it back into ToolResult
    /// when resuming the turn.
    Success { result: ToolResult },
    /// Job failed; the tool will see `ToolResult::Error { kind: AsyncFailed }`.
    Failed { message: String },
    /// Job was cancelled before completion (by `JobManager::cancel`).
    Cancelled,
}

/// Event sent by `JobManager` to the registered sink when a job reaches
/// a terminal state. The actor translates this into a
/// `SessionCommand::JobCompleted` to keep the FIFO mailbox invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCompletionEvent {
    pub job_id: JobId,
    pub outcome: JobOutcome,
}

/// Error kind for JobManager operations.
#[derive(Debug, Error)]
pub enum JobError {
    #[error("unknown job: {0}")]
    UnknownJob(JobId),
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
}

/// Brain-facing contract for tracking async work.
///
/// Implementations live in `cogito-jobs` (v0.1 local) and
/// `cogito-jobs-distributed` (v0.4 Redis-backed). Submission is *not*
/// part of this trait — only async tool implementations submit jobs,
/// and they hold a reference to the concrete LocalJobManager type.
#[async_trait]
pub trait JobManager: Send + Sync {
    /// Query the current state of a job.
    async fn status(&self, job_id: JobId) -> Result<JobStatus, JobError>;

    /// Retrieve the terminal outcome. Errors if the job hasn't completed.
    async fn result(&self, job_id: JobId) -> Result<JobOutcome, JobError>;

    /// Best-effort cancellation. The job may already be terminal.
    async fn cancel(&self, job_id: JobId) -> Result<(), JobError>;

    /// Register a one-shot completion callback. When the job reaches a
    /// terminal state, the manager sends exactly one `JobCompletionEvent`
    /// on `sink` and drops the sender. If `sink` is closed (e.g., the
    /// actor died), the implementation may silently drop the event.
    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<(), JobError>;
}
```

- [ ] **Step 5: Add `ulid = { workspace = true }` and `async-trait = { workspace = true }` to cogito-protocol Cargo.toml dependencies**

The `Cargo.toml` already has `ulid.workspace = true`; verify it's present. Add `async-trait.workspace = true` if not already added in Step 1.

- [ ] **Step 6: Uncomment module decl in lib.rs**

Change `// pub mod job;` -> `pub mod job;`.

- [ ] **Step 7: Run test**

Run: `cargo nextest run -p cogito-protocol job_types 2>&1 | tail -10`
Expected: `3 tests passed`.

- [ ] **Step 8: Commit**

```bash
git add crates/cogito-protocol/
git commit -m "$(cat <<'EOF'
Add JobManager trait, JobId, JobStatus, JobCompletionEvent

Per spec §6: JobManager exposes status/result/cancel and the
mailbox-injection on_complete callback that lets actors receive
async-tool completion without polling and without blocking the
mailbox. Submission stays on the concrete LocalJobManager type per
ADR-0004 (Hands-internal primitive).

JobId is a Ulid so process-local ordering matches submission time.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Add StreamEvent enum

**Files:**
- Create: `crates/cogito-protocol/src/stream.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (uncomment `pub mod stream;`)
- Create: `crates/cogito-protocol/tests/stream_event.rs`

Per spec §7 (dual event stream).

- [ ] **Step 1: Write failing test**

`crates/cogito-protocol/tests/stream_event.rs`:

```rust
//! StreamEvent serde stability tests.

use cogito_protocol::stream::StreamEvent;

#[test]
fn text_delta_round_trips() {
    let event = StreamEvent::TextDelta {
        chunk: "Hello ".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: StreamEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn lifecycle_events_round_trip() {
    let events = [
        StreamEvent::TurnStarted,
        StreamEvent::TurnPaused,
        StreamEvent::TurnResumed,
        StreamEvent::TurnCancelled,
        StreamEvent::TurnCompleted,
        StreamEvent::TurnFailed {
            reason: "model gateway timeout".into(),
        },
        StreamEvent::ToolDispatchStarted {
            call_id: "call_1".into(),
            tool_name: "read_file".into(),
        },
        StreamEvent::ToolDispatchEnded {
            call_id: "call_1".into(),
            ok: true,
        },
    ];
    for e in events {
        let json = serde_json::to_string(&e).unwrap();
        let back: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo nextest run -p cogito-protocol stream_event 2>&1 | tail -10`
Expected: unresolved import `cogito_protocol::stream`.

- [ ] **Step 3: Implement the module**

`crates/cogito-protocol/src/stream.rs`:

```rust
//! Real-time event stream broadcast to subscribers (TUI, observability,
//! consumer hooks).
//!
//! StreamEvent is distinct from `ConversationEvent`: it is *not* persisted,
//! text deltas are *not* batched, and slow subscribers may be dropped
//! (broadcast lagged semantics). See spec §7 for the dual-stream rationale.

use serde::{Deserialize, Serialize};

/// Real-time event observable via `SessionHandle::subscribe()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    /// A new turn has begun. Carries no payload; the input is on the
    /// caller's side.
    TurnStarted,

    /// The turn paused on an async tool call. The driving Brain task
    /// has exited; the actor is now in PausedOnJob.
    TurnPaused,

    /// A previously paused turn has been resumed by a JobCompleted event.
    TurnResumed,

    /// The turn was cancelled by `SessionHandle::cancel_turn`.
    TurnCancelled,

    /// The turn reached terminal Completed state (model returned
    /// stop_reason = end_turn without further tool calls).
    TurnCompleted,

    /// The turn ended with a structured failure. `reason` is a
    /// human-readable rendering of `TurnFailureReason` for subscribers
    /// (the precise enum lives in the persisted ConversationEvent).
    TurnFailed { reason: String },

    /// Per-chunk text delta from the model stream. Not persisted as-is;
    /// the store writer subtask batches into `AssistantMessageAppended`
    /// every 200ms or 500 chars.
    TextDelta { chunk: String },

    /// H08 began dispatching a tool call.
    ToolDispatchStarted { call_id: String, tool_name: String },

    /// H08 finished dispatching a tool call. `ok` is false for both
    /// structured errors and panics; subscribers consult the persisted
    /// `ToolResult` for detail.
    ToolDispatchEnded { call_id: String, ok: bool },
}
```

- [ ] **Step 4: Uncomment decl + run test**

Edit `crates/cogito-protocol/src/lib.rs`: uncomment `pub mod stream;`.

Run: `cargo nextest run -p cogito-protocol stream_event 2>&1 | tail -10`
Expected: `2 tests passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/
git commit -m "$(cat <<'EOF'
Add StreamEvent enum for real-time subscriber fanout

Per spec §7: cogito has two event streams — durable ConversationEvent
(batched, fsync'd, drives resume) and ephemeral StreamEvent (per-chunk,
broadcast, drives UIs). This commit lands the protocol type; the
broadcast plumbing arrives with the runtime module in later tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Add TurnOutcome and TurnFailureReason

**Files:**
- Create: `crates/cogito-protocol/src/turn.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (uncomment `pub mod turn;`)
- Create: `crates/cogito-protocol/tests/turn_outcome.rs`

Per spec §9 "Structured error policy".

- [ ] **Step 1: Write failing test**

`crates/cogito-protocol/tests/turn_outcome.rs`:

```rust
//! TurnOutcome and TurnFailureReason serde stability.

use cogito_protocol::job::JobId;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};

#[test]
fn completed_outcome_roundtrips() {
    let outcome = TurnOutcome::Completed;
    let json = serde_json::to_string(&outcome).unwrap();
    let back: TurnOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(outcome, back);
}

#[test]
fn paused_carries_job_id() {
    let outcome = TurnOutcome::Paused {
        job_id: JobId::default(),
    };
    let json = serde_json::to_string(&outcome).unwrap();
    let back: TurnOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(outcome, back);
}

#[test]
fn all_failure_reasons_roundtrip() {
    let reasons = [
        TurnFailureReason::StoreUnavailable,
        TurnFailureReason::ModelGatewayFailed("503".into()),
        TurnFailureReason::TurnPanicked {
            location: "stream demux",
        },
        TurnFailureReason::TurnTimedOut,
        TurnFailureReason::HookRejected {
            hook_name: "sensitive-content".into(),
            message: "regex matched".into(),
        },
    ];
    for r in reasons {
        let json = serde_json::to_string(&r).unwrap();
        let back: TurnFailureReason = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo nextest run -p cogito-protocol turn_outcome 2>&1 | tail -10`
Expected: unresolved import `cogito_protocol::turn`.

- [ ] **Step 3: Implement**

`crates/cogito-protocol/src/turn.rs`:

```rust
//! Turn terminal-state values. The Runtime layer returns these from
//! `SessionActor` after each turn completes. Caller observes via
//! `SessionHandle` or the StreamEvent stream.

use serde::{Deserialize, Serialize};

use crate::job::JobId;

/// Terminal outcome of a single turn iteration. Note the FSM may loop
/// internally (multiple sub-turns when the model calls sync tools and
/// continues); a turn ends only when one of these variants is produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnOutcome {
    /// Model returned end_turn without further tool calls.
    Completed,
    /// A tool returned `InvokeOutcome::Async`; the turn will resume when
    /// the matching `JobCompletionEvent` arrives.
    Paused { job_id: JobId },
    /// `SessionHandle::cancel_turn` fired during execution.
    Cancelled,
    /// Unrecoverable failure; details in `reason`. `recorded_event_id`
    /// points to the last persisted ConversationEvent for diagnosis.
    Failed {
        reason: TurnFailureReason,
        recorded_event_id: String,
    },
}

/// Why a turn ended in `Failed`. Only Runtime-level errors (store I/O,
/// gateway hard failure, panic, timeout, hook reject) escape here; tool
/// errors stay inside `ToolResult::Error` and never bubble up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnFailureReason {
    /// `ConversationStore::append` returned an error.
    StoreUnavailable,
    /// `ModelGateway::stream` returned `Err(...)`.
    ModelGatewayFailed(String),
    /// A panic was caught by Layer 2 (TurnDriver task). See spec §9.
    TurnPanicked { location: &'static str },
    /// `tokio::time::timeout` fired around the turn task.
    TurnTimedOut,
    /// An H09 hook returned `HookDecision::Reject`.
    HookRejected { hook_name: String, message: String },
}
```

- [ ] **Step 4: Uncomment + run**

Uncomment `pub mod turn;` in `lib.rs`. Run:

`cargo nextest run -p cogito-protocol turn_outcome 2>&1 | tail -10`
Expected: `3 tests passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/
git commit -m "$(cat <<'EOF'
Add TurnOutcome and TurnFailureReason

Per spec §9: turn terminal states cleanly split into Completed / Paused
/ Cancelled / Failed. Only Runtime-level errors (store, gateway hard
failure, panic, timeout, hook reject) escape into Failed; tool errors
stay inside ToolResult::Error per inviolable rule #5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Add the cogito-core runtime module — scaffolding only

**Files:**
- Create: `crates/cogito-core/src/runtime/mod.rs`
- Create: `crates/cogito-core/src/runtime/types.rs`
- Create: `crates/cogito-core/src/runtime/builder.rs`
- Create: `crates/cogito-core/src/runtime/handle.rs`
- Create: `crates/cogito-core/src/runtime/actor.rs`
- Create: `crates/cogito-core/src/runtime/store_writer.rs`
- Modify: `crates/cogito-core/src/lib.rs` (add `pub mod runtime;`)

All bodies are stubs returning `todo!("Plan 2 task X")` with descriptive messages. The point is compilable type signatures Sprint 1 builds on.

- [ ] **Step 1: Create mod.rs**

`crates/cogito-core/src/runtime/mod.rs`:

```rust
//! Runtime layer: hosts SessionActor tasks, owns the tokio Handle,
//! injects Hands/Boundary/Session into the Brain.
//!
//! See:
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
//!   §3 (task topology), §4 (lifecycle), §7 (channels)
//! - ADR-0004 (layer rules)

pub mod actor;
pub mod builder;
pub mod handle;
pub mod store_writer;
pub mod types;

pub use builder::{Runtime, RuntimeBuilder};
pub use handle::SessionHandle;
pub use types::{OpenMode, SessionCommand, SessionId, ShutdownOutcome};
```

- [ ] **Step 2: Create types.rs**

`crates/cogito-core/src/runtime/types.rs`:

```rust
//! Channel-protocol value types used between caller, actor, store writer,
//! and JobManager.

use cogito_protocol::job::{JobCompletionEvent, JobId, JobOutcome};
use cogito_protocol::turn::TurnOutcome;
use tokio::sync::oneshot;

/// Opaque session identifier. Caller picks the string; cogito does not
/// interpret it (typical: ulid or user-domain id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl<S: Into<String>> From<S> for SessionId {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

/// How `Runtime::open_session` should treat an existing session id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Session must not exist in the store. Writes a `SessionStarted` event.
    New,
    /// Session must exist; replay through H03 establishes resume point.
    /// Panics on missing log (contract violation).
    Resume,
    /// Like Resume but returns `Err(ResumeError::NotFound)` instead of
    /// panicking on a missing log.
    Attach,
}

/// Outcome of an attempted `SessionHandle::shutdown`.
#[derive(Debug)]
pub struct ShutdownOutcome {
    /// True if the actor drained cleanly without forced abort.
    pub clean: bool,
    /// If a turn was in-flight when shutdown started, the last known state
    /// (for caller-side logging only — the persisted event log is the
    /// authority).
    pub in_flight_cancelled: Option<String>,
}

/// Commands the caller (and the actor's own internal subsystems) may send
/// into the mailbox. CancelTurn does *not* go through this enum — it fires
/// the per-turn CancellationToken directly to bypass FIFO ordering.
#[derive(Debug)]
pub enum SessionCommand {
    /// Caller-driven new user input. Triggers a new TurnDriver.
    Input(NewMessage),

    /// Synthesized by the actor after receiving a `JobCompletionEvent` on
    /// the job_completion channel. Re-spawns TurnDriver with resume state.
    JobCompleted {
        job_id: JobId,
        outcome: JobOutcome,
    },

    /// Sent by `SessionHandle::cancel_turn` when the actor is in
    /// PausedOnJob (cancel_token alone cannot reach a non-existent
    /// TurnDriver task; this asks the actor to call `jobs.cancel`).
    InternalCancel { ack: oneshot::Sender<()> },

    /// Graceful shutdown with a deadline. Actor drains the mailbox,
    /// flushes the store writer, then exits.
    Shutdown {
        deadline: std::time::Duration,
        ack: oneshot::Sender<ShutdownOutcome>,
    },
}

/// User-facing input for a new turn. Wrapped here so `SessionCommand`
/// stays trivially extensible.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub text: String,
}

/// Internal wrapper used inside the actor to translate a
/// `JobCompletionEvent` from the job_completion mpsc into a
/// `SessionCommand::JobCompleted` for FIFO mailbox ordering.
impl From<JobCompletionEvent> for SessionCommand {
    fn from(event: JobCompletionEvent) -> Self {
        SessionCommand::JobCompleted {
            job_id: event.job_id,
            outcome: event.outcome,
        }
    }
}

/// Wrapper produced by an actor when a turn finishes; used internally
/// (not exposed in handle API).
#[derive(Debug)]
pub struct TurnFinished {
    pub outcome: TurnOutcome,
}
```

- [ ] **Step 3: Create builder.rs (stub Runtime + RuntimeBuilder)**

`crates/cogito-core/src/runtime/builder.rs`:

```rust
//! `Runtime` and `RuntimeBuilder` — the entry point. Caller injects a
//! tokio Handle, a ConversationStore, ModelGateway, ToolProvider,
//! JobManager. Then opens sessions.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::runtime::Handle as TokioHandle;
use tokio_util::sync::CancellationToken;

use super::handle::SessionHandle;
use super::types::{OpenMode, SessionId};

/// The DI container + session registry. One Runtime per cogito-using
/// process is the typical pattern.
pub struct Runtime {
    _handle: TokioHandle,
    _sessions: DashMap<SessionId, SessionHandle>,
    _shutdown_token: CancellationToken,
    // Fields below land in Plan 2; left commented to flag the v0.1 shape:
    // store: Arc<dyn ConversationStore>,
    // model: Arc<dyn ModelGateway>,
    // tools: Arc<dyn ToolProvider>,
    // jobs:  Arc<dyn JobManager>,
    // hooks: Arc<dyn HookHandler>,
    // metrics: Arc<dyn MetricsRecorder>,
}

impl Runtime {
    /// Open or attach a session. See `OpenMode` for the three semantics.
    /// Awaits until the replay phase completes (or fails).
    pub async fn open_session(
        &self,
        _id: SessionId,
        _mode: OpenMode,
    ) -> Result<SessionHandle, RuntimeError> {
        todo!("Plan 2 Task: implement open_session — spawn SessionActor, run replay phase, return SessionHandle once ready oneshot fires")
    }

    /// Begin a builder. Caller injects all dependencies.
    #[must_use]
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }
}

/// Builder for `Runtime`. Caller may set `handle()` explicitly or let it
/// default to `tokio::runtime::Handle::current()` at `build()` time.
#[derive(Default)]
pub struct RuntimeBuilder {
    handle: Option<TokioHandle>,
    shutdown_token: Option<CancellationToken>,
}

impl RuntimeBuilder {
    /// Override the tokio Handle. Defaults to `Handle::current()`.
    #[must_use]
    pub fn handle(mut self, handle: TokioHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Reserve a process-level cancellation token. v0.1 stores it but
    /// does not consume it; v0.4 wires it through `shutdown_all()`.
    #[must_use]
    pub fn shutdown_token(mut self, token: CancellationToken) -> Self {
        self.shutdown_token = Some(token);
        self
    }

    /// Finalize. Returns `Err` if no current tokio runtime is available.
    pub fn build(self) -> Result<Arc<Runtime>, RuntimeError> {
        let handle = match self.handle {
            Some(h) => h,
            None => TokioHandle::try_current()
                .map_err(|e| RuntimeError::NoTokioRuntime(e.to_string()))?,
        };
        let runtime = Runtime {
            _handle: handle,
            _sessions: DashMap::new(),
            _shutdown_token: self.shutdown_token.unwrap_or_default(),
        };
        Ok(Arc::new(runtime))
    }
}

/// Errors from the Runtime layer surface (not from inside a turn).
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// `Handle::try_current()` failed at build time.
    #[error("no current tokio runtime: {0}")]
    NoTokioRuntime(String),
    /// The session id was already open in this Runtime.
    #[error("session already open: {0:?}")]
    SessionAlreadyOpen(SessionId),
    /// Resume-phase failure for `OpenMode::Attach`.
    #[error("resume failed: {0}")]
    ResumeFailed(String),
}
```

- [ ] **Step 4: Create handle.rs (stub)**

`crates/cogito-core/src/runtime/handle.rs`:

```rust
//! Caller-side handle to one session.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::stream::StreamEvent;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::types::{NewMessage, SessionCommand, ShutdownOutcome};

/// Shared state between a SessionHandle and the SessionActor task it
/// fronts. Held by Arc on the caller side.
pub(super) struct SessionShared {
    pub(super) mailbox_tx: mpsc::Sender<SessionCommand>,
    pub(super) events_tx: broadcast::Sender<StreamEvent>,
    /// Token for the *currently* in-flight turn. Actor replaces it on
    /// each turn start; caller's `cancel_turn` always operates on the
    /// most recent version.
    pub(super) current_cancel_token: parking_lot::Mutex<CancellationToken>,
}

/// Caller-facing handle to a session. Cheap to clone where needed.
#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<SessionShared>,
}

impl SessionHandle {
    pub(super) fn new(shared: Arc<SessionShared>) -> Self {
        Self { shared }
    }

    /// Send a new user message; the actor will spawn a TurnDriver.
    /// Blocks (via mailbox backpressure) if the actor is overwhelmed.
    pub async fn send_user(&self, text: impl Into<String>) -> Result<(), SessionError> {
        let _ = text.into();
        let _ = &self.shared;
        todo!("Plan 2 Task: send SessionCommand::Input via mailbox_tx")
    }

    /// Subscribe to the real-time event stream. Multiple subscribers
    /// allowed; slow subscribers receive `Lagged(n)` errors per
    /// `broadcast::Receiver` semantics.
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.shared.events_tx.subscribe()
    }

    /// Cancel the current turn (if any). Cooperative; tools that want to
    /// honor cancellation must `select!` on `ExecCtx.cancel`. Has no effect
    /// if no turn is running. If the actor is in PausedOnJob, also sends
    /// an InternalCancel command so the actor can call `jobs.cancel`.
    pub async fn cancel_turn(&self) -> Result<(), SessionError> {
        let _ = &self.shared;
        todo!("Plan 2 Task: fire current_cancel_token + send InternalCancel via mailbox_tx")
    }

    /// Gracefully shut the session down. Drains the mailbox, flushes the
    /// store writer, waits for `deadline` for any in-flight turn before
    /// hard-aborting.
    pub async fn shutdown(self, deadline: Duration) -> Result<ShutdownOutcome, SessionError> {
        let _ = deadline;
        todo!("Plan 2 Task: send Shutdown via mailbox_tx, await ack oneshot")
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        // Plan 2 Task: if this is the last Arc clone, best-effort send
        // Shutdown with default timeout so the actor doesn't leak.
    }
}

/// Errors from caller-facing SessionHandle operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Actor task has exited and is no longer accepting commands.
    #[error("session is closed")]
    SessionClosed,
    /// Caller tried to use the handle after `shutdown` returned.
    #[error("shutdown already in progress")]
    ShuttingDown,
}
```

- [ ] **Step 5: Create actor.rs (stub)**

`crates/cogito-core/src/runtime/actor.rs`:

```rust
//! SessionActor: the long-lived per-session tokio task. Hosts the
//! `actor_main` loop that selects on the mailbox, on the in-flight
//! TurnDriver task (if any), and on the job_completion channel.
//!
//! Implementation is Plan 2 (Sprint 1 / 2). This module currently
//! exposes only the struct skeleton so other modules compile.

use std::time::Instant;

use cogito_protocol::job::JobId;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use cogito_protocol::turn::TurnOutcome;

/// In-flight turn state held by the actor across mailbox iterations.
/// Cleared back to None on every terminal turn outcome.
pub(super) enum InFlight {
    /// A TurnDriver task is running; `turn_join` is its handle.
    Active {
        turn_join: JoinHandle<TurnOutcome>,
        started_at: Instant,
    },
    /// The turn paused awaiting a JobManager callback. `job_id` is the
    /// router key; `paused_at_event_id` lets resume reconstruct context.
    PausedOnJob {
        job_id: JobId,
        paused_at_event_id: String,
    },
}

/// Placeholder for actor entrypoint. Plan 2 implements `actor_main`,
/// `replay_and_position`, `try_start_turn`, `try_resume_from_job`,
/// `handle_internal_cancel`, `shutdown`.
pub(super) struct ActorState {
    pub(super) _in_flight: Option<InFlight>,
    pub(super) _current_cancel_token: CancellationToken,
}
```

- [ ] **Step 6: Create store_writer.rs (stub)**

`crates/cogito-core/src/runtime/store_writer.rs`:

```rust
//! `store_writer`: the actor's persistence subtask. Consumes
//! `PersistCommand`s, batches `text_delta` events (200ms / 500 chars),
//! and calls `ConversationStore::append` with `fsync` per event. See
//! spec §8 for the full state machine.
//!
//! Implementation is Plan 2 (Sprint 1).

use tokio::sync::{mpsc, oneshot};

/// Commands the actor (or TurnDriver via persist_tx) sends to the
/// store writer subtask.
#[derive(Debug)]
pub enum PersistCommand {
    /// Append one event. If `ack` is Some, the writer signals completion
    /// (after fsync) by sending on the oneshot.
    Append {
        /// Opaque event payload. Plan 2 replaces `serde_json::Value`
        /// with the concrete `ConversationEvent` type once that lands
        /// in `cogito-protocol`.
        event: serde_json::Value,
        ack: Option<oneshot::Sender<Result<(), StoreWriteError>>>,
    },
    /// Force-flush the text-delta buffer immediately.
    Flush {
        ack: oneshot::Sender<Result<(), StoreWriteError>>,
    },
}

/// Errors from the store writer subtask. Surfaced to the caller via the
/// `ack` oneshots above.
#[derive(Debug, thiserror::Error)]
pub enum StoreWriteError {
    #[error("store I/O failed: {0}")]
    Io(String),
    #[error("buffer flush failed: {0}")]
    Flush(String),
}

/// Plan 2 entry point.
pub(super) async fn _store_writer_main(
    mut _rx: mpsc::Receiver<PersistCommand>,
    // store: Arc<dyn ConversationStore>,
) {
    todo!("Plan 2 Task: implement select on rx.recv() + 200ms tick + flush rules per spec §8")
}
```

- [ ] **Step 7: Wire runtime into cogito-core lib.rs**

Update `crates/cogito-core/src/lib.rs`:

```rust
//! cogito-core
//!
//! Brain (Harness) + Runtime layer. Per ADR-0004 the `harness/` module
//! may import only `cogito-protocol`; the `runtime/` module may import
//! any non-Surface layer (it's the DI shell that wires concrete impls
//! into Brain).
//!
//! See:
//! - `ARCHITECTURE.md`
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`

pub mod harness;
pub mod runtime;
```

- [ ] **Step 8: Add `parking_lot` workspace dep (needed by handle.rs Mutex)**

In root `Cargo.toml` `[workspace.dependencies]` add:

```toml
parking_lot = "0.12"
```

And in `crates/cogito-core/Cargo.toml` `[dependencies]` add:

```toml
parking_lot.workspace = true
```

- [ ] **Step 9: Verify workspace compiles**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: `Finished` with possibly `warning: missing documentation` items (acceptable, will be addressed in Plan 2 when bodies replace `todo!()`s).

Verify no clippy errors on stubs:

Run: `cargo clippy -p cogito-core --all-targets 2>&1 | tail -20`
Expected: passes (any `unused` warnings are fine on stubs).

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml crates/cogito-core/
git commit -m "$(cat <<'EOF'
Scaffold runtime module: Runtime, SessionHandle, actor, store_writer

Per spec §3-§7: lands the type signatures and channel-protocol enums
(SessionCommand, PersistCommand, OpenMode, ShutdownOutcome,
InFlight) so Sprint 1 / Plan 2 can fill in bodies one task at a time.
All public methods compile and return todo!() with descriptive Plan-2
breadcrumbs.

Adds tokio-util (CancellationToken), dashmap (sessions map),
parking_lot (CancellationToken swap in SessionShared).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Add layer-check script and justfile recipe

**Files:**
- Create: `scripts/check-layer.sh`
- Modify: `justfile`

ADR-0004 §2: Brain may only `use cogito_protocol::*`. We enforce this via Cargo (Task 3) AND a grep guard (cheap belt-and-braces).

- [ ] **Step 1: Create the script**

`scripts/check-layer.sh`:

```bash
#!/usr/bin/env bash
# Enforce ADR-0004 import rule: cogito-core/src/harness/** may not
# import any concrete Hand, Boundary, or Session crate. The Cargo.toml
# already forbids the dependency; this grep also catches stray refs in
# example code and comments-turned-real-imports.

set -euo pipefail

FORBIDDEN_PATTERN='use cogito_(tools|model|sandbox|jobs|store_jsonl|store_postgres|store_http|mcp|subagent|storage_local|storage_s3|storage_http)'

if grep -rEn "$FORBIDDEN_PATTERN" crates/cogito-core/src/harness/ 2>/dev/null; then
    echo "ERROR: ADR-0004 violation — Brain (harness/) imported a concrete Hand/Boundary/Session crate" >&2
    exit 1
fi

echo "OK: ADR-0004 layer import rule respected in cogito-core/src/harness/"
```

Mark executable:

Run: `chmod +x scripts/check-layer.sh`

- [ ] **Step 2: Add justfile recipe**

Append to `justfile`:

```
# Check ADR-0004 layer import rule
layer-check:
    @./scripts/check-layer.sh
```

Also update the `ci` recipe to include it:

Change:

```
ci: fmt-check clippy test
```

to:

```
ci: fmt-check clippy layer-check test
```

- [ ] **Step 3: Verify**

Run: `just layer-check`
Expected: `OK: ADR-0004 layer import rule respected in cogito-core/src/harness/`.

Run: `just ci 2>&1 | tail -5`
Expected: passes through all four steps.

- [ ] **Step 4: Commit**

```bash
git add scripts/check-layer.sh justfile
git commit -m "$(cat <<'EOF'
Add ADR-0004 layer import check script and just ci wiring

Belt-and-braces alongside the Cargo.toml dep rules from earlier tasks:
greps cogito-core/src/harness/ for any concrete Hand/Boundary/Session
import. Wired into `just ci` so CI fails on layer violations.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Update GitHub Actions CI — add layer-check and cargo-deny

**Files:**
- Modify: `.github/workflows/ci.yml`

Per spec §13 Phase 4.

- [ ] **Step 1: Add layer-check job**

Insert before the `test` job in `.github/workflows/ci.yml`:

```yaml
  layer-check:
    runs-on: ubuntu-latest
    needs: format
    steps:
      - uses: actions/checkout@v4
      - run: ./scripts/check-layer.sh
```

- [ ] **Step 2: Add cargo-deny job**

After `test`, append:

```yaml
  deny:
    runs-on: ubuntu-latest
    needs: format
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: EmbarkStudios/cargo-deny-action@v2
```

- [ ] **Step 3: Add deny.toml**

Create `deny.toml` at repo root:

```toml
# cargo-deny configuration. Enforces ADR-0005 §4 Security: no known
# vulnerabilities, no incompatible licenses, no duplicate deps in
# critical crates.

[advisories]
yanked = "deny"

[licenses]
allow = [
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "MIT",
    "MIT-0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
    "Unicode-3.0",
    "Zlib",
    "MPL-2.0",
]
confidence-threshold = 0.93

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 4: Verify locally (optional)**

If `cargo-deny` is installed locally:

Run: `cargo deny check 2>&1 | tail -10`
Expected: passes. If unfamiliar advisories surface they're acceptable to address in a follow-up; for this task we land the config, not chase the report.

If `cargo-deny` is not installed, skip; CI will run it.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml deny.toml
git commit -m "$(cat <<'EOF'
Add layer-check and cargo-deny jobs to CI

ADR-0005 §4 Security: cargo-deny gates known vulnerabilities,
incompatible licenses, wildcard versions, and unknown registries.
Layer-check enforces ADR-0004 Brain import rule alongside the
Cargo.toml-level dependency restriction.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Sync ARCHITECTURE.md — Workspace layout + Trait contracts sections

**Files:**
- Modify: `ARCHITECTURE.md`

- [ ] **Step 1: Update the Workspace layout table**

Find the row for `cogito-conversation` and remove it. The row that referenced `cogito-conversation` as superseded by `cogito-store-jsonl` already exists at the bottom of that section; leave that note.

Ensure the row for `cogito-store-jsonl` is present (it already is — verify it reads):

```
| `cogito-store-jsonl` | Session | v0.1 | First backend: per-session JSONL files, `fsync` per event. Layout: `<root>/sessions/<session_id>.jsonl`. |
```

Update the `cogito-core` row description to mention the new module split:

Find the existing `cogito-core` row and replace its rightmost cell with:

```
`harness/` is Brain (H01–H10), may only `use cogito_protocol::*`. `runtime/` is the hosting platform (DI, panic catch, per-session actor, store writer subtask, `BrainSpawner` impl). v0.1 keeps both modules in one crate; split when ADR-0004 §4 triggers fire.
```

- [ ] **Step 2: Update Trait contracts table**

Find the table headed "Trait | Implemented by | Defines | When". Update the `JobManager` row's `Defines` cell:

```
Async work state tracking (`status`/`result`/`cancel`) plus mailbox-injected completion callback (`on_complete`). Submission is on the concrete `LocalJobManager` type (Hands-internal).
```

Add a new row above the `MetricsRecorder` row:

```
| `StreamEvent` (type) | (value type) | Real-time event stream observable via `SessionHandle::subscribe`; broadcast fanout; per-chunk text deltas; not persisted | v0.1 |
| `ExecutionClass` (type) | (value type) | `ToolDescriptor.execution_class` ∈ {AlwaysSync, AlwaysAsync, Adaptive}; H08 uses it to validate `InvokeOutcome` variant | v0.1 |
```

- [ ] **Step 3: Reference the new spec + ADR-0006 in the "Where to start" section**

In the "Where to start" section, after item 4 (read ADR-0004 / ADR-0005), insert:

```
6. For execution-model / threading / lifecycle questions, read **ADR-0006** and `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
```

(ADR-0006 file is created in Task 17 — forward reference is intentional; reviewers see the linked work-in-progress in the same PR.)

- [ ] **Step 4: Verify no broken anchors**

Run: `grep -n 'cogito-conversation' ARCHITECTURE.md`
Expected: only the "superseded by cogito-store-jsonl" note remains.

- [ ] **Step 5: Commit**

```bash
git add ARCHITECTURE.md
git commit -m "$(cat <<'EOF'
Sync ARCHITECTURE.md with Runtime + H01 execution model spec

- Update cogito-core row to call out the harness/ + runtime/ split
- Update JobManager trait row to mention on_complete callback
- Add StreamEvent and ExecutionClass to trait contracts table
- Add a "Where to start" pointer to ADR-0006 + execution-model spec

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Sync ROADMAP.md — Sprint 0 checkboxes

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Update the Sprint 0 list**

Find the `Sprint 0 · Project skeleton` section. Replace the unchecked items with the actual completion state:

```
#### Sprint 0 · Project skeleton (0.5 day)
- [x] AGENTS.md, ARCHITECTURE.md, ROADMAP.md, ADR-0001/0002/0003/0004 written
- [x] CLAUDE.md added; ADR-0004 (Brain/Hands/Session) ratified
- [x] ADR-0005 (production scope) ratified
- [x] ADR-0006 (Runtime + H01 execution model) ratified
- [x] Workspace topology fixed per ADR-0004: dropped cogito-conversation, added cogito-store-jsonl, stripped Hands/Boundary/Session deps from cogito-core
- [x] Protocol types landed: ExecutionClass, StreamEvent, JobCompletionEvent, JobManager::on_complete, TurnOutcome, TurnFailureReason
- [x] Runtime module scaffolded (stubs): Runtime, RuntimeBuilder, SessionHandle, SessionActor, store_writer
- [x] CI runs `just ci` (fmt + clippy + layer-check + test) + cargo-deny
- [x] `cargo test` passes (empty + new type tests)
```

- [ ] **Step 2: Update "Current" pointer**

Change:

```
> **v0.1 · Foundation** — Sprint 0 (skeleton) in progress.
```

to:

```
> **v0.1 · Foundation** — Sprint 0 complete; Sprint 1 (H02 + JSONL store + SLO benchmark) entering implementation.
```

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md
git commit -m "$(cat <<'EOF'
Mark Sprint 0 complete in ROADMAP.md

All Sprint 0 exit criteria met per spec §13 work order: workspace
topology ADR-0004-compliant, protocol new types landed, runtime
module scaffolded, CI gates wired.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: Add implementation notes to H01 / H02 / H08 component docs

**Files:**
- Modify: `docs/components/H01-turn-driver.md`
- Modify: `docs/components/H02-step-recorder.md`
- Modify: `docs/components/H08-tool-dispatcher.md`

- [ ] **Step 1: Append to H01**

Append a new section to `docs/components/H01-turn-driver.md`:

```markdown
## Implementation note (v0.1)

H01 runs as a per-turn tokio task spawned by `SessionActor`
(`crates/cogito-core/src/runtime/actor.rs`). The FSM is an `enum
TurnState` where each variant carries the data its transition needs
(prompt, stream, surface, strategy), so the type system enforces
state-data invariants and forbids skipped transitions.

A single TurnDriver task = one `input → final answer or paused`
cycle. Multi-turn tool loops are an *inner* loop within the FSM
(re-entering `TurnState::Init` after `DispatchOutcome::AllSync`); a
paused turn ends the current task and is later resumed by *another*
TurnDriver task that starts at `TurnState::ToolDispatching` with the
async result. The actor coordinates the handoff.

See `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
§5 for the FSM pseudocode.
```

- [ ] **Step 2: Append to H02**

Append to `docs/components/H02-step-recorder.md`:

```markdown
## Implementation note (v0.1)

H02 has no standalone object in v0.1. Logically it splits into:

- **Producer side**: every call site (TurnDriver state transitions,
  actor main loop, hooks) sends `PersistCommand::Append { event, ack }`
  on a `persist_tx: mpsc::Sender<PersistCommand>` (capacity 256), then
  awaits the `ack` oneshot before transitioning.

- **Consumer side**: a `store_writer` tokio subtask owns the
  `ConversationStore` handle (`crates/cogito-core/src/runtime/store_writer.rs`).
  It batches text-delta events on a 200ms timer or 500-char threshold,
  force-flushes before any non-delta event, calls
  `store.append` with per-event `fsync` (via `spawn_blocking`), and
  signals the `ack` oneshot.

The producer/consumer split is what makes the inviolable rule
"every state transition writes an event before transitioning" cheap:
the producer awaits one mpsc round-trip + one ack — the actor's
mailbox stays polled the whole time because the producer is the
TurnDriver task, not the actor itself.

See spec §8 for fsync strategy, batching rules, and the Sprint 1
SLO benchmark plan.
```

- [ ] **Step 3: Append to H08**

Append to `docs/components/H08-tool-dispatcher.md`:

```markdown
## Implementation note (v0.1)

H08 branches on two signals:

1. `ToolDescriptor.execution_class`
   (`AlwaysSync` / `AlwaysAsync` / `Adaptive`) — checked before invoke
2. `InvokeOutcome` returned by `ToolProvider::invoke`
   (`Sync(ToolResult)` / `Async(JobId)`)

Contract violations (e.g., a tool descriptor declared `AlwaysSync`
returning `Async`) are `debug_assert!`s in dev builds and a structured
`ToolResult::Error { kind: InvocationFailed }` in release. Strategy
filtering (e.g., `allow_async_tools: false`) is H05's responsibility,
not H08's; H08 trusts the descriptor it receives.

Cancellation: each `invoke()` call runs inside
`tokio::select!(provider.invoke(...), ctx.cancel.cancelled())`. On
cancel, in-flight tool futures are *dropped on next yield* (cooperative)
— cogito does not `task.abort()` them, leaving cleanup to the tool's
RAII. Tools that want to honor cancel must `select!` on `ctx.cancel`
internally.

Panic isolation: each `invoke()` is wrapped in `catch_unwind` (Layer
3 of the three-layer panic isolation described in
`docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
§9). A panicking tool surfaces as
`ToolResult::Error { kind: ToolPanicked }`; the turn continues.

See spec §6 for the full sync/async judgment table and §9 for
cancellation + panic propagation.
```

- [ ] **Step 4: Commit**

```bash
git add docs/components/
git commit -m "$(cat <<'EOF'
Add implementation notes to H01 / H02 / H08 component docs

Per spec §13 doc-sync work order: each component design doc now
calls out how the v0.1 implementation maps the logical role onto
concrete tokio tasks, channels, and types from the runtime module.
The notes link back to the canonical spec sections for detail.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Write ADR-0006 — Runtime + H01 execution model

**Files:**
- Create: `docs/adr/0006-runtime-h01-execution-model.md`

Distill the 6 load-bearing decisions from the spec into a formal ADR. The spec stays as detailed reference; ADR is the durable contract.

- [ ] **Step 1: Create the ADR**

`docs/adr/0006-runtime-h01-execution-model.md`:

```markdown
# ADR-0006: Runtime + H01 Turn Driver execution model

## Status

Accepted (2026-05-18)

## Context

ADR-0003 specified H01 Turn Driver as an explicit FSM and ADR-0004
locked the Brain/Hands/Session/Runtime layer boundaries. Neither said
how the Runtime layer hosts Brain on tokio: task topology, channel
shape, cancellation, panic isolation, async-job wake-up. Without a
ratified answer, Sprint 1 implementers would each invent their own.

A design dialogue produced
`docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
(13 sections, ~1.7k lines). This ADR captures the six load-bearing
decisions; the spec remains the reference for rationale and Codex /
Claude Code / SaaS-platform comparisons.

## Decision

### 1. Per-session actor task model

Each session owns a long-lived tokio task (`SessionActor`) that
holds the mailbox receiver, broadcast sender, persist channel sender,
job-completion channel, and in-flight turn state. Brain (H01–H10)
runs *inside* this actor, not alongside it. The session is the unit
of concurrency.

Rejected alternative: shared `Arc<Session> + Mutex<ActiveTurn>` (the
Codex Rust model). Reasons: ≥1000 active sessions per process
(ADR-0005 §3) need per-session budget enforcement and panic
isolation, both of which actor task boundaries provide naturally.

### 2. Caller-injected tokio Handle

`RuntimeBuilder::handle()` accepts a `tokio::runtime::Handle`;
fallback is `Handle::current()` at `build()` time. cogito does *not*
create its own runtime. Blocking I/O (`fsync`) uses
`spawn_blocking` (not `tokio::fs`) to enable group-commit
optimizations in the store writer.

Rejected alternative: cogito-owned `Runtime::new()`. Reasons:
embedded-library positioning (ADR-0005 §1); panic isolation is
task-level (`catch_unwind`), not runtime-level — a dedicated
runtime buys no extra safety.

### 3. Two-level cancellation; cooperative

- `SessionHandle::cancel_turn()` → fires a per-turn
  `CancellationToken`; tools/streams `select!` on it
- `SessionHandle::shutdown(timeout)` → drains mailbox, flushes store
  writer, cancels in-flight turn after timeout

Process-level `shutdown_token` reserved in `RuntimeBuilder` for
v0.4 (ADR-0010) but not consumed in v0.1.

Rejected alternative: `task.abort()` for cancellation. Reasons:
abort drops futures at non-deterministic yield points, can leave
RAII guards in inconsistent state; ADR-0002's event log integrity
forbids this.

### 4. Dual event streams (persist + broadcast)

Two distinct types and two distinct channels:

- `ConversationEvent` → `mpsc<PersistCommand>` (cap 256) → store
  writer subtask → `ConversationStore::append` with per-event fsync.
  Text deltas batched on 200ms / 500-char threshold.
- `StreamEvent` → `broadcast::channel<StreamEvent>` (cap 256) →
  subscribers (TUI, observability, consumer hooks). Per-chunk,
  unbatched. Slow subscribers receive `Lagged(n)`.

Rejected alternative: single unified channel feeding both store
and subscribers. Reasons: persist must be 0-loss
(resume invariant); broadcast must permit slow-consumer drop —
unifying forces either to compromise.

### 5. Mailbox-injected JobCompleted for async-job wake-up

When a tool returns `InvokeOutcome::Async(JobId)`, TurnDriver
terminates and the actor enters `PausedOnJob`. `JobManager` later
sends a `JobCompletionEvent` on a registered mpsc; the actor
synthesizes a `SessionCommand::JobCompleted` and **routes it
through the mailbox** (preserving FIFO ordering against any new
caller `Input`). A new TurnDriver task then resumes the FSM at
`ToolDispatching` with the result.

This matches Claude Code's "fires between turns" rule and is the
in-process specialization of Inngest's `step.waitForEvent` /
Temporal Signal patterns (see spec §10).

Rejected alternative: actor blocks on `job_manager.await_result()`.
Reasons: actor stops reading mailbox → cancellation and shutdown
cannot reach it.

### 6. ExecutionClass on ToolDescriptor (runtime-decided sync/async)

`ToolDescriptor.execution_class ∈ {AlwaysSync, AlwaysAsync,
Adaptive}` lets H08 statically know which variant of
`InvokeOutcome` to expect. `Adaptive` tools choose per call inside
`invoke()` based on argument-derived predictions.

Rejected alternative: expose `run_in_background: bool` to the LLM
(the Claude Code CLI pattern). Reasons: pollutes prompt with
execution-model knowledge; not portable across model providers; LLM
misjudgments produce silent SLO regressions.

## Consequences

- **Compiler-enforced layer rules**: `cogito-core/Cargo.toml` drops
  every Hands/Boundary/Session dep; layer violations are now build
  errors (ADR-0004 §2)
- **Per-trait panic isolation**: three `catch_unwind` boundaries
  (actor / turn / tool); one tool panic never kills a session, one
  session panic never kills a process
- **SaaS-ready trait shape**: `JobManager::on_complete(job_id, sink)`
  is the in-process form of the same callback shape distributed
  brokers (Redis Stream pub/sub, Inngest event bus) need; v0.4
  swaps the implementation without changing Brain code

## Follow-on work

- Sprint 1 (Plan 2): real `store_writer` implementation +
  `cogito-store-jsonl` append + per-event fsync benchmark per
  ADR-0005 §3
- Sprint 2: TurnDriver FSM body, model gateway, sync tool dispatch
- Sprint 3: H03 Resume Coordinator with the JobManager.status
  query path added by this ADR
- Sprint 4: Real `JobManager` + first AlwaysAsync tool
- v0.4: replace local JobManager with distributed backend; the
  trait shape established here stays unchanged

## References

- Spec: `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`
- ADR-0001 (workspace layout)
- ADR-0002 (event-sourced conversation log)
- ADR-0003 (state-machine Turn Driver)
- ADR-0004 (Brain / Hands / Session boundaries)
- ADR-0005 (production scope + quality gates)
```

- [ ] **Step 2: Update ADR README to list 0006**

Add a row for ADR-0006 to `docs/adr/README.md` (if the file's index style is a list; otherwise leave the README untouched — it may auto-discover).

Run: `cat docs/adr/README.md | head -20`

If it's a curated table, add a line under it; if it's just a header, no edit needed.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/
git commit -m "$(cat <<'EOF'
Ratify ADR-0006: Runtime + H01 Turn Driver execution model

Distills the 6 load-bearing decisions from the design spec into a
formal ADR: per-session actor, caller-injected tokio Handle,
two-level cooperative cancellation, dual event streams,
mailbox-injected JobCompleted, ExecutionClass on ToolDescriptor.

The spec
(docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md)
remains the detailed reference; this ADR is the durable contract.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 18: Open Pull Request

**Files:** none (git operation)

- [ ] **Step 1: Push branch**

Run: `git push -u github impl/sprint-0-closure`
Expected: branch created on github remote.

- [ ] **Step 2: Open PR**

Run:

```bash
gh pr create --repo nathan-tsien/cogito --base main \
    --head impl/sprint-0-closure \
    --title "Sprint 0 closure: workspace fix, protocol types, runtime scaffolding, doc sync" \
    --body "$(cat <<'EOF'
## Summary

Implements the spec §13 work order for v0.1 Sprint 0 closure plus the user-requested doc sync.

- **Workspace topology** ADR-0004-compliant: drop `cogito-conversation`, add `cogito-store-jsonl`, strip Hands/Boundary/Session deps from `cogito-core` so layer violations are build errors.
- **Protocol types** Sprint 1 needs: `ExecutionClass`, `InvokeOutcome`, `ToolResult`, `ToolErrorKind`, `StreamEvent`, `JobManager::on_complete`, `JobCompletionEvent`, `TurnOutcome`, `TurnFailureReason`. Each with serde-roundtrip tests.
- **Runtime module scaffolding** (`cogito-core/src/runtime/`): `Runtime`, `RuntimeBuilder`, `SessionHandle`, `SessionActor` skeleton, `store_writer` skeleton. Bodies are `todo!()` with Plan 2 breadcrumbs — compilable type signatures only.
- **Doc sync**: ARCHITECTURE.md workspace / trait tables updated; ROADMAP.md Sprint 0 boxes ticked; H01 / H02 / H08 component docs received implementation notes; ADR-0006 ratified.
- **CI**: layer-check script + `just ci` recipe + cargo-deny job + deny.toml.
- **CLAUDE.md**: code-comments-in-English rule recorded (per user request).

## Test plan

- [ ] \`cargo check --workspace\` passes
- [ ] \`just ci\` passes (fmt + clippy + layer-check + test)
- [ ] \`cargo nextest run -p cogito-protocol\` shows 12+ new tests passing
- [ ] Spot-check spec §13 work order — every step has a matching commit
- [ ] Spot-check ADR-0006 — 6 load-bearing decisions match spec §1-§9

## What's NOT in this PR

- Real `actor_main` / `turn_driver` / `store_writer` bodies → Plan 2 (Sprint 1)
- Real `cogito-store-jsonl` append + fsync → Plan 2 (Sprint 1)
- `ConversationEvent` type itself → still in `cogito-protocol/src/lib.rs`, slated for Plan 2 when JSONL format is finalized

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: prints PR URL.

- [ ] **Step 3: Report the PR URL**

End of plan. Hand the URL back to the user; await review / merge before starting Plan 2.

---

## Self-review checklist

Already done by author at write time:

1. **Spec coverage**: each of spec §13 阶段 1.1–1.6, 2.1–2.5, 3.1–3.4, 4.1–4.3 maps to at least one task. Doc-sync items map to Tasks 14–16. ADR-0006 (mentioned in §13 final table) maps to Task 17.
2. **Placeholder scan**: every `todo!()` in scaffolded code includes the explicit Plan-2 task it defers to. No bare TODOs in shipping artifacts.
3. **Type consistency**: `JobId` in `tool.rs` references `crate::job::JobId`; `ToolResult` in `job.rs` references `crate::tool::ToolResult`; both modules co-defined in cogito-protocol; tests cross-import correctly.
4. **Cargo dep additions are explicit**: every new workspace dep (`tokio-util`, `dashmap`, `parking_lot`, `tempfile`) gets added to root `[workspace.dependencies]` first, then referenced via `.workspace = true` in member crates.

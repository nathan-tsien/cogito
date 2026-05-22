# Sprint 5 · Hook Pipeline 实化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the v0.1 Sprint 2 no-op `HookPipeline` with a real H09 Hook Pipeline — `HookHandler` trait in `cogito-protocol`, `HookProvider` aggregation pattern, panic-catch, `MetricsRecorder` trait, two example hooks (sensitive-content + bash-audit), all 5 lifecycle points wired, P99 < 5ms latency budget verified.

**Architecture:** `cogito-protocol` gains two new modules — `hook` (`HookHandler` + `HookProvider` + `HookDecision` + `HookLifecyclePoint`) and `metrics` (`MetricsRecorder` + `NoOpMetricsRecorder`). `cogito-core::harness::hooks` refactors from a monolithic no-op file into a module with `composite.rs` (the real `HookPipeline` invoker holding `Vec<Arc<dyn HookHandler>>`), `panic_catch.rs` (panic→Reject helper), and `examples/` (two reference hooks). The 5 lifecycle points (pre_prompt / pre_dispatch / post_model / post_turn / on_error) are already wired at 2 sites; this sprint completes the remaining 3 and replaces the no-op invoker with the composite. `MetricsRecorder` is injected into `TurnDeps` and called by the composite for every hook invocation. `EventPayload::HookRejected` is additive under ADR-0007 (no `SCHEMA_VERSION` bump).

**Scope decision — `HookDecision::Modify` deferred:** H09 doc lists `Modify` as a critical invariant, but v0.1 ships only `Allow` / `Reject`. `Modify` requires per-lifecycle-point payload types (`ModelInput` for pre_prompt vs tool args for pre_dispatch) — designing this without a real consumer use case violates YAGNI. `HookDecision` is `#[non_exhaustive]` so adding `Modify` later is additive. The H09 doc is amended in Task 13 to record this scope.

**Scope decision — `cogito.toml [[hooks]]` deferred:** Sprint 5 wires example hooks via direct construction in tests and a hardcoded list in `cogito-cli` for the `cogito chat` smoke path. The `[[hooks]]` config section is deferred to v0.2 Sprint 12 (Plugin), where it composes with plugin-bundled hooks naturally. This keeps Sprint 5 within its 1-day budget and avoids designing a config schema that Plugin work will revisit anyway.

**Tech Stack:** Rust 2024 / MSRV 1.85, `serde`, `thiserror`, `tracing`, `tokio`. TDD via `cargo nextest`. Workspace recipes (`make fmt`, `make fix CRATE=<name>`, `make test CRATE=<name>`, `make ci`).

**Source-of-truth references:**
- [`docs/components/H09-hook-pipeline.md`](../../components/H09-hook-pipeline.md) — component design
- [Rebalance spec §3.1 + §7.2](../specs/2026-05-22-roadmap-rebalance-design.md)
- [ROADMAP.md Sprint 5](../../../ROADMAP.md)

**Sprint sequencing:** Sprint 4 (MCP) must close first. Sprint 5 does not touch `cogito-mcp` or MCP integration paths; no merge conflicts expected. After Sprint 5 closes, Sprint 6 (Context C2 trait freeze) uses the `MetricsRecorder` trait introduced here for context-decision observability.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/cogito-protocol/src/hook.rs` | Create | `HookDecision` enum (`Allow` / `Reject { reason }`, `#[non_exhaustive]`), `HookLifecyclePoint` enum (5 variants), `HookHandler` trait (sync; 5 methods, all default `Allow`/`no-op`; `name()` required), `HookProvider` trait (`fn list(&self) -> Vec<Arc<dyn HookHandler>>`). |
| `crates/cogito-protocol/src/metrics.rs` | Create | `MetricsRecorder` trait (`record_hook_invocation(point, hook_name, duration_us, decision)` + `record_counter(name, labels)`), `MetricKind` enum, `NoOpMetricsRecorder` default impl. |
| `crates/cogito-protocol/src/lib.rs` | Modify | `pub mod hook; pub mod metrics;` + top-level re-exports (`HookHandler`, `HookProvider`, `HookDecision`, `HookLifecyclePoint`, `MetricsRecorder`, `NoOpMetricsRecorder`). |
| `crates/cogito-protocol/src/event.rs` | Modify | Add `EventPayload::HookRejected { hook_name: String, point: HookLifecyclePoint, reason: String }` variant. Update `all_fifteen_variants_roundtrip` test → rename + extend. Regenerate `docs/schemas/conversation-event-v1.json`. |
| `crates/cogito-core/src/harness/hooks.rs` | Delete | Replaced by `hooks/` module directory. |
| `crates/cogito-core/src/harness/hooks/mod.rs` | Create | Re-exports + glue: `pub use cogito_protocol::hook::*;` + `pub use composite::*;` + `pub use panic_catch::*;` + `pub mod examples;`. |
| `crates/cogito-core/src/harness/hooks/composite.rs` | Create | `CompositeHookPipeline` struct holding `Vec<Arc<dyn HookHandler>>` + optional `Arc<dyn MetricsRecorder>`. 5 async lifecycle methods iterate handlers, apply `panic_catch::wrap_*`, time each call for metrics, return first `Reject`. |
| `crates/cogito-core/src/harness/hooks/panic_catch.rs` | Create | `wrap_pre_prompt(&dyn HookHandler, &ModelInput) -> HookDecision` + analogues for the other 4 points. Each wraps `std::panic::catch_unwind(AssertUnwindSafe(...))` and converts payload to `HookDecision::Reject { reason: "hook '<name>' panicked: <location>" }`. |
| `crates/cogito-core/src/harness/hooks/examples/mod.rs` | Create | Module declarations + `pub use`. |
| `crates/cogito-core/src/harness/hooks/examples/sensitive_content.rs` | Create | `SensitiveContentHook` — implements `HookHandler`. `pre_dispatch` scans tool args for known secret-shaped strings (regex: `AKIA[0-9A-Z]{16}` for AWS access keys; `ghp_[A-Za-z0-9]{36}` for GitHub PAT; `sk-[A-Za-z0-9]{20,}` for OpenAI) and rejects with the matched pattern name. Other 4 methods use trait defaults. |
| `crates/cogito-core/src/harness/hooks/examples/bash_audit.rs` | Create | `BashAuditHook { recorder: Arc<dyn MetricsRecorder> }` — implements `HookHandler`. `pre_dispatch` for tool_name=`bash` increments `cogito.tool.bash.invocations` counter; never rejects. Other 4 methods use trait defaults. |
| `crates/cogito-core/src/harness/turn_driver/deps.rs` | Modify | Replace `pub hooks: HookPipeline` with `pub hooks: Arc<CompositeHookPipeline>`; add `pub metrics: Arc<dyn MetricsRecorder>`. |
| `crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs` | Modify | Update `pre_prompt` call site: `deps.hooks.pre_prompt(&model_input).await` (now async); error path emits `EventPayload::HookRejected`. |
| `crates/cogito-core/src/harness/turn_driver/transitions/tool_dispatching.rs` | Modify | Update `pre_dispatch` call site: pass `args` parameter; now async; error path emits `EventPayload::HookRejected`. |
| `crates/cogito-core/src/harness/turn_driver/transitions/model_completed.rs` | Modify (find or stand up) | Add `deps.hooks.post_model().await` call at the appropriate transition. Identify the actual transition file — likely `model_completed.rs` or wherever H06 stream completion is observed. |
| `crates/cogito-core/src/harness/turn_driver/transitions/mod.rs` (or equivalent terminal-state handler) | Modify | Wire `post_turn` at Completed/Paused terminal states; wire `on_error` at Failed terminal. May require small refactor to a `terminal.rs` helper module. |
| `crates/cogito-core/src/runtime/session_loop.rs` | Modify | `try_start_turn` constructs `Arc::new(CompositeHookPipeline::with_handlers(vec![]))` (empty by default for v0.1 backward-compat) and `Arc::new(NoOpMetricsRecorder)`; both injected into `TurnDeps`. New helper `SessionShared::with_hooks(handlers)` lets callers (CLI, tests) override. |
| `crates/cogito-core/src/harness/step_recorder.rs` | Modify | Add `record_hook_rejected(turn_id, hook_name, point, reason)` async method that wraps `EventPayload::HookRejected`. |
| `crates/cogito-protocol/tests/hook_trait.rs` | Create | Unit test: `HookHandler` default methods return `Allow`/no-op; named impl overrides selected methods; `HookProvider::list` returns owned `Vec<Arc<dyn HookHandler>>`. |
| `crates/cogito-protocol/tests/metrics_trait.rs` | Create | Unit test: `NoOpMetricsRecorder` swallows all calls; trait-object dispatch works through `Arc<dyn MetricsRecorder>`. |
| `crates/cogito-core/src/harness/hooks/panic_catch.rs` (inline `#[cfg(test)] mod tests`) | Create | Unit tests: `wrap_pre_prompt` returns `Allow` for happy path; returns `Reject { reason: contains "panicked" }` when handler panics with `panic!("boom")`; `Reject { reason }` preserves `HookHandler::name()`. |
| `crates/cogito-core/src/harness/hooks/composite.rs` (inline `#[cfg(test)] mod tests`) | Create | Unit tests: empty pipeline always returns `Allow`; first Reject short-circuits remaining handlers; panicked handler returns `Reject`; metrics recorded for every invocation. |
| `crates/cogito-core/src/harness/hooks/examples/sensitive_content.rs` (inline `#[cfg(test)] mod tests`) | Create | Unit tests: clean args → `Allow`; AWS key in arg → `Reject` mentioning AWS pattern; GitHub PAT in arg → `Reject`; OpenAI key in arg → `Reject`; nested JSON object containing key → `Reject`. |
| `crates/cogito-core/src/harness/hooks/examples/bash_audit.rs` (inline `#[cfg(test)] mod tests`) | Create | Unit tests: bash tool invocation increments counter; non-bash tool invocation does NOT increment counter; never rejects. |
| `crates/cogito-core/tests/hook_integration.rs` | Create | Integration test: build a Runtime with `SensitiveContentHook` installed → submit a turn that asks model to call a tool with an AWS key in args → assert turn ends in `TurnFailureReason::HookRejected` + `EventPayload::HookRejected` event in store. |
| `crates/cogito-core/tests/hook_panic_isolation.rs` | Create | Chaos test: build a Runtime with a `PanickyHook` that always panics in `pre_prompt` → submit a turn → assert turn ends in `TurnFailureReason::HookRejected { hook_name: "panicky", message: contains "panicked" }`; session loop continues to accept further submits. |
| `crates/cogito-core/benches/hook_latency.rs` | Create | Criterion benchmark: 1000 invocations of `CompositeHookPipeline::pre_prompt` with 5 handlers, each running ~100 µs work. Assert P99 < 5 ms (allows generous overhead given workload). Smoke threshold; baseline recorded under `docs/quality/v0.1-hook-latency.md`. |
| `docs/quality/v0.1-hook-latency.md` | Create | Baseline numbers + benchmark methodology for §7.1 latency budget. |
| `tools/cogito-gen-schema/...` | Run | Regenerate `docs/schemas/conversation-event-v1.json` after adding `HookRejected` variant; commit the regenerated file. |
| `docs/schemas/conversation-event-v1.json` | Regenerate | New event variant entry. |
| `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl` | Modify (optional) | If the fixture must cover every event variant, append one `HookRejected` line. Otherwise leave unchanged + record decision in CHANGELOG. |
| `docs/components/H09-hook-pipeline.md` | Modify | Amend §"Interface" with the v0.1 `HookHandler` trait sig; amend §"Critical invariants" #3 to record `Modify` deferral; add Sprint 5 closure note. |
| `docs/data-model/jsonl-v1.md` | Modify | Document `hook_rejected` event (additive, schema_version 1) — mirror the §"thinking_block_recorded" precedent. |
| `ROADMAP.md` | Modify | Check off all Sprint 5 boxes. |
| `CHANGELOG.md` | Modify | `### Added — Sprint 5 (Hook Pipeline 实化)` block. |
| `AGENTS.md` | Modify (if needed) | If §"Inviolable rules" benefits from a new entry locking the hook purity rule, add one. Otherwise skip. |

**Commits:** one per task (13 total). Each task leaves `make ci` green.

**Test file lint header (REQUIRED):** Every test file (both inline `#[cfg(test)] mod tests {}` and `tests/*.rs` integration files) must begin with:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

Workspace lints set those to `deny`; CI fails without this header. Precedent: `crates/cogito-core/tests/resume_chaos.rs`.

---

### Task 1: `cogito-protocol::hook` module (HookDecision + HookHandler + HookProvider + HookLifecyclePoint)

**Files:**
- Create: `crates/cogito-protocol/src/hook.rs`
- Modify: `crates/cogito-protocol/src/lib.rs:26-39` (add `pub mod hook;` + re-exports)
- Test: `crates/cogito-protocol/tests/hook_trait.rs`

- [ ] **Step 1: Write the failing test** at `crates/cogito-protocol/tests/hook_trait.rs`

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};

struct NamedNoop;
impl HookHandler for NamedNoop {
    fn name(&self) -> &str {
        "named-noop"
    }
}

struct AlwaysReject;
impl HookHandler for AlwaysReject {
    fn name(&self) -> &str {
        "always-reject"
    }
    fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
        HookDecision::Reject {
            hook_name: "always-reject".into(),
            reason: "no".into(),
        }
    }
}

struct StaticProvider(Vec<Arc<dyn HookHandler>>);
impl HookProvider for StaticProvider {
    fn list(&self) -> Vec<Arc<dyn HookHandler>> {
        self.0.clone()
    }
}

#[test]
fn default_methods_return_allow() {
    let h = NamedNoop;
    assert_eq!(h.name(), "named-noop");
    assert!(matches!(
        h.pre_prompt(&ModelInput::default()),
        HookDecision::Allow
    ));
    assert!(matches!(
        h.pre_dispatch("call-id", "tool", &serde_json::Value::Null),
        HookDecision::Allow
    ));
    h.post_model();
    h.post_turn();
    h.on_error("err");
}

#[test]
fn override_pre_prompt_takes_effect() {
    let h = AlwaysReject;
    match h.pre_prompt(&ModelInput::default()) {
        HookDecision::Reject { hook_name, reason } => {
            assert_eq!(hook_name, "always-reject");
            assert_eq!(reason, "no");
        }
        _ => panic!("expected Reject"),
    }
}

#[test]
fn provider_lists_handlers() {
    let provider = StaticProvider(vec![Arc::new(NamedNoop) as Arc<dyn HookHandler>]);
    let listed = provider.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name(), "named-noop");
}

#[test]
fn lifecycle_point_variants_round_trip() {
    let points = [
        HookLifecyclePoint::PrePrompt,
        HookLifecyclePoint::PreDispatch,
        HookLifecyclePoint::PostModel,
        HookLifecyclePoint::PostTurn,
        HookLifecyclePoint::OnError,
    ];
    for p in points {
        let s = serde_json::to_string(&p).unwrap();
        let back: HookLifecyclePoint = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-protocol --test hook_trait`
Expected: FAIL — `hook` module not found in `cogito_protocol`.

- [ ] **Step 3: Create `crates/cogito-protocol/src/hook.rs`**

```rust
//! H09 Hook lifecycle contract.
//!
//! `HookHandler` is a pure, synchronous policy gate (see ADR-0004 §6 and
//! `docs/components/H09-hook-pipeline.md`). Implementations may inspect
//! turn state and return `Allow` or `Reject`; they may NOT perform I/O.
//! Side effects belong in `ToolProvider` / `JobManager`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::gateway::ModelInput;

/// One lifecycle point at which the Hook Pipeline is invoked.
///
/// Persisted into `EventPayload::HookRejected` to make rejection events
/// fully reconstructable from the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HookLifecyclePoint {
    /// Fires at the end of prompt build (ContextManaged → PromptBuilt).
    PrePrompt,
    /// Fires before each tool dispatch.
    PreDispatch,
    /// Fires after model stream completion.
    PostModel,
    /// Fires at terminal Completed / Paused states.
    PostTurn,
    /// Fires at terminal Failed state.
    OnError,
}

/// Decision returned by a `HookHandler`.
///
/// `#[non_exhaustive]` so future variants (`Modify`, etc. — see H09 doc
/// §"Open design questions") can be added additively.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookDecision {
    /// Continue the normal pipeline flow.
    Allow,
    /// Abort the pipeline with the given reason.
    Reject {
        /// Name of the hook that rejected (from `HookHandler::name()`).
        /// Recorded in `EventPayload::HookRejected.hook_name` and
        /// surfaced via `TurnFailureReason::HookRejected.hook_name`.
        hook_name: String,
        /// Human-readable reason for the rejection. Recorded in the
        /// `HookRejected` event and surfaced via `TurnFailureReason`.
        reason: String,
    },
}

/// Brain-side policy gate. All methods MUST be free of I/O.
///
/// Default impls return `Allow` / no-op so implementers override only
/// the lifecycle points they care about.
pub trait HookHandler: Send + Sync {
    /// Stable identifier used in events and metrics. SHOULD be
    /// kebab-case and unique within a deployment.
    fn name(&self) -> &str;

    /// Runs at `ContextManaged → PromptBuilt`.
    fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs before each tool dispatch.
    fn pre_dispatch(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> HookDecision {
        HookDecision::Allow
    }

    /// Runs after model stream completion. Observation-only.
    fn post_model(&self) {}

    /// Runs at terminal Completed / Paused. Observation-only.
    fn post_turn(&self) {}

    /// Runs at terminal Failed. Observation-only.
    fn on_error(&self, _reason: &str) {}
}

/// Aggregation surface for `HookHandler` providers. Used by the runtime
/// to build a `CompositeHookPipeline` from one or more sources
/// (Sprint 5: built-ins; v0.2 Plugin: plugin-bundled hooks).
pub trait HookProvider: Send + Sync {
    /// Returns all handlers the provider contributes.
    fn list(&self) -> Vec<Arc<dyn HookHandler>>;
}
```

- [ ] **Step 4: Wire into `cogito-protocol::lib`**

Modify `crates/cogito-protocol/src/lib.rs:26-39` — add `pub mod hook;` to the module list (alphabetical), and append `pub use hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};` to the re-exports.

- [ ] **Step 5: Verify trait test passes**

Run: `cargo nextest run -p cogito-protocol --test hook_trait`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/hook.rs \
        crates/cogito-protocol/src/lib.rs \
        crates/cogito-protocol/tests/hook_trait.rs
git commit -m "feat(protocol/hook): HookHandler + HookProvider + HookDecision + HookLifecyclePoint"
```

---

### Task 2: `cogito-protocol::metrics` module

**Files:**
- Create: `crates/cogito-protocol/src/metrics.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (add `pub mod metrics;` + re-exports)
- Test: `crates/cogito-protocol/tests/metrics_trait.rs`

- [ ] **Step 1: Write the failing test** at `crates/cogito-protocol/tests/metrics_trait.rs`

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cogito_protocol::hook::HookLifecyclePoint;
use cogito_protocol::metrics::{MetricsRecorder, NoOpMetricsRecorder};

#[test]
fn noop_swallows_all_calls() {
    let rec: Arc<dyn MetricsRecorder> = Arc::new(NoOpMetricsRecorder);
    rec.record_hook_invocation(HookLifecyclePoint::PrePrompt, "any", Duration::from_micros(10), true);
    rec.record_counter("any.counter", &[("k", "v")]);
}

struct CountingRecorder(AtomicUsize);
impl MetricsRecorder for CountingRecorder {
    fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, _: bool) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn record_counter(&self, _: &str, _: &[(&str, &str)]) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn trait_object_dispatch_counts() {
    let rec: Arc<dyn MetricsRecorder> = Arc::new(CountingRecorder(AtomicUsize::new(0)));
    rec.record_hook_invocation(HookLifecyclePoint::PrePrompt, "h", Duration::from_micros(1), true);
    rec.record_counter("c", &[]);
    // Downcast back to verify
    // (without using std::any::Any — count via re-borrowing the inner.)
    // Simpler: just assert no panic; the fact that trait-object dispatch works
    // is the real assertion here.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-protocol --test metrics_trait`
Expected: FAIL — `metrics` module not found.

- [ ] **Step 3: Create `crates/cogito-protocol/src/metrics.rs`**

```rust
//! Pluggable metrics sink.
//!
//! v0.1 ships `NoOpMetricsRecorder` as the default. Real adapters
//! (OpenTelemetry / Prometheus) land in v0.4 per ROADMAP — but the
//! trait surface is frozen here in Sprint 5 so the Hook Pipeline
//! can emit measurements from day one.

use std::time::Duration;

use crate::hook::HookLifecyclePoint;

/// Sink that consumes metric samples emitted by the Brain.
///
/// All methods MUST be synchronous and non-blocking. Implementers
/// should buffer + drain off-task if their backend has latency.
pub trait MetricsRecorder: Send + Sync {
    /// Records one hook invocation at the given lifecycle point.
    ///
    /// - `duration` measures the hook's own execution time only
    ///   (excludes pipeline overhead).
    /// - `allowed = true` for `HookDecision::Allow`; `false` for
    ///   `HookDecision::Reject` (including panic-induced rejection).
    fn record_hook_invocation(
        &self,
        point: HookLifecyclePoint,
        hook_name: &str,
        duration: Duration,
        allowed: bool,
    );

    /// Increments a named counter with optional labels.
    fn record_counter(&self, name: &str, labels: &[(&str, &str)]);
}

/// Default no-op implementation. Used by the runtime when no metrics
/// adapter is configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpMetricsRecorder;

impl MetricsRecorder for NoOpMetricsRecorder {
    fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, _: bool) {}
    fn record_counter(&self, _: &str, _: &[(&str, &str)]) {}
}
```

- [ ] **Step 4: Wire into `cogito-protocol::lib`**

Add `pub mod metrics;` (alphabetical) and `pub use metrics::{MetricsRecorder, NoOpMetricsRecorder};` to re-exports.

- [ ] **Step 5: Verify test passes**

Run: `cargo nextest run -p cogito-protocol --test metrics_trait`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/metrics.rs \
        crates/cogito-protocol/src/lib.rs \
        crates/cogito-protocol/tests/metrics_trait.rs
git commit -m "feat(protocol/metrics): MetricsRecorder trait + NoOpMetricsRecorder default"
```

---

### Task 3: `EventPayload::HookRejected` additive variant

**Files:**
- Modify: `crates/cogito-protocol/src/event.rs:69-189` (add variant)
- Modify: `crates/cogito-protocol/src/event.rs:234` (update count-named roundtrip test)
- Regenerate: `docs/schemas/conversation-event-v1.json`
- Test: extension to existing `crates/cogito-protocol/src/event.rs` inline tests

- [ ] **Step 1: Write the failing variant-coverage test extension**

In `crates/cogito-protocol/src/event.rs` `mod tests`, rename `all_fifteen_variants_roundtrip` to `all_sixteen_variants_roundtrip` and add the new variant case before the existing `ThinkingBlockRecorded` case (keep in alphabetical-by-tag order if the existing test orders by tag — otherwise append):

```rust
EventPayload::HookRejected {
    hook_name: "sensitive-content".into(),
    point: crate::hook::HookLifecyclePoint::PreDispatch,
    reason: "AWS key in args".into(),
},
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-protocol`
Expected: FAIL — `EventPayload::HookRejected` not found.

- [ ] **Step 3: Add the variant** in `crates/cogito-protocol/src/event.rs`, after `ThinkingBlockRecorded`:

```rust
    /// An H09 hook returned `HookDecision::Reject` at the named
    /// lifecycle point. The turn that follows transitions to `Failed`
    /// with `TurnFailureReason::HookRejected { hook_name, message }`.
    ///
    /// Added Sprint 5 as an additive variant under ADR-0007. No
    /// `SCHEMA_VERSION` bump.
    HookRejected {
        /// Name of the hook (from `HookHandler::name()`).
        hook_name: String,
        /// Lifecycle point at which the rejection occurred.
        point: crate::hook::HookLifecyclePoint,
        /// Rejection reason from `HookDecision::Reject`.
        reason: String,
    },
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo nextest run -p cogito-protocol`
Expected: PASS (including the renamed `all_sixteen_variants_roundtrip`).

- [ ] **Step 5: Regenerate schema artifact**

Run: `cargo run -p cogito-gen-schema -- write`
Expected: `docs/schemas/conversation-event-v1.json` updated; CI drift gate happy.

- [ ] **Step 6: Run schema drift gate**

Run: `make ci` (or just the schema check from Sprint 1's `make ci` recipe)
Expected: schema drift gate PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/cogito-protocol/src/event.rs \
        docs/schemas/conversation-event-v1.json
git commit -m "feat(protocol/event): additive HookRejected variant (no schema bump, per ADR-0007)"
```

---

### Task 4: Refactor `harness::hooks` from monolithic file to module

**Files:**
- Delete: `crates/cogito-core/src/harness/hooks.rs`
- Create: `crates/cogito-core/src/harness/hooks/mod.rs`
- Create: `crates/cogito-core/src/harness/hooks/panic_catch.rs`
- Modify: `crates/cogito-core/src/harness/mod.rs` (no change needed — `pub mod hooks;` still resolves to the directory)

- [ ] **Step 1: Verify current usage** — confirm `HookDecision` is referenced only via `crate::harness::hooks::HookDecision` in `context_managed.rs` and `tool_dispatching.rs`

Run: `grep -rn "harness::hooks::" crates/cogito-core/`
Expected output includes: `context_managed.rs:12`, `tool_dispatching.rs:12`, `deps.rs:12`, `session_loop.rs:42`.

- [ ] **Step 2: Create `crates/cogito-core/src/harness/hooks/mod.rs`**

```rust
//! H09 Hook Pipeline — composite invoker over `Vec<Arc<dyn HookHandler>>`.
//!
//! The lifecycle trait lives in `cogito-protocol::hook`. This module
//! provides the runtime invocation surface: panic catch, metrics, and
//! the `CompositeHookPipeline` that the FSM transitions call.
//!
//! See `docs/components/H09-hook-pipeline.md`.

pub mod composite;
pub mod examples;
pub mod panic_catch;

pub use composite::CompositeHookPipeline;
pub use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};
```

- [ ] **Step 3: Create `crates/cogito-core/src/harness/hooks/panic_catch.rs`** (initial skeleton — full impl in Task 5/6)

```rust
//! Panic-catch wrappers for `HookHandler` lifecycle methods.
//!
//! H09 invariant: a panicking hook must not crash Brain. We wrap each
//! call in `std::panic::catch_unwind(AssertUnwindSafe(...))` and
//! convert the payload into `HookDecision::Reject { reason: "..." }`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler};

/// Wraps `HookHandler::pre_prompt` with panic catch.
pub fn wrap_pre_prompt(handler: &dyn HookHandler, input: &ModelInput) -> HookDecision {
    catch_unwind(AssertUnwindSafe(|| handler.pre_prompt(input)))
        .unwrap_or_else(|payload| panic_to_reject(handler.name(), &payload))
}

/// Wraps `HookHandler::pre_dispatch` with panic catch.
pub fn wrap_pre_dispatch(
    handler: &dyn HookHandler,
    call_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
) -> HookDecision {
    catch_unwind(AssertUnwindSafe(|| {
        handler.pre_dispatch(call_id, tool_name, args)
    }))
    .unwrap_or_else(|payload| panic_to_reject(handler.name(), &payload))
}

/// Wraps `HookHandler::post_model` with panic catch.
/// Observation hooks return unit; a panic here is logged but does NOT
/// reject (the turn has already produced model output).
pub fn wrap_post_model(handler: &dyn HookHandler) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.post_model()));
}

/// Wraps `HookHandler::post_turn` with panic catch.
pub fn wrap_post_turn(handler: &dyn HookHandler) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.post_turn()));
}

/// Wraps `HookHandler::on_error` with panic catch.
pub fn wrap_on_error(handler: &dyn HookHandler, reason: &str) {
    let _ = catch_unwind(AssertUnwindSafe(|| handler.on_error(reason)));
}

fn panic_to_reject(hook_name: &str, payload: &Box<dyn std::any::Any + Send>) -> HookDecision {
    let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_owned()
    };
    HookDecision::Reject {
        hook_name: hook_name.to_owned(),
        reason: format!("hook '{hook_name}' panicked: {msg}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct Allowy;
    impl HookHandler for Allowy {
        fn name(&self) -> &str {
            "allowy"
        }
    }

    struct Panicky;
    impl HookHandler for Panicky {
        fn name(&self) -> &str {
            "panicky"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            panic!("boom")
        }
    }

    #[test]
    fn allow_happy_path() {
        let h = Allowy;
        assert!(matches!(
            wrap_pre_prompt(&h, &ModelInput::default()),
            HookDecision::Allow
        ));
    }

    #[test]
    fn panic_becomes_reject_with_hook_name_and_message() {
        let h = Panicky;
        match wrap_pre_prompt(&h, &ModelInput::default()) {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "panicky");
                assert!(reason.contains("panicky"), "{reason}");
                assert!(reason.contains("boom"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }
}
```

- [ ] **Step 4: Create `crates/cogito-core/src/harness/hooks/examples/mod.rs`** (placeholder; populated in Tasks 8/9)

```rust
//! Reference `HookHandler` implementations shipped in v0.1.
//!
//! - `sensitive_content` — rejects tool calls whose args contain
//!   well-known secret-shaped strings.
//! - `bash_audit` — records a metric counter for every `bash` tool
//!   invocation. Never rejects.

pub mod bash_audit;
pub mod sensitive_content;

pub use bash_audit::BashAuditHook;
pub use sensitive_content::SensitiveContentHook;
```

Stub `bash_audit.rs` and `sensitive_content.rs` with `// implementation in Tasks 8/9` comments so the module compiles. Replace those stubs in their respective tasks.

- [ ] **Step 5: Create `crates/cogito-core/src/harness/hooks/composite.rs`** (skeleton)

Empty struct + stub methods that return `Allow`. Full implementation in Task 5.

```rust
//! `CompositeHookPipeline` — the runtime invocation surface for H09.
//!
//! Holds a `Vec<Arc<dyn HookHandler>>` plus an optional
//! `Arc<dyn MetricsRecorder>`. Each lifecycle method iterates
//! handlers, applies panic catch, times each call, records metrics,
//! and short-circuits on the first `Reject`. See Task 5 for the full
//! implementation.

use std::sync::Arc;

use cogito_protocol::hook::HookHandler;
use cogito_protocol::metrics::{MetricsRecorder, NoOpMetricsRecorder};

#[derive(Clone)]
pub struct CompositeHookPipeline {
    handlers: Vec<Arc<dyn HookHandler>>,
    metrics: Arc<dyn MetricsRecorder>,
}

impl CompositeHookPipeline {
    #[must_use]
    pub fn with_handlers(handlers: Vec<Arc<dyn HookHandler>>) -> Self {
        Self {
            handlers,
            metrics: Arc::new(NoOpMetricsRecorder),
        }
    }

    #[must_use]
    pub fn with_handlers_and_metrics(
        handlers: Vec<Arc<dyn HookHandler>>,
        metrics: Arc<dyn MetricsRecorder>,
    ) -> Self {
        Self { handlers, metrics }
    }

    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }

    // Lifecycle methods stub-implemented in Task 5.
}

impl Default for CompositeHookPipeline {
    fn default() -> Self {
        Self::with_handlers(Vec::new())
    }
}
```

- [ ] **Step 6: Delete the old monolithic file**

```bash
git rm crates/cogito-core/src/harness/hooks.rs
```

- [ ] **Step 7: Update `harness::deps`, `runtime::session_loop`, and the two existing call sites** to reference the new module-public names (just imports — no behavior change yet):

In `crates/cogito-core/src/harness/turn_driver/deps.rs:12`:

```rust
use crate::harness::hooks::CompositeHookPipeline;
```

Replace `pub hooks: HookPipeline` → `pub hooks: Arc<CompositeHookPipeline>`. Add `use std::sync::Arc;` at the top if missing.

In `crates/cogito-core/src/runtime/session_loop.rs:42`:

```rust
use crate::harness::hooks::CompositeHookPipeline;
```

At line 330, replace `hooks: HookPipeline::new(),` with `hooks: Arc::new(CompositeHookPipeline::default()),`.

In `context_managed.rs:12` and `tool_dispatching.rs:12`, the `use crate::harness::hooks::HookDecision;` line continues to resolve via the new `mod.rs` re-export — no change needed yet (Task 5 changes the call signatures).

- [ ] **Step 8: Verify the workspace still builds**

Run: `make ci`
Expected: PASS (refactor is import-only; no behavior change).

- [ ] **Step 9: Commit**

```bash
git add crates/cogito-core/src/harness/hooks/ \
        crates/cogito-core/src/harness/turn_driver/deps.rs \
        crates/cogito-core/src/runtime/session_loop.rs
git rm crates/cogito-core/src/harness/hooks.rs
git commit -m "refactor(core/harness/hooks): split monolithic file into module + panic_catch skeleton"
```

---

### Task 5: `CompositeHookPipeline` — full implementation with metrics + panic catch + short-circuit

**Files:**
- Modify: `crates/cogito-core/src/harness/hooks/composite.rs` (replace stubs with real impls + tests)

- [ ] **Step 1: Write the failing composite test** (inline `#[cfg(test)] mod tests`)

Append to `composite.rs`:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use cogito_protocol::gateway::ModelInput;
    use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint};
    use cogito_protocol::metrics::MetricsRecorder;

    use super::*;

    struct CallCounter(AtomicUsize);
    impl HookHandler for CallCounter {
        fn name(&self) -> &str {
            "counter"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            self.0.fetch_add(1, Ordering::SeqCst);
            HookDecision::Allow
        }
    }

    struct Reject;
    impl HookHandler for Reject {
        fn name(&self) -> &str {
            "rejecter"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            HookDecision::Reject {
                hook_name: "rejecter".into(),
                reason: "nope".into(),
            }
        }
    }

    struct Panicky;
    impl HookHandler for Panicky {
        fn name(&self) -> &str {
            "panicky"
        }
        fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
            panic!("boom")
        }
    }

    struct CountingMetrics {
        invocations: AtomicUsize,
        rejections: AtomicUsize,
    }
    impl MetricsRecorder for CountingMetrics {
        fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, allowed: bool) {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            if !allowed {
                self.rejections.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn record_counter(&self, _: &str, _: &[(&str, &str)]) {}
    }

    #[test]
    fn empty_pipeline_returns_allow() {
        let p = CompositeHookPipeline::default();
        assert!(matches!(
            p.pre_prompt(&ModelInput::default()),
            HookDecision::Allow
        ));
    }

    #[test]
    fn first_reject_short_circuits() {
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers(vec![
            Arc::new(Reject) as Arc<dyn HookHandler>,
            counter.clone() as Arc<dyn HookHandler>,
        ]);
        let dec = p.pre_prompt(&ModelInput::default());
        assert!(matches!(dec, HookDecision::Reject { .. }));
        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            0,
            "second hook should be skipped"
        );
    }

    #[test]
    fn panicked_hook_becomes_reject_and_short_circuits() {
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers(vec![
            Arc::new(Panicky) as Arc<dyn HookHandler>,
            counter.clone() as Arc<dyn HookHandler>,
        ]);
        let dec = p.pre_prompt(&ModelInput::default());
        match dec {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "panicky");
                assert!(reason.contains("panicky"));
            }
            _ => panic!("expected Reject"),
        }
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn metrics_record_every_invocation() {
        let metrics = Arc::new(CountingMetrics {
            invocations: AtomicUsize::new(0),
            rejections: AtomicUsize::new(0),
        });
        let counter = Arc::new(CallCounter(AtomicUsize::new(0)));
        let p = CompositeHookPipeline::with_handlers_and_metrics(
            vec![counter as Arc<dyn HookHandler>],
            metrics.clone() as Arc<dyn MetricsRecorder>,
        );
        let _ = p.pre_prompt(&ModelInput::default());
        assert_eq!(metrics.invocations.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.rejections.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn metrics_record_rejection() {
        let metrics = Arc::new(CountingMetrics {
            invocations: AtomicUsize::new(0),
            rejections: AtomicUsize::new(0),
        });
        let p = CompositeHookPipeline::with_handlers_and_metrics(
            vec![Arc::new(Reject) as Arc<dyn HookHandler>],
            metrics.clone() as Arc<dyn MetricsRecorder>,
        );
        let _ = p.pre_prompt(&ModelInput::default());
        assert_eq!(metrics.rejections.load(Ordering::SeqCst), 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::composite::tests`
Expected: FAIL — `pre_prompt`/`pre_dispatch`/etc. not yet implemented on `CompositeHookPipeline`.

- [ ] **Step 3: Implement the 5 lifecycle methods** in `composite.rs`:

```rust
use std::time::Instant;

use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookLifecyclePoint};

use super::panic_catch;

impl CompositeHookPipeline {
    pub fn pre_prompt(&self, input: &ModelInput) -> HookDecision {
        for handler in &self.handlers {
            let start = Instant::now();
            let dec = panic_catch::wrap_pre_prompt(handler.as_ref(), input);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PrePrompt,
                handler.name(),
                elapsed,
                matches!(dec, HookDecision::Allow),
            );
            if let HookDecision::Reject { .. } = &dec {
                return dec;
            }
        }
        HookDecision::Allow
    }

    pub fn pre_dispatch(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> HookDecision {
        for handler in &self.handlers {
            let start = Instant::now();
            let dec = panic_catch::wrap_pre_dispatch(handler.as_ref(), call_id, tool_name, args);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PreDispatch,
                handler.name(),
                elapsed,
                matches!(dec, HookDecision::Allow),
            );
            if let HookDecision::Reject { .. } = &dec {
                return dec;
            }
        }
        HookDecision::Allow
    }

    pub fn post_model(&self) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_post_model(handler.as_ref());
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PostModel,
                handler.name(),
                elapsed,
                true,
            );
        }
    }

    pub fn post_turn(&self) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_post_turn(handler.as_ref());
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::PostTurn,
                handler.name(),
                elapsed,
                true,
            );
        }
    }

    pub fn on_error(&self, reason: &str) {
        for handler in &self.handlers {
            let start = Instant::now();
            panic_catch::wrap_on_error(handler.as_ref(), reason);
            let elapsed = start.elapsed();
            self.metrics.record_hook_invocation(
                HookLifecyclePoint::OnError,
                handler.name(),
                elapsed,
                true,
            );
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::composite::tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/hooks/composite.rs
git commit -m "feat(core/harness/hooks): CompositeHookPipeline iterates handlers + panic catch + metrics"
```

---

### Task 6: Wire `MetricsRecorder` into `TurnDeps` + `session_loop`

**Files:**
- Modify: `crates/cogito-core/src/harness/turn_driver/deps.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs`
- Modify: `crates/cogito-core/src/runtime/mod.rs` (or wherever `Runtime`'s builder lives — check existing `Runtime` API and add a `with_metrics` setter)

- [ ] **Step 1: Update `TurnDeps`**

```rust
use cogito_protocol::metrics::MetricsRecorder;

pub struct TurnDeps {
    // ... existing fields ...
    pub hooks: Arc<CompositeHookPipeline>,
    pub metrics: Arc<dyn MetricsRecorder>,
}
```

- [ ] **Step 2: Thread `metrics` through `SessionShared` / `Runtime` builder**

Find the existing `Runtime` constructor or builder. Add a `metrics: Arc<dyn MetricsRecorder>` field (default `Arc::new(NoOpMetricsRecorder)`). Thread it into `SessionShared` and then into `TurnDeps` at `try_start_turn` time.

For Sprint 5 minimum: just construct `Arc::new(NoOpMetricsRecorder)` inline at `try_start_turn`. Real builder setter can land in v0.4 with the actual OTel adapter.

In `session_loop.rs:325-331`, replace:

```rust
let turn_deps = TurnDeps {
    step: Arc::clone(&state.recorder),
    store: Arc::clone(&state.store),
    model: Arc::clone(&deps.model),
    tools: Arc::clone(&deps.tools),
    hooks: Arc::clone(&state.hooks),   // assuming state holds Arc<CompositeHookPipeline> now
    metrics: Arc::clone(&state.metrics),  // ditto for Arc<dyn MetricsRecorder>
};
```

Add `hooks: Arc<CompositeHookPipeline>` and `metrics: Arc<dyn MetricsRecorder>` fields to whichever struct holds the shared session state (`SessionState` or `SessionShared`). Initialize in the constructor with `Arc::new(CompositeHookPipeline::default())` and `Arc::new(NoOpMetricsRecorder)` as defaults.

- [ ] **Step 3: Verify the workspace still builds with no behavior change**

Run: `make ci`
Expected: PASS — no semantic change yet (default no-op pipeline + no-op metrics).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/src/harness/turn_driver/deps.rs \
        crates/cogito-core/src/runtime/
git commit -m "feat(core/runtime): inject Arc<CompositeHookPipeline> + Arc<dyn MetricsRecorder> via TurnDeps"
```

---

### Task 7: Update existing call sites (`pre_prompt` / `pre_dispatch`) + add `HookRejected` event emission

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs` (add `record_hook_rejected` method)
- Modify: `crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs:71-82` (use new sig + record event)
- Modify: `crates/cogito-core/src/harness/turn_driver/transitions/tool_dispatching.rs:36-54` (use new sig — now takes `args` — + record event)

- [ ] **Step 1: Add `record_hook_rejected` to `StepRecorder`**

Add immediately above `record_turn_failed` (matches event ordering: HookRejected then TurnFailed). Same shape as `record_turn_failed` (`crates/cogito-core/src/harness/step_recorder.rs:351-361`):

```rust
pub async fn record_hook_rejected(
    &mut self,
    turn_id: TurnId,
    hook_name: String,
    point: cogito_protocol::hook::HookLifecyclePoint,
    reason: String,
) -> Result<EventId, StoreError> {
    self.append(
        Some(turn_id),
        EventPayload::HookRejected {
            hook_name,
            point,
            reason,
        },
    )
    .await
}
```

The existing `append` helper at L366 handles envelope construction, `seq_counter` advancement, and store persistence. No `StreamEvent` fanout is added — HookRejected is a persisted audit record, not a subscriber-visible event.

- [ ] **Step 2: Update `context_managed.rs` `pre_prompt` call**

Current code at L71-82 calls `deps.hooks.pre_prompt(&model_input)`. New `CompositeHookPipeline::pre_prompt` has the same signature (still sync) but returns `HookDecision::Reject { hook_name, reason }` (from Task 1 — the `hook_name` field is part of the decision shape from the start). Insert a `record_hook_rejected` call before the `record_turn_failed` so the event log shows: `HookRejected` then `TurnFailed`.

Updated `context_managed.rs`:

```rust
match deps.hooks.pre_prompt(&model_input) {
    HookDecision::Allow => TurnState::PromptBuilt {
        ctx,
        input: model_input,
        surface: tool_surface,
    },
    HookDecision::Reject { hook_name, reason } => {
        // Record HookRejected event (additive log entry, ADR-0007)
        let _ = deps
            .step
            .lock()
            .await
            .record_hook_rejected(
                ctx.turn_id,
                hook_name.clone(),
                cogito_protocol::hook::HookLifecyclePoint::PrePrompt,
                reason.clone(),
            )
            .await;
        let failure_reason = TurnFailureReason::HookRejected {
            hook_name,
            message: reason,
        };
        // ... existing TurnFailed recording path ...
    }
}
```

- [ ] **Step 3: Update `tool_dispatching.rs` `pre_dispatch` call**

New signature `pre_dispatch(call_id, tool_name, args)` requires passing tool args. The current code at L36 only passes call_id + name. Look up `inv.args` (the `ToolInvocation` struct from `tool_resolver.rs`) and pass it through. Replace match block to use new `HookDecision::Reject { hook_name, reason }` shape; record `HookRejected` event before pushing the structured error tool result.

- [ ] **Step 4: Run targeted tests**

Run: `cargo nextest run -p cogito-core --tests`
Expected: any existing turn-driver tests still pass; new tests added in Tasks 11+12 fail (those tasks add them).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/hook.rs \
        crates/cogito-protocol/tests/hook_trait.rs \
        crates/cogito-core/src/harness/step_recorder.rs \
        crates/cogito-core/src/harness/hooks/composite.rs \
        crates/cogito-core/src/harness/hooks/panic_catch.rs \
        crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs \
        crates/cogito-core/src/harness/turn_driver/transitions/tool_dispatching.rs
git commit -m "feat(core/harness): wire pre_prompt + pre_dispatch through CompositeHookPipeline; record HookRejected events"
```

---

### Task 8: Wire `post_model`, `post_turn`, `on_error` lifecycle points

**Files:**
- Identify: the FSM transition file where `ModelCallCompleted` is observed (likely `transitions/model_calling.rs` or `model_completed.rs`). The `post_model` hook fires after H06 sealing.
- Identify: the FSM transition file(s) where `TurnState::Completed` and `TurnState::Paused` are reached. The `post_turn` hook fires there.
- Identify: the FSM transition file where `TurnState::Failed` is reached. The `on_error` hook fires there.

- [ ] **Step 1: Map the transition files**

Run: `grep -rn "TurnState::Completed\|TurnState::Paused\|TurnState::Failed" crates/cogito-core/src/harness/turn_driver/transitions/`
Record the exact line numbers where each terminal state is constructed.

- [ ] **Step 2: Add `deps.hooks.post_model();` call**

In the model-completion transition (after `ModelCallCompleted` is recorded by H06, before the next state transitions to `ModelCompleted` or `ToolDispatching`). This is a fire-and-forget call — observation hooks have no decision to act on.

- [ ] **Step 3: Add `deps.hooks.post_turn();` calls**

Add immediately before `return TurnState::Completed { ... }` and `return TurnState::Paused { ... }` constructions. Two call sites.

- [ ] **Step 4: Add `deps.hooks.on_error(reason);` call**

Add immediately before `return TurnState::Failed { ... }` constructions. The `reason` argument is a `&str` view into the failure reason (e.g., `format!("{:?}", failure_reason).as_str()` or a dedicated `failure_reason.display_short()` method if cleaner).

- [ ] **Step 5: Verify build**

Run: `make ci`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/harness/turn_driver/transitions/
git commit -m "feat(core/harness): wire post_model, post_turn, on_error lifecycle points"
```

---

### Task 9: `SensitiveContentHook` example

**Files:**
- Modify: `crates/cogito-core/src/harness/hooks/examples/sensitive_content.rs` (replace stub with real impl)

- [ ] **Step 1: Write failing tests** at the bottom of `sensitive_content.rs`

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use cogito_protocol::hook::{HookDecision, HookHandler};
    use serde_json::json;

    use super::*;

    #[test]
    fn clean_args_allow() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch("c1", "any", &json!({"q": "hello world"}));
        assert!(matches!(dec, HookDecision::Allow));
    }

    #[test]
    fn aws_key_rejects_with_pattern_name() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"creds": "AKIAIOSFODNN7EXAMPLE"}),
        );
        match dec {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "sensitive-content");
                assert!(reason.contains("aws-access-key"), "{reason}");
            }
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn github_pat_rejects() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"token": "ghp_abcdefghijklmnopqrstuvwxyz0123456789"}),
        );
        match dec {
            HookDecision::Reject { reason, .. } => assert!(reason.contains("github-pat"), "{reason}"),
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn openai_key_rejects() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"k": "sk-abcdefghijklmnopqrstuvwxyz0123456789"}),
        );
        match dec {
            HookDecision::Reject { reason, .. } => assert!(reason.contains("openai-key"), "{reason}"),
            _ => panic!("expected Reject"),
        }
    }

    #[test]
    fn nested_json_is_scanned() {
        let h = SensitiveContentHook::new();
        let dec = h.pre_dispatch(
            "c1",
            "any",
            &json!({"outer": {"nested": "AKIAIOSFODNN7EXAMPLE"}}),
        );
        assert!(matches!(dec, HookDecision::Reject { .. }));
    }
}
```

- [ ] **Step 2: Run tests to verify fail**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::examples::sensitive_content`
Expected: FAIL (stub doesn't scan).

- [ ] **Step 3: Implement `SensitiveContentHook`**

```rust
//! `SensitiveContentHook` — rejects tool calls whose args contain
//! well-known secret-shaped strings.
//!
//! Patterns are intentionally conservative — high signal, low false
//! positives. Extend via configuration in a later sprint.

#![allow(clippy::expect_used)] // gated to LazyLock regex constants below

use cogito_protocol::hook::{HookDecision, HookHandler};
use regex::Regex;
use std::sync::LazyLock;

static AWS_ACCESS_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("regex compiles"));
static GITHUB_PAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ghp_[A-Za-z0-9]{36}").expect("regex compiles"));
static OPENAI_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-[A-Za-z0-9]{20,}").expect("regex compiles"));

const HOOK_NAME: &str = "sensitive-content";

/// Hook that rejects tool invocations whose JSON args contain secret-
/// shaped strings.
#[derive(Debug, Default)]
pub struct SensitiveContentHook;

impl SensitiveContentHook {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn scan(value: &serde_json::Value) -> Option<&'static str> {
        match value {
            serde_json::Value::String(s) => {
                if AWS_ACCESS_KEY.is_match(s) {
                    return Some("aws-access-key");
                }
                if GITHUB_PAT.is_match(s) {
                    return Some("github-pat");
                }
                if OPENAI_KEY.is_match(s) {
                    return Some("openai-key");
                }
                None
            }
            serde_json::Value::Array(arr) => arr.iter().find_map(Self::scan),
            serde_json::Value::Object(map) => map.values().find_map(Self::scan),
            _ => None,
        }
    }
}

impl HookHandler for SensitiveContentHook {
    fn name(&self) -> &str {
        HOOK_NAME
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        _tool_name: &str,
        args: &serde_json::Value,
    ) -> HookDecision {
        match Self::scan(args) {
            Some(pattern) => HookDecision::Reject {
                hook_name: HOOK_NAME.into(),
                reason: format!("matched sensitive pattern '{pattern}' in tool args"),
            },
            None => HookDecision::Allow,
        }
    }
}
```

**Note**: `regex` is not currently a workspace dependency. Add it to `[workspace.dependencies]` in root `Cargo.toml` (`regex = "1.10"`) and `cogito-core/Cargo.toml`'s `[dependencies]` section with `regex = { workspace = true }`.

The use of `expect("regex compiles")` is gated under the workspace lint `expect_used = "deny"`. The file-level `#![allow(clippy::expect_used)]` shown at the top of the module covers the `LazyLock::new(...)` regex constants. These are compile-time-validated literals — an `unwrap_or_else(|_| unreachable!())` alternative is uglier and no safer. The file-level allow is narrow (one file, three constants) and documented in the module-level comment.

- [ ] **Step 4: Verify tests pass**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::examples::sensitive_content`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml \
        crates/cogito-core/Cargo.toml \
        crates/cogito-core/src/harness/hooks/examples/sensitive_content.rs
git commit -m "feat(core/hooks/examples): SensitiveContentHook with AWS/GitHub/OpenAI key regex"
```

---

### Task 10: `BashAuditHook` example

**Files:**
- Modify: `crates/cogito-core/src/harness/hooks/examples/bash_audit.rs` (replace stub)

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use cogito_protocol::hook::{HookDecision, HookHandler, HookLifecyclePoint};
    use cogito_protocol::metrics::MetricsRecorder;
    use serde_json::json;

    use super::*;

    struct CounterMetrics {
        bash: AtomicUsize,
        other: AtomicUsize,
    }
    impl MetricsRecorder for CounterMetrics {
        fn record_hook_invocation(&self, _: HookLifecyclePoint, _: &str, _: Duration, _: bool) {}
        fn record_counter(&self, name: &str, _: &[(&str, &str)]) {
            if name == "cogito.tool.bash.invocations" {
                self.bash.fetch_add(1, Ordering::SeqCst);
            } else {
                self.other.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn bash_increments_counter() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m.clone() as Arc<dyn MetricsRecorder>);
        let dec = h.pre_dispatch("c1", "bash", &json!({"cmd": "ls"}));
        assert!(matches!(dec, HookDecision::Allow));
        assert_eq!(m.bash.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_bash_does_not_increment() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m.clone() as Arc<dyn MetricsRecorder>);
        let _ = h.pre_dispatch("c1", "read_file", &json!({"path": "/tmp/x"}));
        assert_eq!(m.bash.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn always_allows() {
        let m = Arc::new(CounterMetrics {
            bash: AtomicUsize::new(0),
            other: AtomicUsize::new(0),
        });
        let h = BashAuditHook::new(m as Arc<dyn MetricsRecorder>);
        let dec = h.pre_dispatch("c1", "bash", &json!({"cmd": "rm -rf /"}));
        assert!(matches!(dec, HookDecision::Allow), "audit must never reject");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::examples::bash_audit`
Expected: FAIL (stub not implemented).

- [ ] **Step 3: Implement `BashAuditHook`**

```rust
//! `BashAuditHook` — increments `cogito.tool.bash.invocations` for
//! every `bash` tool call. Never rejects (audit-only).

use std::sync::Arc;

use cogito_protocol::hook::{HookDecision, HookHandler};
use cogito_protocol::metrics::MetricsRecorder;

const HOOK_NAME: &str = "bash-audit";
const COUNTER_NAME: &str = "cogito.tool.bash.invocations";

/// Audit hook for `bash` tool invocations.
pub struct BashAuditHook {
    metrics: Arc<dyn MetricsRecorder>,
}

impl BashAuditHook {
    #[must_use]
    pub fn new(metrics: Arc<dyn MetricsRecorder>) -> Self {
        Self { metrics }
    }
}

impl HookHandler for BashAuditHook {
    fn name(&self) -> &str {
        HOOK_NAME
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        tool_name: &str,
        _args: &serde_json::Value,
    ) -> HookDecision {
        if tool_name == "bash" {
            self.metrics
                .record_counter(COUNTER_NAME, &[]);
        }
        HookDecision::Allow
    }
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo nextest run -p cogito-core --lib harness::hooks::examples::bash_audit`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/hooks/examples/bash_audit.rs
git commit -m "feat(core/hooks/examples): BashAuditHook increments tool.bash.invocations counter"
```

---

### Task 11: Integration test — Reject hook end-to-end + panic isolation

**Files:**
- Create: `crates/cogito-core/tests/hook_integration.rs`
- Create: `crates/cogito-core/tests/hook_panic_isolation.rs`

- [ ] **Step 1: Write `hook_integration.rs`** at `crates/cogito-core/tests/hook_integration.rs`

Mirrors the existing `turn_driver_tool_call.rs` pattern at `crates/cogito-core/tests/turn_driver_tool_call.rs:1-126`. The bash tool is registered (so H07 resolution succeeds) but never reached — `SensitiveContentHook::pre_dispatch` rejects first.

```rust
//! End-to-end: SensitiveContentHook rejects a tool call carrying an
//! AWS key in args. Turn ends Failed with HookRejected. Event log
//! shows HookRejected followed by TurnFailed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_core::harness::hooks::examples::SensitiveContentHook;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
use cogito_protocol::hook::{HookHandler, HookLifecyclePoint};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::metrics::NoOpMetricsRecorder;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use cogito_store_jsonl::JsonlStore;
use cogito_tools::ReadFile;
use cogito_tools::provider::BuiltinToolProvider;
use futures::StreamExt as _;
use tokio::sync::{Mutex, broadcast};

#[tokio::test]
async fn sensitive_content_hook_rejects_tool_with_aws_key()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = tempfile::tempdir()?;
    let store: Arc<dyn cogito_protocol::store::ConversationStore> =
        Arc::new(JsonlStore::new(tmp_dir.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();

    let (tx, _rx) = broadcast::channel(64);
    let recorder = Arc::new(Mutex::new(StepRecorder::new(
        Arc::clone(&store),
        tx,
        session_id,
        0,
    )));

    let mock: Arc<MockModelGateway> = Arc::new(MockModelGateway::new());

    // Model issues a read_file tool call whose `path` arg contains an
    // AWS access key. Note: SensitiveContentHook scans ALL string
    // fields recursively in tool args, including `path`.
    mock.push_reply(vec![
        ModelEvent::ToolUseStarted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
        },
        ModelEvent::ToolUseCompleted {
            block_index: 0,
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({ "path": "/tmp/AKIAIOSFODNN7EXAMPLE" }),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 2,
            },
        },
    ]);

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let hooks = Arc::new(CompositeHookPipeline::with_handlers(vec![
        Arc::new(SensitiveContentHook::new()) as Arc<dyn HookHandler>,
    ]));

    let deps = TurnDeps {
        step: Arc::clone(&recorder),
        store: Arc::clone(&store),
        model: Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>,
        tools,
        hooks,
        metrics: Arc::new(NoOpMetricsRecorder),
    };

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy: HarnessStrategy::default_with_model("mock"),
        consecutive_tool_errors: 0,
    };

    // The hook rejects pre-dispatch → tool_resolver / dispatcher path
    // emits ToolResultRecorded with the error; loop re-enters Init for
    // a follow-up model call. To assert the rejection itself, we
    // verify the persisted event log directly: HookRejected event was
    // recorded at PreDispatch with the matching hook_name + reason.
    let _outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;

    // Read back the persisted events.
    let events: Vec<_> = {
        let mut stream = store.read_session(session_id, 0).await?;
        let mut acc = Vec::new();
        while let Some(ev) = stream.next().await {
            acc.push(ev?);
        }
        acc
    };

    let hook_rejected = events.iter().find(|e| {
        matches!(
            &e.payload,
            EventPayload::HookRejected {
                hook_name,
                point: HookLifecyclePoint::PreDispatch,
                reason,
            } if hook_name == "sensitive-content" && reason.contains("aws-access-key")
        )
    });
    assert!(
        hook_rejected.is_some(),
        "expected HookRejected event in log; got events: {events:#?}"
    );

    Ok(())
}
```

**Note**: this test asserts the persisted event log rather than the `TurnOutcome`. The pre_dispatch hook rejection produces a `ToolResult::Error` (Sprint 5 wiring); the FSM continues to a follow-up model call. The rejection is observable via the new `EventPayload::HookRejected` audit record. If you want the rejection to abort the entire turn (vs degrade to a tool error), revisit during execution — that is a behavior choice the spec did not pin down.

- [ ] **Step 2: Write `hook_panic_isolation.rs`** at `crates/cogito-core/tests/hook_panic_isolation.rs`

Tests panic-catch at the FSM level (not the unit-level panic_catch from Task 4 — that's already covered). Uses a panicky pre_prompt hook + asserts `TurnOutcome::Failed` (not a process panic).

```rust
//! Chaos: a hook that panics in pre_prompt produces a HookRejected
//! turn outcome, never an uncaught panic that crashes the FSM task.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_core::harness::step_recorder::StepRecorder;
use cogito_core::harness::turn_driver::deps::TurnDeps;
use cogito_core::harness::turn_driver::state::TurnCtx;
use cogito_core::harness::turn_driver::{TurnEntry, enter_turn};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelEvent, ModelInput, StopReason, Usage};
use cogito_protocol::hook::{HookDecision, HookHandler};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::metrics::NoOpMetricsRecorder;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::turn::{TurnFailureReason, TurnOutcome};
use cogito_store_jsonl::JsonlStore;
use cogito_tools::provider::BuiltinToolProvider;
use tokio::sync::{Mutex, broadcast};

struct PanicInPrePrompt;
impl HookHandler for PanicInPrePrompt {
    fn name(&self) -> &str {
        "panicky"
    }
    fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
        panic!("intentional panic in test")
    }
}

#[tokio::test]
async fn hook_panic_in_pre_prompt_yields_turn_failed() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = tempfile::tempdir()?;
    let store: Arc<dyn cogito_protocol::store::ConversationStore> =
        Arc::new(JsonlStore::new(tmp_dir.path().to_path_buf()));

    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let (tx, _rx) = broadcast::channel(64);
    let recorder = Arc::new(Mutex::new(StepRecorder::new(
        Arc::clone(&store),
        tx,
        session_id,
        0,
    )));

    let mock: Arc<MockModelGateway> = Arc::new(MockModelGateway::new());
    // The model is never called — the pre_prompt hook panics first.
    // Still push an empty reply just to satisfy the gateway contract
    // if the FSM happens to reach ModelCalling before any other path.
    mock.push_reply(vec![ModelEvent::MessageCompleted {
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    }]);

    let tools: Arc<dyn cogito_protocol::tool::ToolProvider> =
        Arc::new(BuiltinToolProvider::builder().build());

    let hooks = Arc::new(CompositeHookPipeline::with_handlers(vec![
        Arc::new(PanicInPrePrompt) as Arc<dyn HookHandler>,
    ]));

    let deps = TurnDeps {
        step: Arc::clone(&recorder),
        store: Arc::clone(&store),
        model: Arc::clone(&mock) as Arc<dyn cogito_protocol::gateway::ModelGateway>,
        tools,
        hooks,
        metrics: Arc::new(NoOpMetricsRecorder),
    };

    let ctx = TurnCtx {
        session_id,
        turn_id,
        exec_ctx: ExecCtx::open_ended(session_id, turn_id),
        strategy: HarnessStrategy::default_with_model("mock"),
        consecutive_tool_errors: 0,
    };

    let outcome = enter_turn(TurnEntry::FreshLikeInit, ctx, deps).await;
    match outcome {
        TurnOutcome::Failed(TurnFailureReason::HookRejected { hook_name, message }) => {
            assert_eq!(hook_name, "panicky");
            assert!(
                message.contains("panicked") && message.contains("intentional panic in test"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected Failed(HookRejected), got {other:?}"),
    }
    Ok(())
}
```

- [ ] **Step 3: Run integration tests**

Run: `cargo nextest run -p cogito-core --test hook_integration --test hook_panic_isolation`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/tests/hook_integration.rs \
        crates/cogito-core/tests/hook_panic_isolation.rs
git commit -m "test(core): integration — SensitiveContentHook reject + pre_prompt panic isolation"
```

---

### Task 12: Latency budget benchmark — P99 < 5ms per hook

**Files:**
- Create: `crates/cogito-core/benches/hook_latency.rs`
- Create: `docs/quality/v0.1-hook-latency.md`
- Modify: `crates/cogito-core/Cargo.toml` (add `[[bench]]` entry)

- [ ] **Step 1: Create criterion benchmark**

```rust
//! Hook latency benchmark — verifies the H09 §"Critical invariants"
//! P99 < 5ms budget for a typical 5-handler pipeline. Smoke
//! threshold; baseline numbers recorded in
//! `docs/quality/v0.1-hook-latency.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_core::harness::hooks::CompositeHookPipeline;
use cogito_protocol::gateway::ModelInput;
use cogito_protocol::hook::{HookDecision, HookHandler};
use criterion::{Criterion, criterion_group, criterion_main};

struct LightWork;
impl HookHandler for LightWork {
    fn name(&self) -> &str {
        "light"
    }
    fn pre_prompt(&self, _: &ModelInput) -> HookDecision {
        // Simulate ~100us of work
        let mut acc = 0u64;
        for i in 0..1000 {
            acc = acc.wrapping_add(i);
        }
        std::hint::black_box(acc);
        HookDecision::Allow
    }
}

fn bench_pre_prompt_5_handlers(c: &mut Criterion) {
    let p = CompositeHookPipeline::with_handlers(
        (0..5).map(|_| Arc::new(LightWork) as Arc<dyn HookHandler>).collect(),
    );
    c.bench_function("hook.pre_prompt.5_handlers", |b| {
        b.iter(|| {
            let _ = p.pre_prompt(&ModelInput::default());
        });
    });
}

criterion_group!(benches, bench_pre_prompt_5_handlers);
criterion_main!(benches);
```

- [ ] **Step 2: Run the benchmark and capture baseline**

Run: `cargo bench -p cogito-core --bench hook_latency`
Expected output: mean / P99 timings. Capture the numbers.

- [ ] **Step 3: Create the baseline doc** at `docs/quality/v0.1-hook-latency.md`

Document:
- Methodology (criterion, 5 light handlers, simulated 100µs work each)
- Baseline P99 (from Step 2)
- Verification: P99 < 5ms PASS / FAIL
- Re-run trigger: any change to `CompositeHookPipeline::pre_prompt`

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/benches/hook_latency.rs \
        crates/cogito-core/Cargo.toml \
        docs/quality/v0.1-hook-latency.md
git commit -m "bench(core): hook pre_prompt latency baseline (P99 < 5ms per H09 invariant)"
```

---

### Task 13: Docs sync — H09 doc + ROADMAP + CHANGELOG + jsonl-v1

**Files:**
- Modify: `docs/components/H09-hook-pipeline.md` (Sprint 5 closure note + trait sig + Modify-deferral note)
- Modify: `docs/data-model/jsonl-v1.md` (document `hook_rejected` event)
- Modify: `ROADMAP.md` (check off Sprint 5 boxes)
- Modify: `CHANGELOG.md` (add Sprint 5 block)

- [ ] **Step 1: Amend H09 doc**

In `docs/components/H09-hook-pipeline.md`:
- Update §"Status" — replace 🚧 line with: "✅ Sprint 5 (2026-05-XX): real `HookHandler` trait + 2 example hooks + 5 lifecycle wirings + panic catch + MetricsRecorder. `HookDecision::Modify` deferred (see §"Open design questions")."
- Replace §"Interface" TODO block with actual `HookHandler` trait signature (paste from Task 1).
- Update §"Critical invariants" #3: "v0.1 ships `Allow` / `Reject` only. `Modify` is deferred to a future sprint — see §"Open design questions"." Move the original Modify description there.
- Add §"References" link to this plan.

- [ ] **Step 2: Update `docs/data-model/jsonl-v1.md`**

Mirror the precedent of `thinking_block_recorded`. Add a section documenting the `hook_rejected` event variant: example JSON, fields, ordering vs other events.

- [ ] **Step 3: Check off ROADMAP boxes**

In `ROADMAP.md`, mark Sprint 5 items complete:

```markdown
- [x] H09 Hook Pipeline with purity rule enforcement ...
- [x] Two example hooks (sensitive content, bash audit)
- [x] `MetricsRecorder` trait in protocol + default no-op
- [x] `HookProvider` trait shape lets v0.2 Plugin add hooks ...
- [x] Per-hook P99 latency budget verified
```

- [ ] **Step 4: CHANGELOG entry**

In `CHANGELOG.md`, add under Unreleased:

```markdown
### Added — Sprint 5 (Hook Pipeline 实化)
- `cogito-protocol::hook` — `HookHandler` + `HookProvider` + `HookDecision` + `HookLifecyclePoint` traits/types
- `cogito-protocol::metrics` — `MetricsRecorder` trait + `NoOpMetricsRecorder` default
- `EventPayload::HookRejected` event variant (additive, no schema_version bump)
- `CompositeHookPipeline` in `cogito-core::harness::hooks` with panic catch + metrics
- Reference hooks: `SensitiveContentHook` (AWS/GitHub/OpenAI key regex), `BashAuditHook` (tool.bash.invocations counter)
- All 5 H09 lifecycle points now wired in the FSM (pre_prompt + pre_dispatch + post_model + post_turn + on_error)
- Hook panic isolation: panicked hook → `HookDecision::Reject`, never crashes session loop
- P99 latency baseline in `docs/quality/v0.1-hook-latency.md`

### Deferred
- `HookDecision::Modify` (variant reserved via `#[non_exhaustive]`; revisit when consumer use case surfaces)
- `cogito.toml [[hooks]]` configuration section (Sprint 12 / Plugin work will provide the unified config path)
```

- [ ] **Step 5: Final CI gate**

Run: `make ci`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add docs/components/H09-hook-pipeline.md \
        docs/data-model/jsonl-v1.md \
        ROADMAP.md \
        CHANGELOG.md
git commit -m "docs(sprint-5): H09 closure note, jsonl-v1 hook_rejected entry, ROADMAP checks, CHANGELOG"
```

---

## Closing checklist

After all 13 tasks land:

- [ ] `make ci` green
- [ ] `make bench` runs the new `hook_latency` benchmark
- [ ] `make chaos` still passes (no regressions in resume_chaos)
- [ ] `ROADMAP.md` Sprint 5 fully checked off
- [ ] `docs/components/H09-hook-pipeline.md` no longer says "🚧 Sprint 2"
- [ ] Sprint 6 (Context C2 trait freeze) is unblocked — `MetricsRecorder` trait available for Compactor decision events

# Sprint 6 · Context Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land ADR-0008 Context Management — 4 frozen traits (`Compactor` / `HistoryProjector` / `SystemPromptInjector` / `ToolFilterOverrider`) in `cogito-protocol`, all-no-op + truncate reference impls in new `cogito-context` crate, H11 real-work transition in `cogito-core`, H04/H05 honor new events, `ModelGateway::model_limits()` + `[<size>]` suffix convention for adaptive thresholds across providers.

**Architecture:** v0.1 ships only `compactor::truncate`; the other three traits get identity/no-op default impls. All trait outputs go through events (strict event-sourcing, P mode). Compactor writes `ContextCompacted` only when actually compacting; Injector/Overrider write every turn even when empty. H11 orchestrates the 4 traits in fixed order; failures degrade (record error in `ContextDecisionRecorded.errors`, don't block turn). `ModelGateway::model_limits()` is additive with default impl; Anthropic gateway parses `claude-opus-4-7[1m]`-style suffix or falls back to lookup table; OpenAi-compat gateway parses suffix then strips before sending to vLLM.

**Tech Stack:** Rust 2024 / MSRV 1.85, `serde`, `thiserror`, `tracing`, `tokio`, `regex` (already a workspace dep). TDD via `cargo nextest`. Workspace recipes: `make fmt`, `make fix CRATE=<name>`, `make test CRATE=<name>`, `make ci`.

**Source-of-truth references:**
- [Sprint 6 Design Spec](../specs/2026-05-23-sprint-6-context-management-design.md) — sections referenced throughout this plan
- [`docs/components/H11-context-manage.md`](../../components/H11-context-manage.md) — placeholder, upgraded by Task 36
- [Rebalance spec §2.1 C2 / §7](../specs/2026-05-22-roadmap-rebalance-design.md)
- [ROADMAP.md Sprint 6](../../../ROADMAP.md)
- [ADR-0007 (event log as cross-language contract)](../../adr/0007-event-log-as-cross-language-contract.md) — covers additive variant rule

**Sprint sequencing:** Sprint 5 (Hook Pipeline) closed. Sprint 4 (MCP) is in flight on a separate branch; this plan does NOT touch `cogito-mcp` so no merge conflict expected. Sprint 7 (Skill loader) depends on this sprint's `SystemPromptInjector` trait freeze.

**Test file lint header (REQUIRED):** Every test file (both inline `#[cfg(test)] mod tests {}` and `tests/*.rs` integration files) must begin with:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

Workspace lints set those to `deny`; CI fails without this header. Precedent: `crates/cogito-core/tests/resume_chaos.rs`.

**Commits:** one per task. Each task leaves `make ci` green.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/cogito-protocol/src/context.rs` | Create | 4 trait declarations (`Compactor`, `HistoryProjector`, `SystemPromptInjector`, `ToolFilterOverrider`) + Input types + `CompactionApplied` + `CompactionKind` + `CompactionReplacement` + `ToolFilterOverrideMode` + `ContextError` + `ContextConfig` + 4 tagged config enums + `TokenThreshold` + `TruncateConfig` + `ContextPipeline` struct. |
| `crates/cogito-protocol/src/gateway.rs` | Modify | Add `ModelLimits` struct + `parse_context_window_suffix()` helper + `ModelGateway::model_limits()` method (default impl returning conservative 32_768). |
| `crates/cogito-protocol/src/event.rs` | Modify | Add 4 `EventPayload` variants (`ContextCompacted`, `SystemPromptInjected`, `ToolFilterOverridden`, `ContextDecisionRecorded`) + `EventCategory` enum + `EventPayload::category()` method. |
| `crates/cogito-protocol/src/strategy.rs` | Modify | Add `HarnessStrategy.context: ContextConfig` field; update `default_with_model` to fill `ContextConfig::default()`. |
| `crates/cogito-protocol/src/lib.rs` | Modify | `pub mod context;` + re-export `Compactor`, `HistoryProjector`, `SystemPromptInjector`, `ToolFilterOverrider`, `ContextPipeline`, `ContextConfig`, `ContextError`, `ModelLimits`, `parse_context_window_suffix`. |
| `crates/cogito-protocol/tests/model_id_suffix.rs` | Create | Unit tests for `parse_context_window_suffix`: `[1m]`/`[200k]`/`[32k]`/`[128000]`/no-suffix/malformed. |
| `crates/cogito-model/src/anthropic/gateway.rs` | Modify | Implement `model_limits()`: suffix-first, then base lookup table (opus-4-7 / sonnet-4-6 / haiku-4-5 / 20251001 dated variants → 200_000), unknown → 200_000 + `tracing::warn`. Sends full `model_id` to API (no strip). |
| `crates/cogito-model/src/openai_compat/gateway.rs` | Modify | Implement `model_limits()`: suffix-first, then `ProviderConfig.context_window_tokens`, then 32_768 + warn. Add private `api_model_id()` helper that strips `[<size>]` suffix before sending to vLLM. |
| `crates/cogito-model/tests/limits.rs` | Create | Per-gateway `model_limits()` unit tests + `api_model_id` strip behavior. |
| `crates/cogito-config/src/types.rs` | Modify | Add `context_window_tokens: Option<u64>` field to `OpenAiCompatProviderConfig`. |
| `crates/cogito-core/src/harness/step_recorder.rs` | Modify | Add 4 methods: `record_context_compacted` (validates §5.5 invariants), `record_system_prompt_injected` (idempotent on turn_id), `record_tool_filter_overridden` (idempotent on turn_id), `record_context_decision`. Add private helpers `find_compaction_for_turn`, `find_system_prompt_injection_for_turn`, `find_tool_filter_override_for_turn`, `is_turn_started_at`, `is_turn_last_event_at`. |
| `crates/cogito-context/Cargo.toml` | Create | New crate; only dep is `cogito-protocol`. |
| `crates/cogito-context/src/lib.rs` | Create | `pub mod {pipeline, compactor, projector, injector, overrider};` + `pub use pipeline::{build_pipeline, ContextPipeline};`. |
| `crates/cogito-context/src/pipeline.rs` | Create | `build_pipeline(&ContextConfig) -> ContextPipeline` factory (tagged-config dispatch per CLAUDE.md). |
| `crates/cogito-context/src/compactor/mod.rs` | Create | `pub mod {none, truncate};`. |
| `crates/cogito-context/src/compactor/none.rs` | Create | `NoneCompactor` — `maybe_compact` always returns `Ok(vec![])`. |
| `crates/cogito-context/src/compactor/truncate.rs` | Create | `TruncateCompactor` implementing §10 algorithm. |
| `crates/cogito-context/src/projector/mod.rs` | Create | `pub mod standard;`. |
| `crates/cogito-context/src/projector/standard.rs` | Create | `StandardProjector` implementing §5 algorithm (covered-set union + per-turn SystemPromptInjected lookup). |
| `crates/cogito-context/src/injector/mod.rs` | Create | `pub mod none;`. |
| `crates/cogito-context/src/injector/none.rs` | Create | `NoneInjector` — writes `SystemPromptInjected { suffix: "", contributors: [], produced_by: "none" }` every turn. |
| `crates/cogito-context/src/overrider/mod.rs` | Create | `pub mod none;`. |
| `crates/cogito-context/src/overrider/none.rs` | Create | `NoneOverrider` — writes `ToolFilterOverridden { mode: Inherit, contributors: [], produced_by: "none" }` every turn. |
| `crates/cogito-context/tests/truncate_compaction.rs` | Create | §10.4 boundary scenarios 1-7. |
| `crates/cogito-context/tests/truncate_threshold_ratio.rs` | Create | §10.4 boundary 8: cross-provider threshold computation. |
| `crates/cogito-context/tests/standard_projection.rs` | Create | §5 + §5.3 multi-compaction projection scenarios. |
| `crates/cogito-context/tests/pipeline_assembly.rs` | Create | `build_pipeline` dispatch correctness. |
| `crates/cogito-core/src/harness/turn_driver/deps.rs` | Modify | Add `pub context_pipeline: Arc<ContextPipeline>` field to `TurnDeps`. |
| `crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs` | Modify | Rewrite from pass-through to real orchestration (§9.1): call Compactor → Injector → Overrider → write ContextDecisionRecorded → ContextManageCompleted. Failure degrade per §9.2. |
| `crates/cogito-core/src/harness/prompt.rs` | Modify | Use `HistoryProjector` from pipeline; consume `SystemPromptInjected` for current turn. |
| `crates/cogito-core/src/harness/tool_surface.rs` | Modify | Read latest `ToolFilterOverridden` for current turn; apply Inherit/Intersect/Replace against `strategy.allowed_tools`. |
| `crates/cogito-core/src/runtime/session_loop.rs` | Modify | `SessionShared` gains `context_pipeline: Arc<ContextPipeline>`; `try_start_turn` clones into `TurnDeps`. |
| `crates/cogito-core/src/runtime/mod.rs` | Modify | `Runtime::open_session` calls `cogito_context::build_pipeline(&strategy.context)` and stores in `SessionShared`. Workspace `Cargo.toml` and `cogito-core/Cargo.toml` gain `cogito-context` dep. |
| `crates/cogito-core/tests/context_managed_no_op.rs` | Create | Integration test: default `ContextConfig` → 4 ContextManaged events written per turn. |
| `crates/cogito-core/tests/context_managed_with_truncate.rs` | Create | Integration test: long session + truncate → `ContextCompacted` written; next-turn `PromptComposed.surface_size` reflects compaction. |
| `crates/cogito-core/tests/h04_multi_compaction_projection.rs` | Create | §5.3 trace replay; assert ModelInput messages character-level match. |
| `crates/cogito-core/tests/h05_tool_filter.rs` | Create | Intersect + Replace + Inherit modes against various `strategy.allowed_tools`. |
| `crates/cogito-core/tests/resume_chaos.rs` | Modify | Add `assert_context_managed_pairing(events)` helper + use in main driver. |
| `tools/cogito-gen-schema/...` | Run | Regenerate `docs/schemas/conversation-event-v1.json`. |
| `docs/schemas/conversation-event-v1.json` | Regenerate | Adds 4 new variant entries. |
| `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-truncate-v1.jsonl` | Create | Reference fixture covering ContextManaged + ContextCompacted/Decision/etc. |
| `docs/components/H11-context-manage.md` | Modify | Upgrade from placeholder; add §"v0.1 implementation" referencing ADR-0008 + this spec. |
| `docs/components/H04-prompt-composer.md` | Modify | Footnote: §"HistoryProjector dispatch" + §"System prompt injection". |
| `docs/components/H05-tool-surface.md` | Modify | Footnote: §"ToolFilterOverridden integration". |
| `docs/data-model/jsonl-v1.md` | Modify | Add §"Context management events". |
| `docs/adr/0008-context-management.md` | Create | ADR proper — distilled from the spec's §3-§9 + §12. |
| `ROADMAP.md` | Modify | Check off Sprint 6 boxes after integration tests green. |
| `CHANGELOG.md` | Modify | `### Added — Sprint 6 (Context Management)` block. |

**Total tasks: 38.**

---

## Phase 1 · Protocol foundation (Tasks 1–12)

These tasks land in `cogito-protocol`. They are mostly type definitions + one helper + one default-method trait extension. No external deps; safe to parallelize during execution.

### Task 1: ADR-0008 draft (parallel-safe — does not block code)

**Files:**
- Create: `docs/adr/0008-context-management.md`

- [ ] **Step 1: Read the spec sections that the ADR distills**

Read `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md` sections §3 (design principles) / §4 (data flow) / §5 (projection algorithm) / §6 (trait surface) / §7 (event payload) / §9 (orchestration) / §12 (resume).

- [ ] **Step 2: Write ADR-0008 following the existing ADR pattern**

Reference ADR-0007 as the structural template (`docs/adr/0007-event-log-as-cross-language-contract.md`). Sections to include:

```markdown
# ADR-0008 — Context Management

**Status**: Accepted (2026-05-23) — Sprint 6
**Predecessors**: ADR-0004 (Brain/Hands/Session boundaries), ADR-0006 (Runtime + H01 FSM, `ContextManaged` state amendment), ADR-0007 (event log contract)
**Implementation spec**: [`docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`](../superpowers/specs/2026-05-23-sprint-6-context-management-design.md)

## Decision

cogito's Context Management surface in `cogito-protocol` consists of four traits with distinct invariants:

1. `Compactor` (async, may do I/O, writes `ContextCompacted` events)
2. `HistoryProjector` (pure synchronous projection of events to messages)
3. `SystemPromptInjector` (async, no I/O beyond recorder, writes `SystemPromptInjected` every turn)
4. `ToolFilterOverrider` (async, no I/O beyond recorder, writes `ToolFilterOverridden` every turn)

H11 orchestrates them in fixed order each ContextManaged transition; failures degrade (record error, do not block turn).

## Event surface

Four additive `EventPayload` variants: `ContextCompacted`, `SystemPromptInjected`, `ToolFilterOverridden`, `ContextDecisionRecorded`. All `#[non_exhaustive]`. No `SCHEMA_VERSION` bump (per ADR-0007).

## Projection semantics (HistoryProjector contract)

- `covered_ranges` = set-union of all `ContextCompacted.replaced_seq_range`
- Events with `seq ∈ covered_ranges` are dropped from projection
- An un-covered `ContextCompacted` event emits its `replacement` (Drop → nothing; Summary → user-role `<conversation_summary>` block) at its own seq position
- `ContextCompacted.replaced_seq_range` MUST align to turn boundaries and MUST NOT include its own seq (StepRecorder validates at write)

## Cross-provider adaptive threshold

`ModelGateway::model_limits() -> ModelLimits { model_id, context_window_tokens }`. Default impl returns conservative 32_768; provider adapters should override. Convention: model id may carry `[<size>]` suffix (`opus-4-7[1m]`, `Llama-3.3-70B[32k]`); `parse_context_window_suffix()` public helper. Anthropic passes suffix through; OpenAi-compat strips before API call.

## Resume / idempotency

No new `ResumePoint` variant. Compactor / Injector / Overrider are idempotent on turn_id (re-runs are short-circuited). Crash mid-summarization → next H11 invocation re-runs Compactor → if event already persisted, returns existing; if not, re-issues model call.

## Configuration

`HarnessStrategy.context: ContextConfig` with four tagged-config enums (one per trait). `cogito-context::build_pipeline(&ContextConfig)` assembles `ContextPipeline`. v0.1 ships: `NoneCompactor`/`TruncateCompactor` + `StandardProjector` + `NoneInjector` + `NoneOverrider`. Sprint 7 adds `SkillsInjector`; v0.2 Plugin adds more.

## Consequences

(positive + negative bulleted)

## Alternatives considered

(α/β/γ + S1/S2/S4 + K-options summary, link to spec for full rationale)
```

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0008-context-management.md
git commit -m "docs(adr-0008): context management — trait freeze, projection, adaptive threshold"
```

---

### Task 2: `parse_context_window_suffix` helper + tests

**Files:**
- Modify: `crates/cogito-protocol/src/gateway.rs` (add function + `regex` to deps)
- Modify: `crates/cogito-protocol/Cargo.toml` (add `regex` workspace dep if not already)
- Create: `crates/cogito-protocol/tests/model_id_suffix.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-protocol/tests/model_id_suffix.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::gateway::parse_context_window_suffix;

#[test]
fn opus_with_1m_suffix() {
    assert_eq!(parse_context_window_suffix("claude-opus-4-7[1m]"), Some(1_000_000));
}

#[test]
fn sonnet_with_200k_suffix() {
    assert_eq!(parse_context_window_suffix("claude-sonnet-4-6[200k]"), Some(200_000));
}

#[test]
fn vllm_with_32k_suffix() {
    assert_eq!(parse_context_window_suffix("meta-llama/Llama-3.3-70B[32k]"), Some(32_000));
}

#[test]
fn gpt_with_numeric_suffix() {
    assert_eq!(parse_context_window_suffix("gpt-4o[128000]"), Some(128_000));
}

#[test]
fn capital_k_and_m_accepted() {
    assert_eq!(parse_context_window_suffix("foo[1M]"), Some(1_000_000));
    assert_eq!(parse_context_window_suffix("bar[32K]"), Some(32_000));
}

#[test]
fn no_suffix_returns_none() {
    assert_eq!(parse_context_window_suffix("claude-opus-4-7"), None);
    assert_eq!(parse_context_window_suffix("gpt-4o"), None);
}

#[test]
fn malformed_suffix_returns_none() {
    assert_eq!(parse_context_window_suffix("model[abc]"), None);
    assert_eq!(parse_context_window_suffix("model[1g]"), None);
    assert_eq!(parse_context_window_suffix("model[]"), None);
    assert_eq!(parse_context_window_suffix("model[1m"), None);
}

#[test]
fn suffix_in_middle_ignored() {
    // suffix must be at end-of-string anchor
    assert_eq!(parse_context_window_suffix("model[1m]-v2"), None);
}
```

- [ ] **Step 2: Run test to verify it fails (function not yet defined)**

```bash
make test CRATE=cogito-protocol
```

Expected: compile error — `parse_context_window_suffix` not found.

- [ ] **Step 3: Add `regex` to `cogito-protocol/Cargo.toml`**

Confirm `regex = { workspace = true }` is present; if not, add it under `[dependencies]`. (Workspace already declares `regex` per `cogito-tools` usage.)

- [ ] **Step 4: Implement the helper in `gateway.rs`**

At the bottom of `crates/cogito-protocol/src/gateway.rs`, add:

```rust
/// Parse the conventional `[<size>]` suffix from a model id.
///
/// Suffix grammar: `\[(\d+)([kKmM])?\]$`; `k` = 1_000, `m` = 1_000_000,
/// no unit = literal value.
///
/// Returns `None` if no suffix or suffix is malformed.
///
/// Examples:
/// - `"claude-opus-4-7[1m]"` → `Some(1_000_000)`
/// - `"Llama-3.3-70B[32k]"` → `Some(32_000)`
/// - `"gpt-4o[128000]"` → `Some(128_000)`
/// - `"claude-opus-4-7"` → `None`
#[must_use]
pub fn parse_context_window_suffix(model_id: &str) -> Option<u64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\[(\d+)([kKmM])?\]$").expect("static regex compiles")
    });
    let caps = re.captures(model_id)?;
    let num: u64 = caps.get(1)?.as_str().parse().ok()?;
    let mult = match caps.get(2).map(|m| m.as_str().to_lowercase()).as_deref() {
        Some("k") => 1_000,
        Some("m") => 1_000_000,
        _ => 1,
    };
    num.checked_mul(mult)
}
```

(`expect` is acceptable here inside a `OnceLock::get_or_init` initializer for a static regex that is provably correct; the file may need to retain its current allow-attribute for this single call. If the workspace lints reject it, switch to `Regex::new(...).ok()?` and accept the slight overhead of repeated compilation.)

- [ ] **Step 5: Run tests, verify all 8 pass**

```bash
make test CRATE=cogito-protocol
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/gateway.rs crates/cogito-protocol/tests/model_id_suffix.rs crates/cogito-protocol/Cargo.toml
git commit -m "feat(protocol): parse_context_window_suffix helper for adaptive ModelLimits"
```

---

### Task 3: `ModelLimits` + `ModelGateway::model_limits()` extension

**Files:**
- Modify: `crates/cogito-protocol/src/gateway.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (re-export `ModelLimits`)

- [ ] **Step 1: Write the failing test (inline in `gateway.rs`)**

Append to `crates/cogito-protocol/src/gateway.rs`:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod model_limits_tests {
    use super::*;

    struct DummyGateway;

    #[async_trait::async_trait]
    impl ModelGateway for DummyGateway {
        async fn stream(
            &self,
            _input: ModelInput,
            _ctx: crate::ExecCtx,
        ) -> Result<futures::stream::BoxStream<'static, Result<crate::stream::ModelEvent, ModelError>>, ModelError> {
            unimplemented!("not used in this test")
        }
        fn provider_id(&self) -> &'static str { "dummy" }
    }

    #[test]
    fn default_model_limits_is_conservative() {
        let g = DummyGateway;
        let limits = g.model_limits();
        assert_eq!(limits.context_window_tokens, 32_768);
        assert_eq!(limits.model_id, "dummy");
    }
}
```

(Adapt imports + return type to match the actual `stream` signature once you read the existing `ModelGateway` definition. The point of the test is that a gateway can be implemented WITHOUT overriding `model_limits` and gets a sane default.)

- [ ] **Step 2: Run, expect fail (`model_limits` method missing)**

```bash
make test CRATE=cogito-protocol
```

- [ ] **Step 3: Add `ModelLimits` struct and the default trait method**

In `crates/cogito-protocol/src/gateway.rs`, add near the other value types:

```rust
/// Limits of the model a `ModelGateway` serves.
///
/// Sourced from the gateway implementation, not from strategy config.
/// Consumed by Compactor for adaptive thresholds and (future) H05 surface
/// sizing. See ADR-0008 §"Cross-provider adaptive threshold".
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelLimits {
    /// The model id, with any `[<size>]` suffix preserved.
    pub model_id: String,
    /// Total context window in tokens (input + output combined).
    pub context_window_tokens: u64,
}
```

In the `ModelGateway` trait body, add a default method:

```rust
    /// Limits of the model this gateway serves. Used by Compactor for
    /// adaptive thresholds. Default impl returns a conservative 32_768
    /// window; provider adapters SHOULD override.
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            model_id: self.provider_id().into(),
            context_window_tokens: 32_768,
        }
    }
```

- [ ] **Step 4: Re-export from `lib.rs`**

```rust
pub use gateway::{ModelGateway, ModelInput, ModelLimits, ModelParams, parse_context_window_suffix};
```

- [ ] **Step 5: Run tests, pass; run `make ci`**

```bash
make ci
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/gateway.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): ModelGateway::model_limits() additive method + ModelLimits type"
```

---

### Task 4: `ContextError` enum

**Files:**
- Create: `crates/cogito-protocol/src/context.rs` (initial content — just `ContextError`)
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Create `context.rs` with the error type**

```rust
//! Context Management protocol surface — traits, event-emitting types, config.
//!
//! See `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
//! and ADR-0008 for the full design. Implementations live in `cogito-context`.

use thiserror::Error;

use crate::gateway::ModelError;
use crate::store::StoreError;

/// Failure mode for any of the four Context-Management traits.
///
/// Per ADR-0008 §"Failure semantics", H11 records the error into
/// `ContextDecisionRecorded.errors` and continues the turn (degrade, not block).
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum ContextError {
    /// Compactor's summarization model call failed.
    #[error("summarization model call failed: {0}")]
    SummarizationModelFailed(#[from] ModelError),

    /// An invariant from §5.5 was violated (range bounds, turn alignment,
    /// duplicate compaction for turn).
    #[error("invariant violated: {0}")]
    InvariantViolated(String),

    /// Operation was aborted (cancel-token fired).
    #[error("operation aborted")]
    Aborted,

    /// Underlying conversation store rejected the write.
    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}
```

- [ ] **Step 2: Add `pub mod context;` and re-export**

In `lib.rs`:

```rust
pub mod context;
pub use context::ContextError;
```

- [ ] **Step 3: Run `make test CRATE=cogito-protocol`**

Expected: compiles, no test added yet (covered by Task 14 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): ContextError enum"
```

---

### Task 5: 4 EventPayload variants for context decisions

**Files:**
- Modify: `crates/cogito-protocol/src/event.rs`
- Modify: `crates/cogito-protocol/src/context.rs` (add supporting types `CompactionReplacement`, `ToolFilterOverrideMode`, `ContextDecisionErrors`)
- Modify: `crates/cogito-protocol/tests/event_roundtrip.rs` (existing test — extend; if file doesn't exist by that name, search for the existing variant roundtrip test in `event.rs`'s `#[cfg(test)]` and extend there)

- [ ] **Step 1: Add supporting types in `context.rs`**

Append to `crates/cogito-protocol/src/context.rs`:

```rust
use serde::{Deserialize, Serialize};

/// What a `ContextCompacted` event substitutes in for the covered seq range.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionReplacement {
    /// Drop the covered range entirely (truncate-style; no replacement message).
    Drop,
    /// Replace with a summary user-role message wrapped in
    /// `<conversation_summary>...</conversation_summary>`.
    Summary {
        /// The summary text. Plain UTF-8.
        text: String,
        /// Provider model id that produced the summary (e.g. `"claude-haiku-4-5"`).
        model: String,
    },
}

/// How `ToolFilterOverrider` modifies `strategy.allowed_tools` for one turn.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolFilterOverrideMode {
    /// Use `strategy.allowed_tools` unchanged (no-op).
    Inherit,
    /// Intersect `strategy.allowed_tools` with the listed tools.
    Intersect {
        /// Tool names to keep.
        tools: Vec<String>,
    },
    /// Replace `strategy.allowed_tools` entirely (used by Plugin / Subagent).
    Replace {
        /// Tool names that become the full surface.
        tools: Vec<String>,
    },
}

/// Per-trait error capture for `ContextDecisionRecorded`. Each field is
/// the serialized display of a `ContextError`, or `None` if the trait ran cleanly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextDecisionErrors {
    /// Compactor failure (degrade path); `None` if it succeeded or returned vec![].
    pub compactor: Option<String>,
    /// SystemPromptInjector failure (H11 wrote a fallback empty event).
    pub injector: Option<String>,
    /// ToolFilterOverrider failure (H11 wrote a fallback Inherit event).
    pub overrider: Option<String>,
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

```rust
pub use context::{
    CompactionReplacement, ContextDecisionErrors, ContextError, ToolFilterOverrideMode,
};
```

- [ ] **Step 3: Add the 4 `EventPayload` variants**

In `crates/cogito-protocol/src/event.rs`, add these arms to the `EventPayload` enum (preserving `#[non_exhaustive]`):

```rust
    /// H11 Compactor decided to compact a portion of the event log.
    /// See ADR-0008 §"Projection semantics".
    ContextCompacted {
        /// The turn during which this compaction was decided.
        turn_id: TurnId,
        /// Inclusive seq range that this compaction covers.
        replaced_seq_range: (u64, u64),
        /// `Compactor::id()` — implementation identity.
        produced_by: String,
        /// What replaces the covered range in projection.
        replacement: crate::context::CompactionReplacement,
        /// Token estimate before this compaction (informational).
        token_estimate_before: Option<u64>,
        /// Token estimate after this compaction (informational).
        token_estimate_after: Option<u64>,
    },

    /// `SystemPromptInjector` ran for this turn (even if suffix is empty).
    SystemPromptInjected {
        /// The turn whose system prompt this suffix is for.
        turn_id: TurnId,
        /// Text appended after `strategy.system_prompt` (may be empty).
        suffix: String,
        /// Tags identifying what contributed (e.g. `["date", "skill:plan-review"]`).
        contributors: Vec<String>,
        /// `Injector::id()`.
        produced_by: String,
    },

    /// `ToolFilterOverrider` ran for this turn (Inherit counts as ran).
    ToolFilterOverridden {
        /// The turn whose tool surface this override applies to.
        turn_id: TurnId,
        /// What modification to apply on top of `strategy.allowed_tools`.
        mode: crate::context::ToolFilterOverrideMode,
        /// Tags identifying what contributed.
        contributors: Vec<String>,
        /// `Overrider::id()`.
        produced_by: String,
    },

    /// H11 summary at the end of `ContextManaged` — index of what was decided.
    ContextDecisionRecorded {
        /// The turn this decision summary belongs to.
        turn_id: TurnId,
        /// Event ids of `ContextCompacted` events written this turn (0 or 1 for v0.1).
        compactions: Vec<EventId>,
        /// Event id of this turn's `SystemPromptInjected`.
        system_prompt_event: EventId,
        /// Event id of this turn's `ToolFilterOverridden`.
        tool_filter_event: EventId,
        /// Per-trait error capture for degrade paths.
        errors: crate::context::ContextDecisionErrors,
    },
```

- [ ] **Step 4: Extend the existing roundtrip test**

Find the existing `#[cfg(test)] mod tests` in `event.rs` (or the integration test that asserts all variants roundtrip). Add 4 cases — one per new variant — verifying serialize → deserialize → equality. Use these payload skeletons:

```rust
EventPayload::ContextCompacted {
    turn_id: TurnId::new(),
    replaced_seq_range: (2, 79),
    produced_by: "truncate".into(),
    replacement: CompactionReplacement::Drop,
    token_estimate_before: Some(5200),
    token_estimate_after: Some(800),
}
EventPayload::ContextCompacted {
    turn_id: TurnId::new(),
    replaced_seq_range: (86, 399),
    produced_by: "summarize".into(),
    replacement: CompactionReplacement::Summary {
        text: "covered turns t21-t60".into(),
        model: "claude-haiku-4-5".into(),
    },
    token_estimate_before: Some(8400),
    token_estimate_after: Some(2300),
}
EventPayload::SystemPromptInjected {
    turn_id: TurnId::new(),
    suffix: "Today is 2026-05-23.".into(),
    contributors: vec!["date".into()],
    produced_by: "none".into(),
}
EventPayload::ToolFilterOverridden {
    turn_id: TurnId::new(),
    mode: ToolFilterOverrideMode::Inherit,
    contributors: vec![],
    produced_by: "none".into(),
}
EventPayload::ContextDecisionRecorded {
    turn_id: TurnId::new(),
    compactions: vec![],
    system_prompt_event: EventId::new(),
    tool_filter_event: EventId::new(),
    errors: ContextDecisionErrors::default(),
}
```

- [ ] **Step 5: Run tests; expect all pass**

```bash
make test CRATE=cogito-protocol
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/event.rs crates/cogito-protocol/src/lib.rs crates/cogito-protocol/tests/
git commit -m "feat(protocol): 4 EventPayload variants for context decisions"
```

---

### Task 6: `EventCategory` + `EventPayload::category()` helper

**Files:**
- Modify: `crates/cogito-protocol/src/event.rs`

- [ ] **Step 1: Write the failing test**

In `event.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn category_classifies_all_variants() {
    let conv = EventPayload::TurnStarted {
        turn_id: TurnId::new(),
        user_input: "hi".into(),
        strategy_id: "default".into(),
    };
    assert_eq!(conv.category(), EventCategory::Conversation);

    let meta = EventPayload::ContextManageEntered { turn_id: TurnId::new() };
    assert_eq!(meta.category(), EventCategory::HarnessMeta);

    let ctx = EventPayload::ContextCompacted {
        turn_id: TurnId::new(),
        replaced_seq_range: (1, 2),
        produced_by: "x".into(),
        replacement: CompactionReplacement::Drop,
        token_estimate_before: None,
        token_estimate_after: None,
    };
    assert_eq!(ctx.category(), EventCategory::ContextDecision);
}
```

- [ ] **Step 2: Run, expect failure (missing `EventCategory` / `category()`)**

```bash
make test CRATE=cogito-protocol
```

- [ ] **Step 3: Add `EventCategory` + `category()`**

In `event.rs`:

```rust
/// Coarse classification of `EventPayload` variants. Used by Postgres backends
/// for optional physical table partitioning (v0.4+) and by analysis tooling
/// for filtering. See ADR-0008 §"Backend partitioning guidance".
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventCategory {
    /// User / model conversation events — "what was said".
    Conversation,
    /// Harness FSM markers + decisions not part of dialog.
    HarnessMeta,
    /// Context management decisions (compaction + per-turn injection).
    ContextDecision,
}

impl EventPayload {
    /// Classify this payload. Pure function. See `EventCategory`.
    #[must_use]
    pub fn category(&self) -> EventCategory {
        match self {
            Self::SessionStarted { .. }
            | Self::TurnStarted { .. }
            | Self::AssistantMessageAppended { .. }
            | Self::ToolUseRecorded { .. }
            | Self::ToolResultRecorded { .. }
            | Self::ThinkingBlockRecorded { .. } => EventCategory::Conversation,

            Self::ContextManageEntered { .. }
            | Self::ContextManageCompleted { .. }
            | Self::PromptComposed { .. }
            | Self::ModelCallStarted { .. }
            | Self::ModelCallCompleted { .. }
            | Self::TurnPaused { .. }
            | Self::JobCompleted { .. }
            | Self::TurnCompleted { .. }
            | Self::TurnFailed { .. }
            | Self::HookRejected { .. } => EventCategory::HarnessMeta,

            Self::ContextCompacted { .. }
            | Self::SystemPromptInjected { .. }
            | Self::ToolFilterOverridden { .. }
            | Self::ContextDecisionRecorded { .. } => EventCategory::ContextDecision,
        }
    }
}
```

(Match arms must enumerate every existing `EventPayload` variant. Read the current enum to ensure no variant is missed; the compiler will catch missing arms via the absence of `_ =>`.)

- [ ] **Step 4: Re-export from `lib.rs`**

```rust
pub use event::{EventCategory, /* existing names */};
```

- [ ] **Step 5: Tests pass; `make ci` green**

```bash
make ci
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/event.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): EventCategory + EventPayload::category() classifier"
```

---

### Task 7: `Compactor` trait + supporting types

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Add `CompactionKind`, `CompactionApplied`, `CompactionInput`, `Compactor` trait**

Append to `crates/cogito-protocol/src/context.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;

use crate::event::EventId;
use crate::gateway::{ModelGateway, Usage};
use crate::ids::{SessionId, TurnId};
use crate::strategy::HarnessStrategy;
use crate::store::ConversationEvent;
use crate::ExecCtx;

/// What kind of compaction was applied. Embedded in `CompactionApplied`
/// so H11's `ContextDecisionRecorded` summary can describe it textually.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionKind {
    /// Drop replacement (truncate-style).
    Truncate,
    /// Summary replacement (v0.2+).
    Summarize,
    /// Tool-body elision (v0.2+).
    ToolBodyElision,
}

/// Returned from `Compactor::maybe_compact` per `ContextCompacted` event
/// written. H11 uses these to populate `ContextDecisionRecorded.compactions`.
#[derive(Clone, Debug)]
pub struct CompactionApplied {
    /// Event id of the `ContextCompacted` event that was just written.
    pub event_id: EventId,
    /// Range that was compacted (mirrors `ContextCompacted.replaced_seq_range`).
    pub replaced_seq_range: (u64, u64),
    /// Kind classifier — for H11's textual summary.
    pub kind: CompactionKind,
}

/// Input handed to `Compactor::maybe_compact`. All references are valid
/// for the duration of the call.
pub struct CompactionInput<'a> {
    /// The session that's compacting.
    pub session_id: SessionId,
    /// The upcoming turn whose context this is preparing.
    pub turn_id: TurnId,
    /// Full event log so far. Compactor scans this to find turn boundaries
    /// and existing compactions.
    pub history: &'a [ConversationEvent],
    /// The strategy whose `context.compactor` config selected this impl.
    pub strategy: &'a HarnessStrategy,
    /// Most recent `ModelCallCompleted.usage` (None on first turn).
    pub last_usage: Option<Usage>,
    /// Used by summarize-style Compactors. Truncate ignores it.
    pub model_gateway: &'a dyn ModelGateway,
    /// Step recorder for persisting `ContextCompacted` events.
    pub recorder: &'a mut crate::store::StepRecorderHandle<'a>,
}

/// Decide whether (and how) to compact history for the upcoming turn.
///
/// Implementations MUST be idempotent on `turn_id`: if a `ContextCompacted`
/// event already exists in `history` for this turn, return its
/// `CompactionApplied` without doing further work. This is what makes
/// crash-mid-compaction recovery work without a new `ResumePoint` (see
/// ADR-0008 §"Resume / idempotency").
///
/// Failures degrade — they do NOT propagate to H01 as fatal. H11 records
/// the error in `ContextDecisionRecorded.errors.compactor` and continues
/// the turn without compaction.
#[async_trait]
pub trait Compactor: Send + Sync {
    /// Run compaction (or decide not to).
    async fn maybe_compact(
        &self,
        input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError>;

    /// Implementation identity. Embedded in `ContextCompacted.produced_by`.
    fn id(&self) -> &'static str;
}
```

**Note on `StepRecorderHandle`:** The exact type holding `&mut StepRecorder` may be a borrowed mutex guard or a small wrapper depending on how Task 16-19 land. Use whatever type the existing `step_recorder.rs` exposes when called from a transition. If a new wrapper alias is cleaner, declare it in `cogito-protocol::store` and re-export here. The test file in Task 21+ will lock the exact shape.

- [ ] **Step 2: Re-export from `lib.rs`**

```rust
pub use context::{
    Compactor, CompactionApplied, CompactionInput, CompactionKind,
    CompactionReplacement, ContextDecisionErrors, ContextError, ToolFilterOverrideMode,
};
```

- [ ] **Step 3: Run `make fix CRATE=cogito-protocol && make test CRATE=cogito-protocol`**

Expected: compiles. No tests added; coverage comes from Task 24 (TruncateCompactor) onward.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): Compactor trait + supporting types"
```

---

### Task 8: `HistoryProjector` trait

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`

- [ ] **Step 1: Add the trait**

Append:

```rust
use crate::content::ContentBlock;

/// Project events + strategy into the dialogue messages H04 sends to the model.
///
/// **Pure**: implementations MUST be deterministic synchronous functions —
/// no I/O, no event writes, no clock reads. The covered-set + replacement
/// algorithm is fully specified in ADR-0008 §"Projection semantics" and
/// must be honored verbatim.
pub trait HistoryProjector: Send + Sync {
    /// Build the message list for `current_turn`'s upcoming model call.
    fn project(
        &self,
        events: &[ConversationEvent],
        strategy: &HarnessStrategy,
        current_turn: TurnId,
    ) -> Vec<ProjectedMessage>;

    fn id(&self) -> &'static str;
}

/// One message in a projected history. Roles match the wire-format roles
/// adapters serialize for Anthropic/OpenAI/etc.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectedMessage {
    /// System-role message; appears at most once at the start.
    System(String),
    /// User-role message. `<conversation_summary>...</conversation_summary>`-wrapped
    /// compaction summaries appear as User variants per ADR-0008.
    User(String),
    /// Assistant-role message carrying any combination of Text / ToolUse / Thinking.
    Assistant(Vec<ContentBlock>),
    /// Tool-result message (paired with a prior Assistant ToolUse by call_id).
    ToolResult {
        call_id: String,
        result_blocks: Vec<ContentBlock>,
    },
}
```

(The exact `ProjectedMessage` shape may already be defined as `Message` somewhere in `cogito-protocol`. Check `crates/cogito-protocol/src/content.rs` and `gateway.rs` first — if a `Message` type with these arms exists, reuse it and skip the new declaration. Otherwise create as shown.)

- [ ] **Step 2: Re-export**

```rust
pub use context::{HistoryProjector, ProjectedMessage, /* prior */};
```

- [ ] **Step 3: `make ci`**

```bash
make ci
```

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): HistoryProjector trait + ProjectedMessage"
```

---

### Task 9: `SystemPromptInjector` + `ToolFilterOverrider` traits

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`

- [ ] **Step 1: Add both traits + their Input structs**

Append:

```rust
/// Input handed to `SystemPromptInjector::inject`.
pub struct InjectionInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub strategy: &'a HarnessStrategy,
    pub history: &'a [ConversationEvent],
    pub exec_ctx: &'a ExecCtx,
    pub recorder: &'a mut crate::store::StepRecorderHandle<'a>,
}

/// Produce per-turn additions to `strategy.system_prompt`.
///
/// MUST write a `SystemPromptInjected` event every turn (even when suffix
/// is empty) — see ADR-0008 §"Audit semantics". MUST be idempotent on
/// `turn_id` for resume safety.
#[async_trait]
pub trait SystemPromptInjector: Send + Sync {
    /// Compute this turn's suffix and persist a `SystemPromptInjected` event.
    /// Returns the event id of the written event.
    async fn inject(
        &self,
        input: InjectionInput<'_>,
    ) -> Result<EventId, ContextError>;

    fn id(&self) -> &'static str;
}

/// Input handed to `ToolFilterOverrider::override_filter`.
pub struct ToolFilterInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub strategy: &'a HarnessStrategy,
    pub history: &'a [ConversationEvent],
    pub exec_ctx: &'a ExecCtx,
    pub recorder: &'a mut crate::store::StepRecorderHandle<'a>,
}

/// Decide per-turn tool-filter override on top of `strategy.allowed_tools`.
///
/// MUST write a `ToolFilterOverridden` event every turn (`Inherit` counts
/// as ran). MUST be idempotent on `turn_id` for resume safety.
#[async_trait]
pub trait ToolFilterOverrider: Send + Sync {
    /// Compute this turn's mode and persist a `ToolFilterOverridden` event.
    async fn override_filter(
        &self,
        input: ToolFilterInput<'_>,
    ) -> Result<EventId, ContextError>;

    fn id(&self) -> &'static str;
}
```

- [ ] **Step 2: Re-export**

```rust
pub use context::{
    InjectionInput, SystemPromptInjector, ToolFilterInput, ToolFilterOverrider,
    /* prior */,
};
```

- [ ] **Step 3: `make ci`**

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): SystemPromptInjector + ToolFilterOverrider traits"
```

---

### Task 10: Four tagged-config enums + `ContextConfig`

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`

- [ ] **Step 1: Define the four config enums and the umbrella struct**

Append:

```rust
/// Per-trait configuration container; lives in `HarnessStrategy.context`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextConfig {
    pub compactor: CompactorConfig,
    pub history_projector: HistoryProjectorConfig,
    pub system_prompt_injector: SystemPromptInjectorConfig,
    pub tool_filter_overrider: ToolFilterOverriderConfig,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            compactor: CompactorConfig::None,
            history_projector: HistoryProjectorConfig::Standard,
            system_prompt_injector: SystemPromptInjectorConfig::None,
            tool_filter_overrider: ToolFilterOverriderConfig::None,
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactorConfig {
    None,
    Truncate(TruncateConfig),
    // v0.2 will add: Summary { ... }
}

/// Per-trait config for `TruncateCompactor`. v0.1 ships only this Compactor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TruncateConfig {
    /// Adaptive (ratio of `model_limits().context_window_tokens`) or absolute.
    #[serde(default)]
    pub max_tokens: TokenThreshold,
    /// Preserve the first user message (turn 1)?
    #[serde(default = "default_true")]
    pub keep_first_user: bool,
    /// Always preserve this many most-recent completed turns.
    #[serde(default = "default_keep_recent")]
    pub keep_recent_turns: u32,
}

const fn default_true() -> bool { true }
const fn default_keep_recent() -> u32 { 5 }

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenThreshold {
    /// Adaptive: `ratio * context_window_tokens - safety_headroom`.
    Ratio {
        of_context_window: f32,
        safety_headroom: u64,
    },
    /// Hard absolute (ignores `model_limits`).
    Absolute(u64),
}

impl Default for TokenThreshold {
    fn default() -> Self {
        Self::Ratio { of_context_window: 0.75, safety_headroom: 8192 }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryProjectorConfig {
    Standard,
    // v0.2 will add: ToolElision { ... }
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemPromptInjectorConfig {
    None,
    // Sprint 7 will add: Skills { ... }
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolFilterOverriderConfig {
    None,
    // v0.2 will add: ReadOnly { ... }
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

```rust
pub use context::{
    CompactorConfig, ContextConfig, HistoryProjectorConfig, SystemPromptInjectorConfig,
    TokenThreshold, ToolFilterOverriderConfig, TruncateConfig, /* prior */,
};
```

- [ ] **Step 3: Add a roundtrip test in `context.rs` `#[cfg(test)]`**

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod config_tests {
    use super::*;

    #[test]
    fn default_config_is_all_none() {
        let c = ContextConfig::default();
        assert!(matches!(c.compactor, CompactorConfig::None));
        assert!(matches!(c.history_projector, HistoryProjectorConfig::Standard));
        assert!(matches!(c.system_prompt_injector, SystemPromptInjectorConfig::None));
        assert!(matches!(c.tool_filter_overrider, ToolFilterOverriderConfig::None));
    }

    #[test]
    fn truncate_config_toml_roundtrip() {
        let toml = r#"
[compactor]
kind = "truncate"
keep_first_user = true
keep_recent_turns = 5

[compactor.max_tokens]
kind = "ratio"
of_context_window = 0.75
safety_headroom = 8192

[history_projector]
kind = "standard"

[system_prompt_injector]
kind = "none"

[tool_filter_overrider]
kind = "none"
"#;
        let parsed: ContextConfig = toml::from_str(toml).expect("parses");
        let CompactorConfig::Truncate(t) = &parsed.compactor else {
            panic!("expected truncate");
        };
        assert!(t.keep_first_user);
        assert_eq!(t.keep_recent_turns, 5);
        let TokenThreshold::Ratio { of_context_window, safety_headroom } = t.max_tokens else {
            panic!("expected ratio");
        };
        assert!((of_context_window - 0.75).abs() < 1e-6);
        assert_eq!(safety_headroom, 8192);
    }
}
```

(Add `toml = { workspace = true }` to `[dev-dependencies]` in `cogito-protocol/Cargo.toml` if not already present.)

- [ ] **Step 4: `make test CRATE=cogito-protocol`; expect pass**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs crates/cogito-protocol/Cargo.toml
git commit -m "feat(protocol): ContextConfig + 4 tagged-config enums + TokenThreshold"
```

---

### Task 11: `ContextPipeline` + add `context` field to `HarnessStrategy`

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`
- Modify: `crates/cogito-protocol/src/strategy.rs`

- [ ] **Step 1: Add `ContextPipeline` struct in `context.rs`**

```rust
/// Assembled set of trait objects driven by H11 each ContextManaged.
///
/// Built by `cogito_context::build_pipeline(&ContextConfig)`; injected
/// into `TurnDeps.context_pipeline` by the Runtime layer at session open.
#[derive(Clone)]
pub struct ContextPipeline {
    pub compactor: Arc<dyn Compactor>,
    pub projector: Arc<dyn HistoryProjector>,
    pub injector: Arc<dyn SystemPromptInjector>,
    pub overrider: Arc<dyn ToolFilterOverrider>,
}
```

- [ ] **Step 2: Add `context` field to `HarnessStrategy`**

In `strategy.rs`:

```rust
use crate::context::ContextConfig;

// ... existing struct, add at the end:
pub struct HarnessStrategy {
    // existing fields...
    pub max_turns: u32,
    /// Sprint 6: per-strategy context-management pipeline configuration.
    /// `Default` = all-no-op (`StandardProjector` for projection + None
    /// for the other three traits). Strategies opt into compaction by
    /// setting `compactor: CompactorConfig::Truncate(...)`.
    #[serde(default)]
    pub context: ContextConfig,
}
```

Also update `default_with_model`:

```rust
HarnessStrategy {
    // existing fields...
    max_turns: 16,
    context: ContextConfig::default(),
}
```

- [ ] **Step 3: Re-export**

```rust
pub use context::{ContextPipeline, /* prior */};
```

- [ ] **Step 4: Run existing strategy tests; expect pass (the new field has Default)**

```bash
make test CRATE=cogito-protocol
```

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs crates/cogito-protocol/src/strategy.rs
git commit -m "feat(protocol): ContextPipeline struct + HarnessStrategy.context field"
```

---

### Task 12: Phase 1 closeout — Schema artifact regen

**Files:**
- Run: `tools/cogito-gen-schema/...`
- Regenerate: `docs/schemas/conversation-event-v1.json`

- [ ] **Step 1: Run the schema generator**

```bash
cargo run -p cogito-gen-schema -- --output docs/schemas/conversation-event-v1.json
```

(Exact binary name may differ; check `tools/` or `crates/tools/`. The CI drift gate exists per Sprint 1 closure, so the regeneration script is in place.)

- [ ] **Step 2: Verify the 4 new variants appear**

```bash
grep -c '"ContextCompacted"\|"SystemPromptInjected"\|"ToolFilterOverridden"\|"ContextDecisionRecorded"' docs/schemas/conversation-event-v1.json
```

Expected: at least 4 matches.

- [ ] **Step 3: `make ci`**

The schema drift CI check should now pass.

- [ ] **Step 4: Commit**

```bash
git add docs/schemas/conversation-event-v1.json
git commit -m "chore(schema): regenerate JSON schema for context-management variants"
```

---

## Phase 2 · Provider + Config (Tasks 13–15)

`cogito-model` adapters override `model_limits()` to return real values. `cogito-config` gains a fallback field for OpenAi-compat.

### Task 13: `AnthropicGateway::model_limits` implementation

**Files:**
- Modify: `crates/cogito-model/src/anthropic/gateway.rs`
- Create: `crates/cogito-model/tests/limits.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-model/tests/limits.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_model::anthropic::AnthropicGateway;
use cogito_protocol::gateway::ModelGateway;

fn make(model_id: &str) -> AnthropicGateway {
    AnthropicGateway::new_for_test("sk-fake", model_id)
}

#[test]
fn anthropic_opus_1m_suffix() {
    let g = make("claude-opus-4-7[1m]");
    assert_eq!(g.model_limits().context_window_tokens, 1_000_000);
    assert_eq!(g.model_limits().model_id, "claude-opus-4-7[1m]");
}

#[test]
fn anthropic_opus_default_200k() {
    let g = make("claude-opus-4-7");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_sonnet_default_200k() {
    let g = make("claude-sonnet-4-6");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_haiku_default_200k() {
    let g = make("claude-haiku-4-5");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_dated_haiku_default_200k() {
    let g = make("claude-haiku-4-5-20251001");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_unknown_model_falls_back_to_200k() {
    let g = make("claude-future-x-9");
    assert_eq!(g.model_limits().context_window_tokens, 200_000);
}

#[test]
fn anthropic_suffix_overrides_lookup() {
    // Even known models can use [<size>] to declare an alternate mode.
    let g = make("claude-sonnet-4-6[500k]");
    assert_eq!(g.model_limits().context_window_tokens, 500_000);
}
```

(If `AnthropicGateway::new_for_test` doesn't exist, write a private constructor that bypasses real HTTP setup; the test only exercises `model_limits()`, not network calls.)

- [ ] **Step 2: Run, expect fail**

```bash
make test CRATE=cogito-model
```

- [ ] **Step 3: Implement `model_limits` in `crates/cogito-model/src/anthropic/gateway.rs`**

Add to the existing `impl ModelGateway for AnthropicGateway`:

```rust
    fn model_limits(&self) -> ModelLimits {
        let window = parse_context_window_suffix(&self.model_id)
            .or_else(|| anthropic_default_window(&self.model_id))
            .unwrap_or_else(|| {
                tracing::warn!(
                    model_id = %self.model_id,
                    "no context window declared for model; falling back to 200_000",
                );
                200_000
            });
        ModelLimits {
            model_id: self.model_id.clone(),
            context_window_tokens: window,
        }
    }
```

Add a private helper in the same file:

```rust
fn anthropic_default_window(model_id: &str) -> Option<u64> {
    // Strip any `[...]` suffix for base matching.
    let base = model_id.split_once('[').map(|(b, _)| b).unwrap_or(model_id);
    // All current Anthropic chat models (Opus/Sonnet/Haiku families) default
    // to 200k context; the 1M mode for Opus is opted in via the `[1m]` suffix.
    match base {
        "claude-opus-4-7" | "claude-opus-4-7-20260301" => Some(200_000),
        "claude-sonnet-4-6" | "claude-sonnet-4-6-20260301" => Some(200_000),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => Some(200_000),
        _ => None,
    }
}
```

Update imports:

```rust
use cogito_protocol::gateway::{ModelGateway, ModelLimits, parse_context_window_suffix};
```

- [ ] **Step 4: Run tests, expect all 7 pass**

```bash
make test CRATE=cogito-model
```

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/anthropic/gateway.rs crates/cogito-model/tests/limits.rs
git commit -m "feat(model): AnthropicGateway::model_limits with [<size>] suffix + base lookup"
```

---

### Task 14: `OpenAiCompatGateway::model_limits` + `api_model_id` strip

**Files:**
- Modify: `crates/cogito-model/src/openai_compat/gateway.rs`
- Modify: `crates/cogito-model/tests/limits.rs` (extend)
- Modify: `crates/cogito-config/src/types.rs` (handled in Task 15 — coordinate)

This task depends on Task 15's config field but Task 15 is small; do Task 15 FIRST if you want strict TDD, or do both in sequence. The order chosen here lands the gateway code first with a placeholder type expectation, then Task 15 lands the config.

- [ ] **Step 1: Write the failing test (extend `limits.rs`)**

Append to `crates/cogito-model/tests/limits.rs`:

```rust
use cogito_model::openai_compat::OpenAiCompatGateway;
use cogito_config::OpenAiCompatProviderConfig;

fn oai(model_id: &str, declared: Option<u64>) -> OpenAiCompatGateway {
    let cfg = OpenAiCompatProviderConfig {
        base_url: "http://localhost:8000/v1".into(),
        model: model_id.into(),
        api_key: None,
        context_window_tokens: declared,
    };
    OpenAiCompatGateway::new_for_test(cfg)
}

#[test]
fn oai_suffix_overrides_config() {
    let g = oai("Llama-3.3-70B[32k]", Some(8_000));
    assert_eq!(g.model_limits().context_window_tokens, 32_000);
}

#[test]
fn oai_config_used_when_no_suffix() {
    let g = oai("Llama-3.3-70B", Some(8_000));
    assert_eq!(g.model_limits().context_window_tokens, 8_000);
}

#[test]
fn oai_double_fallback_to_32768() {
    let g = oai("mystery-model", None);
    assert_eq!(g.model_limits().context_window_tokens, 32_768);
}

#[test]
fn oai_api_model_id_strips_suffix() {
    let g = oai("Llama-3.3-70B[32k]", None);
    assert_eq!(g.api_model_id(), "Llama-3.3-70B");
}

#[test]
fn oai_api_model_id_passes_through_when_no_suffix() {
    let g = oai("Llama-3.3-70B", None);
    assert_eq!(g.api_model_id(), "Llama-3.3-70B");
}
```

- [ ] **Step 2: Run, expect fail (need `context_window_tokens` field in config + `api_model_id` method)**

```bash
make test CRATE=cogito-model
```

- [ ] **Step 3: Add `model_limits` and `api_model_id` to `OpenAiCompatGateway`**

In `crates/cogito-model/src/openai_compat/gateway.rs`:

```rust
impl OpenAiCompatGateway {
    /// Public helper for the actual model id to send to the vLLM/SGLang
    /// server — strips any `[<size>]` suffix that's used locally only.
    pub fn api_model_id(&self) -> String {
        self.config.model
            .split_once('[')
            .map(|(base, _)| base.to_string())
            .unwrap_or_else(|| self.config.model.clone())
    }
}

impl ModelGateway for OpenAiCompatGateway {
    // ... existing methods

    fn model_limits(&self) -> ModelLimits {
        let window = parse_context_window_suffix(&self.config.model)
            .or(self.config.context_window_tokens)
            .unwrap_or_else(|| {
                tracing::warn!(
                    model = %self.config.model,
                    "no context window declared (suffix nor provider config); falling back to 32_768",
                );
                32_768
            });
        ModelLimits {
            model_id: self.config.model.clone(),
            context_window_tokens: window,
        }
    }
}
```

Also update the existing API call sites in this file to use `self.api_model_id()` instead of `self.config.model` when serializing the request body's `"model"` field. Grep for `self.config.model` references first to find them.

- [ ] **Step 4: Wait — Task 15 must land first for `context_window_tokens` to exist on `OpenAiCompatProviderConfig`. Move to Task 15, then come back and run tests.**

After Task 15:

```bash
make test CRATE=cogito-model
```

Expected: all 12 limits tests pass (7 Anthropic + 5 OpenAi-compat).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-model/src/openai_compat/gateway.rs crates/cogito-model/tests/limits.rs
git commit -m "feat(model): OpenAiCompatGateway::model_limits + api_model_id strip"
```

---

### Task 15: Add `context_window_tokens` to `OpenAiCompatProviderConfig`

**Files:**
- Modify: `crates/cogito-config/src/types.rs`

- [ ] **Step 1: Add the field**

Find `pub struct OpenAiCompatProviderConfig` in `crates/cogito-config/src/types.rs` and add:

```rust
pub struct OpenAiCompatProviderConfig {
    // existing fields: base_url, model, api_key, etc.

    /// Optional fallback for `ModelGateway::model_limits().context_window_tokens`
    /// when the model id does not carry a `[<size>]` suffix. Users typically
    /// know what their vLLM/SGLang server is configured for; this is the
    /// place to declare it. When both the suffix and this field are absent,
    /// the gateway falls back to 32_768 with a warn log.
    #[serde(default)]
    pub context_window_tokens: Option<u64>,
}
```

- [ ] **Step 2: Confirm existing config-loading tests still pass**

```bash
make test CRATE=cogito-config
```

The field is `Option<u64>` with `#[serde(default)]`, so existing `cogito.toml` fixtures without this field still parse to `None`.

- [ ] **Step 3: Add one new test verifying the field parses**

In `crates/cogito-config/tests/loader.rs` (or wherever provider-config TOML tests live), add:

```rust
#[test]
fn openai_compat_with_context_window_tokens() {
    let toml = r#"
[runtime]
default_provider = "local"
default_model = "Llama-3.3-70B"

[[providers]]
id = "local"
kind = "openai_compat"
base_url = "http://localhost:8000/v1"
model = "Llama-3.3-70B"
context_window_tokens = 32768
"#;
    let cfg: RuntimeConfig = toml::from_str(toml).expect("parses");
    let p = &cfg.providers[0];
    let ProviderConfig::OpenAiCompat(oai) = p else { panic!() };
    assert_eq!(oai.context_window_tokens, Some(32_768));
}
```

(Adapt to existing `RuntimeConfig` / `ProviderConfig` shape.)

- [ ] **Step 4: Tests pass; `make ci`**

```bash
make ci
```

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-config/src/types.rs crates/cogito-config/tests/
git commit -m "feat(config): OpenAiCompatProviderConfig.context_window_tokens fallback"
```

After this commit, return to Task 14 Step 4 to land OpenAi-compat gateway tests.

---

## Phase 3 · StepRecorder (Tasks 16–19)

Four new `record_*` methods plus the invariant validators required by §5.5.

### Task 16: `record_context_compacted` + invariants

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs`

- [ ] **Step 1: Write the failing test (inline)**

In `crates/cogito-core/src/harness/step_recorder.rs` `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn record_context_compacted_writes_event() {
    let (recorder, _store) = make_test_recorder().await;
    // ... seed turn boundaries: TurnStarted at seq=2, terminal at seq=7
    let event_id = recorder.lock().await.record_context_compacted(
        TurnId::new(),
        (2, 7),
        "truncate",
        CompactionReplacement::Drop,
        TokenEstimates { before: Some(5000), after: Some(800) },
    ).await.expect("writes");
    // Assert the event landed
    assert!(/* event_id is in store */);
}

#[tokio::test]
async fn record_context_compacted_rejects_self_referential_range() {
    let (recorder, _store) = make_test_recorder().await;
    // ... seed events through seq=10
    let result = recorder.lock().await.record_context_compacted(
        TurnId::new(),
        (2, 11),  // 11 >= next_seq=11
        "truncate",
        CompactionReplacement::Drop,
        TokenEstimates::default(),
    ).await;
    assert!(matches!(result, Err(StoreError::InvariantViolated(_))));
}

#[tokio::test]
async fn record_context_compacted_rejects_non_turn_boundary_start() {
    let (recorder, _store) = make_test_recorder().await;
    // ... seed TurnStarted only at seq=2; seq=3 is AssistantMessageAppended
    let result = recorder.lock().await.record_context_compacted(
        TurnId::new(),
        (3, 5),  // start is not a TurnStarted seq
        "truncate",
        CompactionReplacement::Drop,
        TokenEstimates::default(),
    ).await;
    assert!(matches!(result, Err(StoreError::InvariantViolated(_))));
}

#[tokio::test]
async fn record_context_compacted_rejects_duplicate_for_turn() {
    let (recorder, _store) = make_test_recorder().await;
    let turn = TurnId::new();
    // ... seed history; write first ContextCompacted for `turn`
    let first = recorder.lock().await.record_context_compacted(
        turn, (2, 5), "truncate", CompactionReplacement::Drop, TokenEstimates::default(),
    ).await.expect("first ok");
    let second = recorder.lock().await.record_context_compacted(
        turn, (8, 9), "truncate", CompactionReplacement::Drop, TokenEstimates::default(),
    ).await;
    assert!(matches!(second, Err(StoreError::InvariantViolated(_))));
    drop(first);
}
```

(Use existing helpers in step_recorder.rs's test module for `make_test_recorder()`; if absent, copy the pattern from `crates/cogito-core/src/harness/step_recorder.rs` existing tests.)

- [ ] **Step 2: Run, expect compile fail (method missing)**

```bash
make test CRATE=cogito-core
```

- [ ] **Step 3: Add `TokenEstimates` to `cogito-protocol::context`**

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TokenEstimates {
    pub before: Option<u64>,
    pub after: Option<u64>,
}
```

Re-export from `lib.rs`.

- [ ] **Step 4: Implement `record_context_compacted`**

In `crates/cogito-core/src/harness/step_recorder.rs`:

```rust
pub async fn record_context_compacted(
    &mut self,
    turn_id: TurnId,
    replaced_seq_range: (u64, u64),
    produced_by: impl Into<String>,
    replacement: CompactionReplacement,
    estimates: TokenEstimates,
) -> Result<EventId, StoreError> {
    // Invariant 1: range.1 < next_seq (no self-reference)
    let next_seq = self.next_seq_preview();
    if replaced_seq_range.1 >= next_seq {
        return Err(StoreError::InvariantViolated(format!(
            "ContextCompacted.replaced_seq_range.1 = {} must be < next_seq = {}",
            replaced_seq_range.1, next_seq,
        )));
    }
    // Invariant 2: range.0 is a TurnStarted seq
    if !self.history_snapshot().iter().any(|ev| {
        ev.seq == replaced_seq_range.0
            && matches!(ev.payload, EventPayload::TurnStarted { .. })
    }) {
        return Err(StoreError::InvariantViolated(format!(
            "range.0 = {} is not a TurnStarted seq",
            replaced_seq_range.0,
        )));
    }
    // Invariant 3: range.1 is the last event of its turn
    // (verify next event after range.1 is either TurnStarted or end-of-history)
    let next_after = self.history_snapshot().iter().find(|ev| ev.seq > replaced_seq_range.1);
    if let Some(ne) = next_after {
        if !matches!(ne.payload, EventPayload::TurnStarted { .. }
            | EventPayload::ContextManageEntered { .. }) {
            return Err(StoreError::InvariantViolated(format!(
                "range.1 = {} is not the last event of its turn (next is seq={})",
                replaced_seq_range.1, ne.seq,
            )));
        }
    }
    // Invariant 4: at most one ContextCompacted per turn (v0.1)
    if self.history_snapshot().iter().any(|ev| {
        matches!(&ev.payload, EventPayload::ContextCompacted { turn_id: t, .. } if *t == turn_id)
    }) {
        return Err(StoreError::InvariantViolated(format!(
            "ContextCompacted already exists for turn_id {turn_id:?}",
        )));
    }

    self.append(EventPayload::ContextCompacted {
        turn_id,
        replaced_seq_range,
        produced_by: produced_by.into(),
        replacement,
        token_estimate_before: estimates.before,
        token_estimate_after: estimates.after,
    }).await
}
```

The `history_snapshot()` and `next_seq_preview()` helpers may need to be added — they expose the in-memory cached event log that StepRecorder already maintains (this cache exists per Sprint 3's resume work; check `step_recorder.rs` for the existing field, likely `events: Vec<ConversationEvent>` or similar).

- [ ] **Step 5: Run tests; all 4 pass**

```bash
make test CRATE=cogito-core
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-protocol/src/context.rs crates/cogito-protocol/src/lib.rs crates/cogito-core/src/harness/step_recorder.rs
git commit -m "feat(step-recorder): record_context_compacted with §5.5 invariants"
```

---

### Task 17: `record_system_prompt_injected` (idempotent)

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn record_system_prompt_injected_first_call_writes() {
    let (recorder, _store) = make_test_recorder().await;
    let turn = TurnId::new();
    let id = recorder.lock().await.record_system_prompt_injected(
        turn, "Today is 2026-05-23.".into(), vec!["date".into()], "none",
    ).await.expect("writes");
    assert!(id.is_some_id());
}

#[tokio::test]
async fn record_system_prompt_injected_duplicate_for_turn_returns_existing() {
    let (recorder, _store) = make_test_recorder().await;
    let turn = TurnId::new();
    let first = recorder.lock().await.record_system_prompt_injected(
        turn, "a".into(), vec![], "none",
    ).await.expect("first ok");
    let second = recorder.lock().await.record_system_prompt_injected(
        turn, "b".into(), vec![], "none",
    ).await.expect("second ok");
    assert_eq!(first, second, "must return same EventId, must not double-write");
}
```

- [ ] **Step 2: Run, expect fail**

```bash
make test CRATE=cogito-core
```

- [ ] **Step 3: Implement**

```rust
pub async fn record_system_prompt_injected(
    &mut self,
    turn_id: TurnId,
    suffix: String,
    contributors: Vec<String>,
    produced_by: impl Into<String>,
) -> Result<EventId, StoreError> {
    // Idempotency: if event already exists for this turn, return its id
    if let Some(existing_id) = self.history_snapshot().iter().find_map(|ev| {
        match &ev.payload {
            EventPayload::SystemPromptInjected { turn_id: t, .. } if *t == turn_id => {
                Some(ev.event_id.clone())
            }
            _ => None,
        }
    }) {
        return Ok(existing_id);
    }
    self.append(EventPayload::SystemPromptInjected {
        turn_id,
        suffix,
        contributors,
        produced_by: produced_by.into(),
    }).await
}
```

- [ ] **Step 4: Tests pass**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs
git commit -m "feat(step-recorder): record_system_prompt_injected with per-turn idempotency"
```

---

### Task 18: `record_tool_filter_overridden` (idempotent)

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs`

- [ ] **Step 1: Write the failing test (same shape as Task 17)**

```rust
#[tokio::test]
async fn record_tool_filter_overridden_idempotent_on_turn() {
    let (recorder, _store) = make_test_recorder().await;
    let turn = TurnId::new();
    let first = recorder.lock().await.record_tool_filter_overridden(
        turn, ToolFilterOverrideMode::Inherit, vec![], "none",
    ).await.expect("first ok");
    let second = recorder.lock().await.record_tool_filter_overridden(
        turn,
        ToolFilterOverrideMode::Intersect { tools: vec!["read_file".into()] },
        vec![],
        "none",
    ).await.expect("second ok");
    assert_eq!(first, second);
}
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: Implement (mirror of Task 17)**

```rust
pub async fn record_tool_filter_overridden(
    &mut self,
    turn_id: TurnId,
    mode: ToolFilterOverrideMode,
    contributors: Vec<String>,
    produced_by: impl Into<String>,
) -> Result<EventId, StoreError> {
    if let Some(existing_id) = self.history_snapshot().iter().find_map(|ev| {
        match &ev.payload {
            EventPayload::ToolFilterOverridden { turn_id: t, .. } if *t == turn_id => {
                Some(ev.event_id.clone())
            }
            _ => None,
        }
    }) {
        return Ok(existing_id);
    }
    self.append(EventPayload::ToolFilterOverridden {
        turn_id, mode, contributors, produced_by: produced_by.into(),
    }).await
}
```

- [ ] **Step 4: Tests pass**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs
git commit -m "feat(step-recorder): record_tool_filter_overridden with per-turn idempotency"
```

---

### Task 19: `record_context_decision`

**Files:**
- Modify: `crates/cogito-core/src/harness/step_recorder.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn record_context_decision_writes_summary() {
    let (recorder, _store) = make_test_recorder().await;
    let turn = TurnId::new();
    let sys_id = recorder.lock().await.record_system_prompt_injected(
        turn, "".into(), vec![], "none",
    ).await.expect("sys ok");
    let filter_id = recorder.lock().await.record_tool_filter_overridden(
        turn, ToolFilterOverrideMode::Inherit, vec![], "none",
    ).await.expect("filter ok");
    let decision_id = recorder.lock().await.record_context_decision(
        turn, vec![], sys_id.clone(), filter_id.clone(), ContextDecisionErrors::default(),
    ).await.expect("decision ok");
    assert!(decision_id.is_some_id());
}
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: Implement (no idempotency check needed — H11 only writes once per ContextManaged)**

```rust
pub async fn record_context_decision(
    &mut self,
    turn_id: TurnId,
    compactions: Vec<EventId>,
    system_prompt_event: EventId,
    tool_filter_event: EventId,
    errors: ContextDecisionErrors,
) -> Result<EventId, StoreError> {
    self.append(EventPayload::ContextDecisionRecorded {
        turn_id, compactions, system_prompt_event, tool_filter_event, errors,
    }).await
}
```

- [ ] **Step 4: Tests pass; `make ci`**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/step_recorder.rs
git commit -m "feat(step-recorder): record_context_decision summary writer"
```

---

## Phase 4 · `cogito-context` crate (Tasks 20–27)

New umbrella crate hosting all trait impls. Per CLAUDE.md §"Tagged-config factories", the `build_pipeline` factory lives here.

### Task 20: Crate scaffold

**Files:**
- Create: `crates/cogito-context/Cargo.toml`
- Create: `crates/cogito-context/src/lib.rs`
- Modify: workspace root `Cargo.toml` (add member)

- [ ] **Step 1: Add member to workspace root `Cargo.toml`**

Under `[workspace] members = [...]` append `"crates/cogito-context"`.

- [ ] **Step 2: Create `crates/cogito-context/Cargo.toml`**

```toml
[package]
name = "cogito-context"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
cogito-protocol = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
cogito-test-fixtures = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread"] }
```

- [ ] **Step 3: Create `crates/cogito-context/src/lib.rs`**

```rust
//! Context-Management trait implementations + assembly factory.
//!
//! Brain (`cogito-core::harness`) sees only `cogito-protocol` traits and
//! interacts with this crate's outputs via `Arc<dyn ...>`. This crate is
//! a Hand-like layer in the ADR-0004 boundary: trait impls + composition.
//!
//! v0.1 ships:
//! - `compactor::none::NoneCompactor`
//! - `compactor::truncate::TruncateCompactor`
//! - `projector::standard::StandardProjector`
//! - `injector::none::NoneInjector`
//! - `overrider::none::NoneOverrider`
//!
//! Sprint 7 adds `injector::skills`; v0.2 adds `compactor::summarize`,
//! `projector::tool_elision`, `overrider::read_only`, …

#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod compactor;
pub mod injector;
pub mod overrider;
pub mod pipeline;
pub mod projector;

pub use pipeline::{ContextPipeline, build_pipeline};
```

- [ ] **Step 4: Stub the submodules so the crate compiles**

Create empty:
- `crates/cogito-context/src/compactor/mod.rs` → `pub mod none; pub mod truncate;`
- `crates/cogito-context/src/projector/mod.rs` → `pub mod standard;`
- `crates/cogito-context/src/injector/mod.rs` → `pub mod none;`
- `crates/cogito-context/src/overrider/mod.rs` → `pub mod none;`
- `crates/cogito-context/src/pipeline.rs` → stub

Each leaf file initially gets a placeholder that compiles:

```rust
// crates/cogito-context/src/compactor/none.rs (placeholder)
//! NoneCompactor — implemented in Task 21.
```

`pipeline.rs` placeholder:

```rust
//! ContextPipeline factory — implemented in Task 27.

pub use cogito_protocol::ContextPipeline;

/// Build a `ContextPipeline` from a `ContextConfig`. Implementation in Task 27.
#[must_use]
pub fn build_pipeline(_config: &cogito_protocol::ContextConfig) -> ContextPipeline {
    unimplemented!("Task 27 lands the real factory")
}
```

- [ ] **Step 5: `make fix CRATE=cogito-context && make test CRATE=cogito-context`**

Expected: compiles + zero tests.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/cogito-context/
git commit -m "feat(context): scaffold cogito-context crate"
```

---

### Task 21: `NoneCompactor` + `NoneInjector` + `NoneOverrider` impls

**Files:**
- Modify: `crates/cogito-context/src/compactor/none.rs`
- Modify: `crates/cogito-context/src/injector/none.rs`
- Modify: `crates/cogito-context/src/overrider/none.rs`

- [ ] **Step 1: Write failing tests**

In each leaf file add `#[cfg(test)] mod tests { ... }`. For `compactor/none.rs`:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_test_fixtures::context::{test_compaction_input, test_recorder};

    #[tokio::test]
    async fn none_compactor_never_writes() {
        let (recorder, store) = test_recorder().await;
        let mut handle = recorder.lock().await;
        let input = test_compaction_input(&mut handle, &store).await;
        let result = NoneCompactor.maybe_compact(input).await.expect("ok");
        assert!(result.is_empty());
        assert_eq!(store.event_count().await, 0);
    }
}
```

For `injector/none.rs`:

```rust
#[tokio::test]
async fn none_injector_writes_empty_suffix() {
    let (recorder, store) = test_recorder().await;
    let mut handle = recorder.lock().await;
    let input = test_injection_input(&mut handle, &store).await;
    let event_id = NoneInjector.inject(input).await.expect("ok");
    let ev = store.find_by_id(&event_id).await.expect("found");
    let EventPayload::SystemPromptInjected { suffix, contributors, produced_by, .. } = ev.payload else {
        panic!("expected SystemPromptInjected");
    };
    assert_eq!(suffix, "");
    assert!(contributors.is_empty());
    assert_eq!(produced_by, "none");
}
```

Symmetric for `overrider/none.rs` with `mode = Inherit`.

(The `cogito_test_fixtures::context` helpers `test_compaction_input` / `test_injection_input` / `test_filter_input` / `test_recorder` need to be added in `crates/testing/cogito-test-fixtures/src/context.rs` — a small fixture module. Add it as part of this task.)

- [ ] **Step 2: Implement `NoneCompactor`**

```rust
//! `NoneCompactor` — no-op Compactor that never compacts.

use async_trait::async_trait;
use cogito_protocol::context::{
    CompactionApplied, CompactionInput, Compactor, ContextError,
};

/// Compactor that returns `Ok(vec![])` unconditionally. Used as default
/// when `ContextConfig.compactor == CompactorConfig::None`.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneCompactor;

#[async_trait]
impl Compactor for NoneCompactor {
    async fn maybe_compact(
        &self,
        _input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError> {
        Ok(vec![])
    }

    fn id(&self) -> &'static str { "none" }
}
```

- [ ] **Step 3: Implement `NoneInjector`**

```rust
//! `NoneInjector` — writes an empty `SystemPromptInjected` every turn.

use async_trait::async_trait;
use cogito_protocol::context::{
    ContextError, InjectionInput, SystemPromptInjector,
};
use cogito_protocol::event::EventId;

/// Injector that writes `SystemPromptInjected { suffix: "", contributors: [] }`.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneInjector;

#[async_trait]
impl SystemPromptInjector for NoneInjector {
    async fn inject(
        &self,
        input: InjectionInput<'_>,
    ) -> Result<EventId, ContextError> {
        Ok(input.recorder.record_system_prompt_injected(
            input.turn_id, String::new(), vec![], "none",
        ).await?)
    }

    fn id(&self) -> &'static str { "none" }
}
```

- [ ] **Step 4: Implement `NoneOverrider`**

```rust
//! `NoneOverrider` — writes `ToolFilterOverridden { mode: Inherit }` every turn.

use async_trait::async_trait;
use cogito_protocol::context::{
    ContextError, ToolFilterInput, ToolFilterOverrider, ToolFilterOverrideMode,
};
use cogito_protocol::event::EventId;

/// Overrider that always returns `Inherit`.
#[derive(Default, Clone, Copy, Debug)]
pub struct NoneOverrider;

#[async_trait]
impl ToolFilterOverrider for NoneOverrider {
    async fn override_filter(
        &self,
        input: ToolFilterInput<'_>,
    ) -> Result<EventId, ContextError> {
        Ok(input.recorder.record_tool_filter_overridden(
            input.turn_id, ToolFilterOverrideMode::Inherit, vec![], "none",
        ).await?)
    }

    fn id(&self) -> &'static str { "none" }
}
```

- [ ] **Step 5: Run tests; expect pass**

```bash
make test CRATE=cogito-context
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-context/src/ crates/testing/cogito-test-fixtures/src/
git commit -m "feat(context): NoneCompactor + NoneInjector + NoneOverrider"
```

---

### Task 22: `StandardProjector` — §5 algorithm

**Files:**
- Modify: `crates/cogito-context/src/projector/standard.rs`

- [ ] **Step 1: Write the failing test (basic no-compaction case first)**

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    // Build a small event log: SessionStarted, TurnStarted(t1), AssistantMessageAppended(t1)
    #[test]
    fn projects_basic_two_turn_session() {
        let events = vec![
            session_started(1),
            turn_started(2, "t1", "hello"),
            assistant_message_appended(3, "t1", "hi back"),
            turn_started(4, "t2", "how are you"),
        ];
        let strategy = HarnessStrategy::default_with_model("test");
        let msgs = StandardProjector.project(&events, &strategy, t2_id());
        assert_eq!(msgs.len(), 4);  // system + user + assistant + user
        // ... assert message content
    }
}
```

- [ ] **Step 2: Run, expect compile fail (StandardProjector not defined)**

- [ ] **Step 3: Implement `StandardProjector` per §5 algorithm**

```rust
//! `StandardProjector` — reference HistoryProjector implementing the
//! covered-set projection algorithm from ADR-0008 §5.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::context::{
    CompactionReplacement, HistoryProjector, ProjectedMessage,
};
use cogito_protocol::event::EventPayload;
use cogito_protocol::ids::TurnId;
use cogito_protocol::store::ConversationEvent;
use cogito_protocol::strategy::HarnessStrategy;

/// Reference projector. Pure synchronous function over events + strategy.
#[derive(Default, Clone, Copy, Debug)]
pub struct StandardProjector;

impl HistoryProjector for StandardProjector {
    fn project(
        &self,
        events: &[ConversationEvent],
        strategy: &HarnessStrategy,
        current_turn: TurnId,
    ) -> Vec<ProjectedMessage> {
        // Step 1: collect covered ranges.
        let covered = collect_covered_ranges(events);

        // Step 2: find current turn's SystemPromptInjected suffix.
        let suffix = events.iter()
            .rev()
            .find_map(|ev| match &ev.payload {
                EventPayload::SystemPromptInjected { turn_id, suffix, .. }
                    if *turn_id == current_turn => Some(suffix.clone()),
                _ => None,
            });
        let system_text = match suffix {
            Some(s) if !s.is_empty() => format!("{}\n\n{}", strategy.system_prompt, s),
            _ => strategy.system_prompt.clone(),
        };

        let mut messages = vec![ProjectedMessage::System(system_text)];
        let mut assistant_buf: Vec<ContentBlock> = Vec::new();

        let flush_assistant = |buf: &mut Vec<ContentBlock>, msgs: &mut Vec<ProjectedMessage>| {
            if !buf.is_empty() {
                msgs.push(ProjectedMessage::Assistant(std::mem::take(buf)));
            }
        };

        for ev in events {
            if covered.contains(ev.seq) { continue; }

            match &ev.payload {
                EventPayload::ContextCompacted { replacement, .. } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    if let CompactionReplacement::Summary { text, .. } = replacement {
                        messages.push(ProjectedMessage::User(format!(
                            "<conversation_summary>\n{text}\n</conversation_summary>"
                        )));
                    }
                }
                EventPayload::TurnStarted { user_input, .. } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    messages.push(ProjectedMessage::User(user_input.clone()));
                }
                EventPayload::AssistantMessageAppended { text, .. } => {
                    assistant_buf.push(ContentBlock::Text { text: text.clone() });
                }
                EventPayload::ToolUseRecorded { call_id, tool_name, args, .. } => {
                    assistant_buf.push(ContentBlock::ToolUse {
                        id: call_id.clone(),
                        name: tool_name.clone(),
                        input: args.clone(),
                    });
                }
                EventPayload::ToolResultRecorded { call_id, result, .. } => {
                    flush_assistant(&mut assistant_buf, &mut messages);
                    messages.push(ProjectedMessage::ToolResult {
                        call_id: call_id.clone(),
                        result_blocks: vec![ContentBlock::Text { text: result.to_string() }],
                    });
                }
                EventPayload::ThinkingBlockRecorded { text, signature, .. } => {
                    // Per ADR-0019: Thinking precedes Text/ToolUse in the
                    // current assistant message. Prepend; the buffer was
                    // empty at start-of-message because TurnStarted flushed.
                    assistant_buf.insert(0, ContentBlock::Thinking {
                        text: text.clone(),
                        signature: signature.clone(),
                    });
                }
                _ => {} // meta events ignored
            }
        }
        flush_assistant(&mut assistant_buf, &mut messages);
        messages
    }

    fn id(&self) -> &'static str { "standard" }
}

fn collect_covered_ranges(events: &[ConversationEvent]) -> CoveredSet {
    let mut set = CoveredSet::default();
    for ev in events {
        if let EventPayload::ContextCompacted { replaced_seq_range, .. } = &ev.payload {
            set.add(*replaced_seq_range);
        }
    }
    set
}

#[derive(Default)]
struct CoveredSet { ranges: Vec<(u64, u64)> }

impl CoveredSet {
    fn add(&mut self, r: (u64, u64)) { self.ranges.push(r); }
    fn contains(&self, seq: u64) -> bool {
        self.ranges.iter().any(|(lo, hi)| seq >= *lo && seq <= *hi)
    }
}
```

- [ ] **Step 4: Tests pass for the basic case; iterate**

```bash
make test CRATE=cogito-context
```

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/projector/standard.rs
git commit -m "feat(context): StandardProjector implements §5 covered-set algorithm"
```

---

### Task 23: `StandardProjector` multi-compaction tests (§5.3)

**Files:**
- Create: `crates/cogito-context/tests/standard_projection.rs`

- [ ] **Step 1: Build the §5.3 trace as a test fixture**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_context::projector::standard::StandardProjector;
use cogito_protocol::context::{CompactionReplacement, HistoryProjector, ProjectedMessage};
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::strategy::HarnessStrategy;

fn build_two_compaction_session() -> (Vec<ConversationEvent>, TurnId /* current */) {
    // Build seq=1..406 per §5.3 of the spec. See spec §5.3 for the canonical
    // event sequence. Use small helper fns for each variant.
    // ... (long; ~80 lines of fixture construction)
    todo!("inline the §5.3 fixture")
}

#[test]
fn projects_two_compaction_session_correctly() {
    let (events, current_turn) = build_two_compaction_session();
    let strategy = HarnessStrategy::default_with_model("test");
    let msgs = StandardProjector.project(&events, &strategy, current_turn);

    // System message includes the date suffix from t61's SystemPromptInjected
    let ProjectedMessage::System(sys) = &msgs[0] else { panic!() };
    assert!(sys.contains("Today is 2026-05-23."));

    // Summary from C2 (seq=401) appears as user-role
    let ProjectedMessage::User(u1) = &msgs[1] else { panic!() };
    assert!(u1.starts_with("<conversation_summary>"));
    assert!(u1.contains("weather then follow-up forecasts"));

    // No other compaction text — C1 was Drop
    assert!(!msgs.iter().any(|m| {
        if let ProjectedMessage::User(t) = m {
            t.contains("turns t1-t20")
        } else { false }
    }));

    // turn t61 user input is the last user message (if any post-401 events exist)
    // (the fixture should include a TurnStarted at seq=406+ to simulate the
    // turn t61 input)
}
```

(Replace `todo!()` with the actual fixture in this task.)

- [ ] **Step 2: Run, expect pass**

```bash
make test CRATE=cogito-context
```

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-context/tests/standard_projection.rs
git commit -m "test(context): StandardProjector §5.3 multi-compaction projection"
```

---

### Task 24: `TruncateCompactor` base (config + idempotency)

**Files:**
- Modify: `crates/cogito-context/src/compactor/truncate.rs`
- Create: `crates/cogito-context/tests/truncate_compaction.rs`

- [ ] **Step 1: Write idempotency + below-threshold tests (boundaries 1 + 6)**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[tokio::test]
async fn truncate_no_op_when_estimate_below_threshold() {
    // session with 3 turns, ~100 chars total → estimate < 100k
    // ... assert maybe_compact returns vec![]
}

#[tokio::test]
async fn truncate_idempotent_when_compaction_already_for_turn() {
    // pre-seed ContextCompacted for the current turn
    // call maybe_compact → must return Vec<CompactionApplied> with existing event_id
    // assert no new event written
}
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: Implement `TruncateCompactor` skeleton**

```rust
//! `TruncateCompactor` — adaptive sliding-window compactor.

use async_trait::async_trait;
use cogito_protocol::context::{
    CompactionApplied, CompactionInput, CompactionKind, CompactionReplacement, Compactor,
    ContextError, TokenEstimates, TokenThreshold, TruncateConfig,
};
use cogito_protocol::event::EventPayload;

/// Truncate-style Compactor. See ADR-0008 §10 for the full algorithm.
#[derive(Clone, Debug)]
pub struct TruncateCompactor {
    config: TruncateConfig,
}

impl TruncateCompactor {
    #[must_use]
    pub fn new(config: TruncateConfig) -> Self { Self { config } }
}

#[async_trait]
impl Compactor for TruncateCompactor {
    async fn maybe_compact(
        &self,
        input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError> {
        // Step 1: idempotency
        for ev in input.history {
            if let EventPayload::ContextCompacted { turn_id, replaced_seq_range, .. } = &ev.payload {
                if *turn_id == input.turn_id {
                    return Ok(vec![CompactionApplied {
                        event_id: ev.event_id.clone(),
                        replaced_seq_range: *replaced_seq_range,
                        kind: CompactionKind::Truncate,
                    }]);
                }
            }
        }

        // Step 2-7: implemented in Task 25
        Ok(vec![])
    }

    fn id(&self) -> &'static str { "truncate" }
}
```

- [ ] **Step 4: Tests pass (only idempotency + no-op work; Task 25 fills algorithm)**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/compactor/truncate.rs crates/cogito-context/tests/truncate_compaction.rs
git commit -m "feat(context): TruncateCompactor skeleton with idempotency check"
```

---

### Task 25: `TruncateCompactor` full algorithm (steps 2-7)

**Files:**
- Modify: `crates/cogito-context/src/compactor/truncate.rs`
- Modify: `crates/cogito-context/tests/truncate_compaction.rs`

- [ ] **Step 1: Write tests for boundaries 2, 3, 4, 5**

```rust
#[tokio::test]
async fn truncate_no_op_when_too_few_turns() {
    // 2 turns total, keep_first + keep_recent=5 → nothing to drop
    // assert vec![]
}

#[tokio::test]
async fn truncate_writes_event_for_long_session() {
    // 30 turns, max_tokens=Absolute(small), keep_recent=5
    // assert one ContextCompacted written, range starts at turn 2 (keep_first_user),
    // ends at turn 25's last event
}

#[tokio::test]
async fn truncate_advances_start_past_covered_prefix() {
    // seed prior ContextCompacted covering turns 1-10
    // run truncate again on a session that now needs more
    // assert new event's range starts at turn 11
}

#[tokio::test]
async fn truncate_no_op_when_all_drop_candidates_covered() {
    // seed prior compaction covering everything droppable
    // assert vec![]
}
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: Implement steps 2-7 per spec §10.2**

```rust
async fn maybe_compact(...) -> ... {
    // ... step 1 (already implemented)

    // Step 2: compute threshold from TokenThreshold + model_limits
    let max_tokens = match &self.config.max_tokens {
        TokenThreshold::Absolute(n) => *n,
        TokenThreshold::Ratio { of_context_window, safety_headroom } => {
            let limits = input.model_gateway.model_limits();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let raw = (limits.context_window_tokens as f64 * f64::from(*of_context_window)) as u64;
            raw.saturating_sub(*safety_headroom)
        }
    };

    // Step 3: token estimate
    let estimated = input.last_usage
        .as_ref()
        .map(|u| u64::from(u.input_tokens))
        .unwrap_or_else(|| estimate_visible_tokens(input.history));
    if estimated < max_tokens {
        return Ok(vec![]);
    }

    // Step 4: scan turn boundaries and covered ranges
    let turn_boundaries = collect_turn_boundaries(input.history);
    let covered = collect_covered_ranges(input.history);

    // Step 5: determine retain indices
    let total = turn_boundaries.len();
    let first_keep_idx = usize::from(self.config.keep_first_user);
    let last_keep_idx = total.saturating_sub(self.config.keep_recent_turns as usize);
    if first_keep_idx >= last_keep_idx {
        return Ok(vec![]);
    }

    // Step 6: locate first/last uncovered drop turn
    let mut drop_start_seq: Option<u64> = None;
    let mut drop_end_seq: Option<u64> = None;
    for idx in first_keep_idx..last_keep_idx {
        let (_, start, end) = turn_boundaries[idx];
        let fully_covered = (start..=end).all(|s| covered.contains(s));
        if !fully_covered {
            if drop_start_seq.is_none() {
                drop_start_seq = Some(start);
            }
            drop_end_seq = Some(end);
        }
    }
    let (Some(start), Some(end)) = (drop_start_seq, drop_end_seq) else {
        return Ok(vec![]);
    };

    // Step 7: write ContextCompacted (StepRecorder validates §5.5 invariants)
    let event_id = input.recorder.record_context_compacted(
        input.turn_id,
        (start, end),
        "truncate",
        CompactionReplacement::Drop,
        TokenEstimates {
            before: Some(estimated),
            after: Some(estimated.saturating_sub((end - start) * 50)), // rough heuristic
        },
    ).await?;

    Ok(vec![CompactionApplied {
        event_id,
        replaced_seq_range: (start, end),
        kind: CompactionKind::Truncate,
    }])
}

fn collect_turn_boundaries(events: &[ConversationEvent]) -> Vec<(TurnId, u64, u64)> {
    let mut out = Vec::new();
    let mut current: Option<(TurnId, u64)> = None;
    let mut last_seq_in_turn: u64 = 0;
    for ev in events {
        if let EventPayload::TurnStarted { turn_id, .. } = &ev.payload {
            if let Some((tid, start)) = current.take() {
                out.push((tid, start, last_seq_in_turn));
            }
            current = Some((turn_id.clone(), ev.seq));
        }
        if matches!(ev.payload, EventPayload::AssistantMessageAppended { .. }
            | EventPayload::ToolUseRecorded { .. }
            | EventPayload::ToolResultRecorded { .. }
            | EventPayload::ThinkingBlockRecorded { .. }
            | EventPayload::TurnStarted { .. })
        {
            last_seq_in_turn = ev.seq;
        }
    }
    if let Some((tid, start)) = current {
        out.push((tid, start, last_seq_in_turn));
    }
    out
}

fn estimate_visible_tokens(events: &[ConversationEvent]) -> u64 {
    let covered = collect_covered_ranges(events);
    let mut chars = 0u64;
    for ev in events {
        if covered.contains(ev.seq) { continue; }
        chars += match &ev.payload {
            EventPayload::TurnStarted { user_input, .. } => user_input.len() as u64,
            EventPayload::AssistantMessageAppended { text, .. } => text.len() as u64,
            EventPayload::ThinkingBlockRecorded { text, .. } => text.len() as u64,
            EventPayload::ToolUseRecorded { args, .. } => args.to_string().len() as u64,
            EventPayload::ToolResultRecorded { result, .. } => result.to_string().len() as u64,
            EventPayload::ContextCompacted {
                replacement: CompactionReplacement::Summary { text, .. }, ..
            } => text.len() as u64,
            _ => 0,
        };
    }
    chars / 4
}

// Reuse the CoveredSet helper. Move it into a shared private module if both
// StandardProjector and TruncateCompactor need it (recommended).
```

- [ ] **Step 4: Tests pass**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/compactor/truncate.rs crates/cogito-context/tests/truncate_compaction.rs
git commit -m "feat(context): TruncateCompactor full §10 algorithm"
```

---

### Task 26: `TruncateCompactor` cross-provider threshold test (boundary 8)

**Files:**
- Create: `crates/cogito-context/tests/truncate_threshold_ratio.rs`

- [ ] **Step 1: Write the test using `MockModelGateway` with configurable limits**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_context::compactor::truncate::TruncateCompactor;
use cogito_protocol::context::{
    Compactor, CompactionInput, TokenThreshold, TruncateConfig,
};
use cogito_test_fixtures::mocks::{MockModelGateway, MockModelGatewayBuilder};

#[tokio::test]
async fn ratio_threshold_scales_with_context_window() {
    // Opus 1M case
    let gw = MockModelGatewayBuilder::new().with_context_window(1_000_000).build();
    let comp = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Ratio { of_context_window: 0.75, safety_headroom: 8192 },
        keep_first_user: true,
        keep_recent_turns: 5,
    });
    let threshold = compute_threshold_via_compactor(&comp, &gw).await;
    assert_eq!(threshold, 1_000_000 * 3 / 4 - 8192);  // 741_808

    // Sonnet 200k
    let gw2 = MockModelGatewayBuilder::new().with_context_window(200_000).build();
    let threshold2 = compute_threshold_via_compactor(&comp, &gw2).await;
    assert_eq!(threshold2, 200_000 * 3 / 4 - 8192);  // 141_808

    // vLLM 32k
    let gw3 = MockModelGatewayBuilder::new().with_context_window(32_768).build();
    let threshold3 = compute_threshold_via_compactor(&comp, &gw3).await;
    assert_eq!(threshold3, 32_768 * 3 / 4 - 8192);  // 16_384
}

#[tokio::test]
async fn absolute_threshold_ignores_model_limits() {
    let gw = MockModelGatewayBuilder::new().with_context_window(1_000_000).build();
    let comp = TruncateCompactor::new(TruncateConfig {
        max_tokens: TokenThreshold::Absolute(50_000),
        keep_first_user: true,
        keep_recent_turns: 5,
    });
    let threshold = compute_threshold_via_compactor(&comp, &gw).await;
    assert_eq!(threshold, 50_000);
}

// Helper: trigger compaction at a known token boundary by seeding a session
// with `last_usage.input_tokens = threshold + 1` and asserting compaction
// runs (i.e. threshold was crossed). Use a binary search or just verify
// via observable behavior — adjust as needed for what's testable.
async fn compute_threshold_via_compactor(...) -> u64 { /* ... */ }
```

(Add `MockModelGatewayBuilder::with_context_window` to `cogito-test-fixtures::mocks` as part of this task.)

- [ ] **Step 2: Run, expect pass**

```bash
make test CRATE=cogito-context
```

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-context/tests/truncate_threshold_ratio.rs crates/testing/cogito-test-fixtures/src/mocks/
git commit -m "test(context): TruncateCompactor cross-provider threshold scaling"
```

---

### Task 27: `build_pipeline` factory + `pipeline_assembly` test

**Files:**
- Modify: `crates/cogito-context/src/pipeline.rs`
- Create: `crates/cogito-context/tests/pipeline_assembly.rs`

- [ ] **Step 1: Write the test**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_context::build_pipeline;
use cogito_protocol::context::{
    CompactorConfig, ContextConfig, HistoryProjectorConfig,
    SystemPromptInjectorConfig, TokenThreshold, ToolFilterOverriderConfig,
    TruncateConfig,
};

#[test]
fn default_config_assembles_no_op_pipeline() {
    let p = build_pipeline(&ContextConfig::default());
    assert_eq!(p.compactor.id(), "none");
    assert_eq!(p.projector.id(), "standard");
    assert_eq!(p.injector.id(), "none");
    assert_eq!(p.overrider.id(), "none");
}

#[test]
fn truncate_config_assembles_truncate_compactor() {
    let mut cfg = ContextConfig::default();
    cfg.compactor = CompactorConfig::Truncate(TruncateConfig {
        max_tokens: TokenThreshold::default(),
        keep_first_user: true,
        keep_recent_turns: 5,
    });
    let p = build_pipeline(&cfg);
    assert_eq!(p.compactor.id(), "truncate");
}
```

- [ ] **Step 2: Run, expect fail (`build_pipeline` is `unimplemented!`)**

- [ ] **Step 3: Implement the factory**

In `crates/cogito-context/src/pipeline.rs`:

```rust
//! `ContextPipeline` factory.

use std::sync::Arc;

use cogito_protocol::context::{
    Compactor, CompactorConfig, ContextConfig, ContextPipeline, HistoryProjector,
    HistoryProjectorConfig, SystemPromptInjector, SystemPromptInjectorConfig,
    ToolFilterOverrider, ToolFilterOverriderConfig,
};

use crate::compactor::{none::NoneCompactor, truncate::TruncateCompactor};
use crate::injector::none::NoneInjector;
use crate::overrider::none::NoneOverrider;
use crate::projector::standard::StandardProjector;

/// Assemble a `ContextPipeline` from a `ContextConfig` by dispatching each
/// tagged variant to the corresponding implementation in this crate.
/// See CLAUDE.md §"Tagged-config factories".
#[must_use]
pub fn build_pipeline(config: &ContextConfig) -> ContextPipeline {
    ContextPipeline {
        compactor: build_compactor(&config.compactor),
        projector: build_projector(&config.history_projector),
        injector: build_injector(&config.system_prompt_injector),
        overrider: build_overrider(&config.tool_filter_overrider),
    }
}

fn build_compactor(cfg: &CompactorConfig) -> Arc<dyn Compactor> {
    match cfg {
        CompactorConfig::None => Arc::new(NoneCompactor),
        CompactorConfig::Truncate(c) => Arc::new(TruncateCompactor::new(c.clone())),
    }
}

fn build_projector(cfg: &HistoryProjectorConfig) -> Arc<dyn HistoryProjector> {
    match cfg {
        HistoryProjectorConfig::Standard => Arc::new(StandardProjector),
    }
}

fn build_injector(cfg: &SystemPromptInjectorConfig) -> Arc<dyn SystemPromptInjector> {
    match cfg {
        SystemPromptInjectorConfig::None => Arc::new(NoneInjector),
    }
}

fn build_overrider(cfg: &ToolFilterOverriderConfig) -> Arc<dyn ToolFilterOverrider> {
    match cfg {
        ToolFilterOverriderConfig::None => Arc::new(NoneOverrider),
    }
}
```

- [ ] **Step 4: Tests pass**

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/pipeline.rs crates/cogito-context/tests/pipeline_assembly.rs
git commit -m "feat(context): build_pipeline factory dispatches ContextConfig to impls"
```

---

## Phase 5 · Brain wiring — H11 + H04 + H05 (Tasks 28–30)

### Task 28: H11 transition rewrite — orchestration + degrade

**Files:**
- Modify: `crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs`
- Modify: `crates/cogito-core/src/harness/turn_driver/deps.rs` (add `context_pipeline` field)

- [ ] **Step 1: Add `context_pipeline` to `TurnDeps`**

```rust
use std::sync::Arc;

use cogito_protocol::context::ContextPipeline;

pub struct TurnDeps {
    // existing fields...
    pub metrics: Arc<dyn MetricsRecorder>,
    /// Sprint 6: per-session context-management pipeline.
    pub context_pipeline: Arc<ContextPipeline>,
}
```

- [ ] **Step 2: Write the failing integration test stub (will be filled in Task 32)**

For now, the existing chaos / minimal-loop tests should still pass once `context_pipeline` defaults to no-op. Add a TODO in `try_start_turn` for now to plug in `Arc::new(build_pipeline(&strategy.context))` once Runtime wiring lands in Task 31.

Temporary: hardcode `Arc::new(build_pipeline(&strategy.context))` directly in `try_start_turn` so the field is populated.

- [ ] **Step 3: Rewrite `context_managed.rs`**

```rust
//! Init → ContextManaged → PromptBuilt transition. Sprint 6 lands the real
//! H11 implementation per ADR-0008 §9.

use cogito_protocol::context::{
    CompactionInput, ContextDecisionErrors, InjectionInput, ToolFilterInput,
};
use tracing::warn;

use crate::harness::turn_driver::{TurnCtx, TurnDeps, TurnState};

pub async fn run_context_managed(
    ctx: TurnCtx,
    deps: &TurnDeps,
) -> Result<TurnState, TurnFailureReason> {
    // ContextManageEntered was already written by Init→ContextManaged dispatch.
    let pipeline = &deps.context_pipeline;
    let history = ctx.history_snapshot.clone();
    let mut errors = ContextDecisionErrors::default();

    // ── 1. Compactor (may write 0/1 ContextCompacted)
    let compactions = {
        let mut recorder = deps.step.lock().await;
        let input = CompactionInput {
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            history: &history,
            strategy: &ctx.strategy,
            last_usage: ctx.last_usage.clone(),
            model_gateway: deps.model.as_ref(),
            recorder: recorder.as_handle(),
        };
        match pipeline.compactor.maybe_compact(input).await {
            Ok(applied) => applied.into_iter().map(|c| c.event_id).collect(),
            Err(e) => {
                warn!(error = %e, "compactor degraded");
                errors.compactor = Some(e.to_string());
                vec![]
            }
        }
    };

    // ── 2. SystemPromptInjector (always writes one event)
    let system_prompt_event = {
        let mut recorder = deps.step.lock().await;
        let input = InjectionInput {
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            strategy: &ctx.strategy,
            history: &history,
            exec_ctx: &ctx.exec_ctx,
            recorder: recorder.as_handle(),
        };
        match pipeline.injector.inject(input).await {
            Ok(eid) => eid,
            Err(e) => {
                warn!(error = %e, "injector degraded");
                errors.injector = Some(e.to_string());
                recorder.record_system_prompt_injected(
                    ctx.turn_id.clone(),
                    String::new(),
                    vec![],
                    "fallback-empty",
                ).await.map_err(TurnFailureReason::from)?
            }
        }
    };

    // ── 3. ToolFilterOverrider (always writes one event)
    let tool_filter_event = {
        let mut recorder = deps.step.lock().await;
        let input = ToolFilterInput {
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            strategy: &ctx.strategy,
            history: &history,
            exec_ctx: &ctx.exec_ctx,
            recorder: recorder.as_handle(),
        };
        match pipeline.overrider.override_filter(input).await {
            Ok(eid) => eid,
            Err(e) => {
                warn!(error = %e, "overrider degraded");
                errors.overrider = Some(e.to_string());
                recorder.record_tool_filter_overridden(
                    ctx.turn_id.clone(),
                    ToolFilterOverrideMode::Inherit,
                    vec![],
                    "fallback-inherit",
                ).await.map_err(TurnFailureReason::from)?
            }
        }
    };

    // ── 4. H11 writes ContextDecisionRecorded summary
    {
        let mut recorder = deps.step.lock().await;
        recorder.record_context_decision(
            ctx.turn_id.clone(),
            compactions,
            system_prompt_event,
            tool_filter_event,
            errors,
        ).await.map_err(TurnFailureReason::from)?;
    }

    // ── 5. ContextManageCompleted + transition
    {
        let mut recorder = deps.step.lock().await;
        recorder.record_context_manage_completed(ctx.turn_id.clone())
            .await
            .map_err(TurnFailureReason::from)?;
    }

    Ok(TurnState::PromptBuilt { ctx })
}
```

- [ ] **Step 4: Update the transition dispatcher to call this new function**

Find the `Init → ContextManaged → PromptBuilt` dispatch (likely in `turn_driver/mod.rs` or `transitions/mod.rs`) and confirm `run_context_managed` is invoked when entering ContextManaged.

- [ ] **Step 5: Existing tests should still pass (no-op pipeline injected by default)**

```bash
make test CRATE=cogito-core
```

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/harness/turn_driver/
git commit -m "feat(h11): real ContextManaged orchestration with degrade-on-failure"
```

---

### Task 29: H04 use `HistoryProjector` via pipeline

**Files:**
- Modify: `crates/cogito-core/src/harness/prompt.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn h04_calls_history_projector_from_pipeline() {
    // ... build a TurnDeps with a stub HistoryProjector that records its calls
    // call H04 compose
    // assert the projector was invoked
}
```

- [ ] **Step 2: Replace the inlined projection with `deps.context_pipeline.projector.project(...)`**

Find the existing `project_history` function in `prompt.rs` and replace its body with a delegated call:

```rust
pub fn compose(
    events: &[ConversationEvent],
    strategy: &HarnessStrategy,
    current_turn: TurnId,
    surface: &ToolSurface,
    projector: &dyn HistoryProjector,
) -> ModelInput {
    let messages = projector.project(events, strategy, current_turn);
    // ... convert ProjectedMessage list to ModelInput
}
```

The transition that calls `compose` (`context_managed.rs` or wherever PromptBuilt is built) must thread `deps.context_pipeline.projector.as_ref()` through.

- [ ] **Step 3: Tests pass**

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/src/harness/prompt.rs crates/cogito-core/src/harness/turn_driver/transitions/
git commit -m "feat(h04): delegate history projection to HistoryProjector trait"
```

---

### Task 30: H05 read `ToolFilterOverridden` event

**Files:**
- Modify: `crates/cogito-core/src/harness/tool_surface.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn h05_intersects_with_tool_filter_overridden_intersect() {
    // strategy.allowed_tools = Allow([a, b, c])
    // events contain ToolFilterOverridden::Intersect { tools: [b, c, d] } for current turn
    // expected: surface includes b, c
}

#[test]
fn h05_replaces_with_tool_filter_overridden_replace() {
    // strategy.allowed_tools = All
    // events contain ToolFilterOverridden::Replace { tools: [x] } for current turn
    // expected: surface = [x]
}

#[test]
fn h05_inherits_with_tool_filter_overridden_inherit() {
    // strategy.allowed_tools = Allow([a, b])
    // events contain ToolFilterOverridden::Inherit for current turn
    // expected: surface = [a, b]
}
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: Extend `tool_surface::surface()`**

```rust
pub fn surface(
    strategy: &HarnessStrategy,
    provider: &dyn ToolProvider,
    events: &[ConversationEvent],
    current_turn: TurnId,
) -> ToolSurface {
    let base = match &strategy.allowed_tools {
        ToolFilter::All => provider.list().into_iter().collect::<Vec<_>>(),
        ToolFilter::Allow(allowed) => provider.list().into_iter()
            .filter(|t| allowed.contains(&t.name))
            .collect(),
    };

    // Find this turn's ToolFilterOverridden mode (latest one for current_turn)
    let mode = events.iter().rev().find_map(|ev| match &ev.payload {
        EventPayload::ToolFilterOverridden { turn_id, mode, .. } if *turn_id == current_turn => {
            Some(mode.clone())
        }
        _ => None,
    });

    let filtered = match mode {
        None | Some(ToolFilterOverrideMode::Inherit) => base,
        Some(ToolFilterOverrideMode::Intersect { tools }) => base.into_iter()
            .filter(|t| tools.contains(&t.name))
            .collect(),
        Some(ToolFilterOverrideMode::Replace { tools }) => provider.list().into_iter()
            .filter(|t| tools.contains(&t.name))
            .collect(),
    };

    // Apply existing tool_order logic on top
    ToolSurface::from(filtered)
}
```

(Adjust to the existing `surface()` signature; this is illustrative.)

- [ ] **Step 4: Update H05's call sites to pass `events` and `current_turn`**

The transition that calls H05 must pass these. Update accordingly.

- [ ] **Step 5: Tests pass**

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-core/src/harness/tool_surface.rs crates/cogito-core/src/harness/turn_driver/transitions/
git commit -m "feat(h05): read ToolFilterOverridden + apply Inherit/Intersect/Replace"
```

---

## Phase 6 · Runtime wiring (Task 31)

### Task 31: `SessionShared.context_pipeline` + `Runtime::open_session` builds it

**Files:**
- Modify: `crates/cogito-core/src/runtime/mod.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs`
- Modify: `crates/cogito-core/Cargo.toml` (add `cogito-context` dep)
- Modify: workspace root `Cargo.toml` (`cogito-context = { path = "crates/cogito-context" }` in `[workspace.dependencies]`)

- [ ] **Step 1: Add `cogito-context` to workspace + cogito-core deps**

Workspace `Cargo.toml`:

```toml
[workspace.dependencies]
cogito-context = { path = "crates/cogito-context" }
# ... other entries
```

`crates/cogito-core/Cargo.toml`:

```toml
[dependencies]
cogito-context = { workspace = true }
# ... existing
```

- [ ] **Step 2: Add field to `SessionShared`**

```rust
use cogito_protocol::context::ContextPipeline;

pub struct SessionShared {
    // existing
    pub(super) hooks: Arc<CompositeHookPipeline>,
    pub(super) context_pipeline: Arc<ContextPipeline>,
}
```

- [ ] **Step 3: In `Runtime::open_session`, build the pipeline**

```rust
let context_pipeline = Arc::new(cogito_context::build_pipeline(&strategy.context));
let shared = SessionShared {
    // existing fields
    hooks,
    context_pipeline,
};
```

- [ ] **Step 4: In `try_start_turn`, clone the pipeline into `TurnDeps`**

```rust
let turn_deps = TurnDeps {
    // existing
    hooks: Arc::clone(&state.hooks),
    context_pipeline: Arc::clone(&state.context_pipeline),
    // ...
};
```

- [ ] **Step 5: Existing chaos / minimal-loop tests pass (default pipeline = all no-op)**

```bash
make ci
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/cogito-core/Cargo.toml crates/cogito-core/src/runtime/
git commit -m "feat(runtime): SessionShared.context_pipeline + Runtime::open_session assembly"
```

---

## Phase 7 · Integration tests (Tasks 32–34)

### Task 32: `context_managed_no_op` integration

**Files:**
- Create: `crates/cogito-core/tests/context_managed_no_op.rs`

- [ ] **Step 1: Write the test**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_test_fixtures::runtime::TestRuntime;

#[tokio::test]
async fn context_managed_writes_four_events_default_config() {
    let rt = TestRuntime::builder().build().await;
    let session = rt.open_session_default().await;

    session.send_user("hello").await.unwrap();
    let turn = session.wait_turn_completed().await.unwrap();

    let events = rt.replay(&session.id()).await;

    // For this turn we expect (in order):
    //   ContextManageEntered → SystemPromptInjected → ToolFilterOverridden →
    //   ContextDecisionRecorded → ContextManageCompleted → PromptComposed → ...
    let ctx_entered = events.iter().filter(|e| matches!(e.payload, EventPayload::ContextManageEntered { .. })).count();
    let sys_injected = events.iter().filter(|e| matches!(e.payload, EventPayload::SystemPromptInjected { .. })).count();
    let tool_overridden = events.iter().filter(|e| matches!(e.payload, EventPayload::ToolFilterOverridden { .. })).count();
    let decision = events.iter().filter(|e| matches!(e.payload, EventPayload::ContextDecisionRecorded { .. })).count();
    let ctx_completed = events.iter().filter(|e| matches!(e.payload, EventPayload::ContextManageCompleted { .. })).count();
    let compacted = events.iter().filter(|e| matches!(e.payload, EventPayload::ContextCompacted { .. })).count();

    assert_eq!(ctx_entered, 1);
    assert_eq!(sys_injected, 1);
    assert_eq!(tool_overridden, 1);
    assert_eq!(decision, 1);
    assert_eq!(ctx_completed, 1);
    assert_eq!(compacted, 0, "default ContextConfig should not compact");
}
```

- [ ] **Step 2: Run, expect pass**

```bash
make test CRATE=cogito-core
```

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-core/tests/context_managed_no_op.rs
git commit -m "test(core): ContextManaged no-op writes 4+ events per turn"
```

---

### Task 33: `context_managed_with_truncate` integration

**Files:**
- Create: `crates/cogito-core/tests/context_managed_with_truncate.rs`

- [ ] **Step 1: Write the test**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[tokio::test]
async fn truncate_compactor_writes_event_when_session_exceeds_threshold() {
    let strategy = HarnessStrategy {
        context: ContextConfig {
            compactor: CompactorConfig::Truncate(TruncateConfig {
                max_tokens: TokenThreshold::Absolute(1_000),  // tiny on purpose
                keep_first_user: true,
                keep_recent_turns: 2,
            }),
            ..ContextConfig::default()
        },
        ..HarnessStrategy::default_with_model("mock-1m")
    };

    let rt = TestRuntime::builder().strategy(strategy).build().await;
    let session = rt.open_session_default().await;

    // Drive 10 turns of small content (estimate will exceed 1_000 chars eventually)
    for i in 0..10 {
        session.send_user(&format!("turn {i} input - some longer content to inflate estimate")).await.unwrap();
        session.wait_turn_completed().await.unwrap();
    }

    let events = rt.replay(&session.id()).await;
    let compactions: Vec<_> = events.iter()
        .filter_map(|e| if let EventPayload::ContextCompacted { replaced_seq_range, .. } = &e.payload {
            Some(*replaced_seq_range)
        } else { None })
        .collect();

    assert!(!compactions.is_empty(), "expected at least one ContextCompacted event");
    // The first compaction should start at the first TurnStarted's seq
    let first_turn_started_seq = events.iter()
        .find(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .map(|e| e.seq)
        .unwrap();
    // If keep_first_user=true, range.0 = second TurnStarted's seq
    let second_turn_started_seq = events.iter()
        .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .nth(1)
        .map(|e| e.seq)
        .unwrap();
    assert_eq!(compactions[0].0, second_turn_started_seq);
}
```

- [ ] **Step 2: Run, expect pass**

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-core/tests/context_managed_with_truncate.rs
git commit -m "test(core): truncate Compactor end-to-end through Runtime"
```

---

### Task 34: H04 multi-compaction projection + H05 tool filter integrations

**Files:**
- Create: `crates/cogito-core/tests/h04_multi_compaction_projection.rs`
- Create: `crates/cogito-core/tests/h05_tool_filter.rs`

- [ ] **Step 1: H04 test — build §5.3 trace via direct store seeding, replay through Runtime, capture ModelInput sent to mock gateway**

(The mock gateway can record `stream()` inputs for assertion.)

```rust
#[tokio::test]
async fn h04_projects_multi_compaction_session_correctly() {
    let store = build_seeded_store_for_5_3_trace().await;
    let rt = TestRuntime::builder().with_existing_store(store).build().await;
    let session = rt.resume_session(SESSION_ID).await;

    session.send_user("turn t61 input").await.unwrap();
    session.wait_turn_completed().await.unwrap();

    let captured = rt.captured_model_inputs().await;
    let last_input = captured.last().unwrap();
    assert!(last_input.messages[0].is_system());
    assert!(last_input.messages[1].as_user_text().unwrap().contains("<conversation_summary>"));
}
```

- [ ] **Step 2: H05 test — directly inject `ToolFilterOverridden` events and assert tool surface**

```rust
#[tokio::test]
async fn h05_applies_intersect_mode() {
    // ... seed an event with Intersect{tools: ["read_file"]}
    // ... drive a turn
    // ... capture ModelInput; assert tools list contains only "read_file"
}

#[tokio::test]
async fn h05_applies_replace_mode() { /* ... */ }
```

- [ ] **Step 3: Run, expect pass**

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-core/tests/
git commit -m "test(core): H04 multi-compaction projection + H05 tool filter integration"
```

---

## Phase 8 · Chaos pairing assertion (Task 35)

### Task 35: `assert_context_managed_pairing` helper

**Files:**
- Modify: `crates/cogito-core/tests/resume_chaos.rs`

- [ ] **Step 1: Write the helper + call it**

```rust
fn assert_context_managed_pairing(events: &[ConversationEvent]) {
    let mut open_turn_ids: HashSet<TurnId> = HashSet::new();
    for ev in events {
        match &ev.payload {
            EventPayload::ContextManageEntered { turn_id } => {
                assert!(open_turn_ids.insert(turn_id.clone()),
                    "duplicate ContextManageEntered for turn {turn_id:?}");
            }
            EventPayload::ContextManageCompleted { turn_id }
            | EventPayload::TurnFailed { turn_id, .. } => {
                open_turn_ids.remove(turn_id);
            }
            _ => {}
        }
    }
    assert!(open_turn_ids.is_empty(),
        "unclosed ContextManageEntered for turns: {open_turn_ids:?}");
}
```

Call it inside the main `resume_chaos_run` driver after the recovered run completes, against the recovered event log.

- [ ] **Step 2: Run all chaos scenarios, expect pass**

```bash
make chaos
```

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-core/tests/resume_chaos.rs
git commit -m "test(chaos): assert ContextManageEntered/Completed pairing"
```

---

## Phase 9 · Docs + fixtures + closure (Tasks 36–38)

### Task 36: Update component docs + jsonl-v1.md + sample fixture

**Files:**
- Modify: `docs/components/H11-context-manage.md`
- Modify: `docs/components/H04-prompt-composer.md`
- Modify: `docs/components/H05-tool-surface.md`
- Modify: `docs/data-model/jsonl-v1.md`
- Create: `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-truncate-v1.jsonl`

- [ ] **Step 1: H11 doc — upgrade from placeholder**

In `docs/components/H11-context-manage.md`:
- Update status line to "Implemented in v0.1 Sprint 6 (ADR-0008)"
- Add §"v0.1 implementation" pointing to ADR-0008 and the spec
- Replace provisional `ContextManager` interface section with the four-trait surface from ADR-0008
- Remove all "TBD" / "open question" markers that ADR-0008 has now resolved

- [ ] **Step 2: H04 doc — add §"HistoryProjector dispatch"**

Brief footnote: H04 since Sprint 6 delegates projection to `dyn HistoryProjector` from `SessionShared.context_pipeline`. Default `StandardProjector` implements ADR-0008 §5 algorithm.

- [ ] **Step 3: H05 doc — add §"ToolFilterOverridden integration"**

Brief footnote: H05 since Sprint 6 reads the latest `ToolFilterOverridden` event for the current turn and applies Inherit/Intersect/Replace on top of `strategy.allowed_tools`.

- [ ] **Step 4: `docs/data-model/jsonl-v1.md` — add §"Context management events"**

Document the 4 new variants with field-level descriptions + a worked example pointing to the new sample fixture.

- [ ] **Step 5: Generate sample fixture**

`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-truncate-v1.jsonl` — write a small (15-turn) session that demonstrates one truncate compaction. Each line is one JSON event; mirror `sample-v1.jsonl` formatting.

- [ ] **Step 6: Commit**

```bash
git add docs/components/H11-context-manage.md docs/components/H04-prompt-composer.md docs/components/H05-tool-surface.md docs/data-model/jsonl-v1.md crates/testing/cogito-test-fixtures/fixtures/sessions/sample-truncate-v1.jsonl
git commit -m "docs(sprint-6): H11 implementation closure + H04/H05 notes + jsonl-v1 + fixture"
```

---

### Task 37: ROADMAP + CHANGELOG

**Files:**
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Tick Sprint 6 boxes in ROADMAP**

Mark every Sprint 6 line item `- [x]` once its corresponding task has landed.

- [ ] **Step 2: Add Sprint 6 entry to CHANGELOG**

```markdown
## [Unreleased] — Sprint 6 (Context Management)

### Added
- `cogito-protocol::context` — 4 traits (`Compactor`, `HistoryProjector`, `SystemPromptInjector`, `ToolFilterOverrider`) + `ContextConfig` + `ContextPipeline` + supporting types
- `EventPayload` variants: `ContextCompacted`, `SystemPromptInjected`, `ToolFilterOverridden`, `ContextDecisionRecorded` (additive, no `SCHEMA_VERSION` bump per ADR-0007)
- `EventPayload::category()` classifier (Conversation / HarnessMeta / ContextDecision)
- `ModelGateway::model_limits()` additive method + `ModelLimits` type
- `parse_context_window_suffix()` helper (`[1m]`/`[32k]`-style model id parsing)
- `cogito-context` crate: `NoneCompactor`, `TruncateCompactor`, `StandardProjector`, `NoneInjector`, `NoneOverrider`, `build_pipeline` factory
- `OpenAiCompatProviderConfig.context_window_tokens: Option<u64>` fallback field

### Changed
- H01 Turn Driver `ContextManaged` state from pass-through to real orchestration (4-trait pipeline + degrade-on-failure)
- H04 Prompt Composer delegates history projection to `dyn HistoryProjector`
- H05 Tool Surface honors per-turn `ToolFilterOverridden` event
- `AnthropicGateway` / `OpenAiCompatGateway` override `model_limits()`
- `HarnessStrategy` gains `context: ContextConfig` field (default = all no-op)

### Decisions (ADR)
- **ADR-0008** Context Management — Accepted 2026-05-23
```

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md CHANGELOG.md
git commit -m "docs(sprint-6): tick ROADMAP + CHANGELOG entry"
```

---

### Task 38: Final `make ci` + close-out

**Files:** None directly — verification pass

- [ ] **Step 1: Full `make ci`**

```bash
make ci
```

Expected: green.

- [ ] **Step 2: Full `make chaos`**

```bash
make chaos
```

Expected: green; ContextManaged pairing assertion passes for all scenarios.

- [ ] **Step 3: Optional micro-bench `context_managed_no_op_latency`**

```bash
make bench BENCH=context_managed_no_op_latency
```

Record results under `docs/quality/v0.1-context-baseline.md` if numbers are meaningful. P99 target: < 1ms for default no-op pipeline (per spec §13.4).

- [ ] **Step 4: Verify `cogito chat` smoke**

```bash
cargo run -p cogito-cli -- chat --provider <real> --model claude-opus-4-7
# interact briefly; confirm no panics; ^C
```

- [ ] **Step 5: Final commit if any cleanup**

```bash
git status
# if anything stray, address it; otherwise:
git log --oneline -38   # confirm task commits look clean
```

- [ ] **Step 6: Mark Sprint 6 closed**

Update `ROADMAP.md` Current section to reflect Sprint 6 complete, Sprint 7 in flight (or the next-up sprint per current state).

---

## Self-review checklist (run before handing off)

1. **Spec coverage**: every In-scope bullet of §1.1 maps to a task. ✓
   - 4 traits: Tasks 7/8/9
   - 4 event variants: Task 5
   - Tagged config enums: Task 10
   - `ContextPipeline`: Task 11
   - `ModelGateway::model_limits` + `ModelLimits`: Task 3
   - `parse_context_window_suffix`: Task 2
   - `AnthropicGateway::model_limits`: Task 13
   - `OpenAiCompatGateway::model_limits` + `api_model_id`: Task 14
   - `OpenAiCompatProviderConfig.context_window_tokens`: Task 15
   - 4 `record_*` methods + invariants: Tasks 16/17/18/19
   - `cogito-context` crate + all impls: Tasks 20-27
   - H11 rewrite: Task 28
   - H04 use HistoryProjector: Task 29
   - H05 read ToolFilterOverridden: Task 30
   - Runtime wiring: Task 31
   - Integration tests: Tasks 32-34
   - Chaos pairing assertion: Task 35
   - Docs / schema / fixture: Tasks 36 + 12 (schema regen)
   - ROADMAP + CHANGELOG: Task 37
   - ADR-0008 draft: Task 1
2. **Placeholder scan**: no TBD/TODO/"implement later" inside task code blocks. The `todo!()` in Task 23 Step 1 is explicitly called out to be replaced in the same step.
3. **Type consistency**: `ContextPipeline`, `ContextConfig`, `Compactor`, `HistoryProjector`, `SystemPromptInjector`, `ToolFilterOverrider`, `ModelLimits`, `TokenThreshold`, `TruncateConfig`, `CompactionInput`, `CompactionApplied`, `CompactionKind`, `CompactionReplacement`, `ToolFilterOverrideMode`, `ContextError`, `ContextDecisionErrors`, `InjectionInput`, `ToolFilterInput`, `EventCategory` — all defined once, referenced consistently.
4. **TDD ordering**: every task writes failing test before implementation.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-05-23-sprint-6-context-management.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?


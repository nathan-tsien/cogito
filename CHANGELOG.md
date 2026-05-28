# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Sprint 9a · Multi-model Strategy (2026-05-27)

**Added**
- `cogito-protocol::StrategyRegistry` trait (read-only, object-safe).
- `cogito-strategy` crate — FS-backed `StrategyRegistry` impl.
  Markdown+frontmatter strategy files under `.cogito/strategies/`
  (Repo scope) and `~/.config/cogito/strategies/` (User scope).
- `cogito-model::openai_responses` adapter — OpenAI Responses API
  with native reasoning-item decoding (ADR-0019).
- `ProviderConfig::OpenAiResponses` variant.
- `cogito.toml` `runtime.default_strategy` key.
- `cogito chat --strategy <name>` and `--list-strategies` flags.
- `cogito_cli::chat::resolve_strategy` helper — single seam for
  combining strategy + CLI flags + `cogito.toml`.
- Example strategies: `.cogito/strategies/{coder,planner,reviewer}.md`.
- Resume-chaos `strategy_with_tool_filter` scenario.
- ADR-0026 (Strategy registry).

**Changed**
- `runtime.strategies_dir` in `cogito.toml` is now an optional Repo-
  scope override rather than a single canonical directory.

**Removed**
- `strategies/claude-opus.yaml` and `strategies/gpt-4.yaml` (stale
  schema; replaced by `.cogito/strategies/*.md`).

### Sprint 8 — Async Jobs

- **Added** `cogito-jobs::LocalJobManager` — in-memory async job manager; jobs run as `tokio::task`s with `on_complete` sink registration.
- **Added** `cogito-jobs::RunTestsTool` — `ExecutionClass::AlwaysAsync` tool that spawns `cargo nextest run`, kills on cancel/deadline (default 10 min), truncates output to 64 KiB.
- **Added** `cogito-jobs::SleepTool` (test fixture, behind `test-tools` feature).
- **Added** `EventPayload::JobSubmitted { call_id, job_id, tool_name }` — additive variant; no `SCHEMA_VERSION` bump.
- **Activated** H08 async dispatch path; turn pauses on `InvokeOutcome::Async(job_id)`, resumes on `JobCompletionEvent`.
- **Activated** `ResumePausedJob` and `ResumeAfterJobCompletion` paths in `apply_resume_point`; `ShutdownOutcome::JobManagerUnavailable` no longer returned.
- **Added** single-slot mid-pause user-input queue (latest-wins, warn on overwrite).
- **Changed** `SessionHandle::cancel_turn` mid-pause now calls `JobManager::cancel`.
- **Fixed** cancel-token-disconnect — `SessionShared` and `SessionState` now share `Arc<parking_lot::Mutex<CancellationToken>>`, so `cancel_turn` works on every turn (not just the first).
- **Removed** the H03 narrowing that inferred `call_id` from `ToolUseRecorded` near `TurnPaused`; now read directly from `JobSubmitted`.
- **Added** chaos scenario `paused_async_job` with three crash boundaries.
- **Added** `cogito-protocol::job::LocalJobSubmitter` — dyn-compatible submission trait (`submit_boxed(self: Arc<Self>, BoxFuture) -> JobId`) for async tools. Replaces concrete `Arc<LocalJobManager>` parameters in `RunTestsTool::new` and `SleepTool::new` (ADR-0025).
- **Changed** `cogito-tools` no longer depends on `cogito-jobs`. `BuiltinToolProvider::with_jobs` and the embedded `run_tests` special-case are removed; `cogito-cli` composes builtins + async tools via `CompositeToolProvider::new(.., NamingPolicy::Strict)`.
- **Added** ADR-0025 — Hands sub-layer boundary.

### Sprint 7 — Skill loader (ADR-0020)

- `cogito-skills` crate (Hands): SkillRegistry impl of SkillProvider; frontmatter parser; sigil regex + code-fence-aware scanner; Repo + User scope discovery.
- `cogito-protocol`: SkillProvider trait + SkillMetadata/Content/Source; SkillActivated event variant; TurnStarted.activate_skills field; TurnTrigger::SkillActivation variant; StreamEvent::SkillActivationRequested broadcast; SystemPromptInjectorConfig::Skill; EventRecorder.record_skill_activated default impl; cogito_protocol::sigil module promoted from cogito-skills for ADR-0004 layer compliance.
- `cogito-context`: SkillInjector impl of SystemPromptInjector; build_pipeline_v2 takes Option<Arc<dyn SkillProvider>>.
- `cogito-core`: H06 sigil side-channel; TurnDeps.skills field; RuntimeBuilder.skills(...); session_loop projects SkillActivation to TurnStarted.activate_skills.
- `cogito-cli`: `/skill <name> [text]` slash parser; `[skills]` cogito.toml section; SkillRegistry built at chat startup.
- Resume chaos: `text_then_skill_then_tool` scenario with crash injection at sigil/activation boundaries.
- Docs: `docs/skills/overview.md`; H06/H11 closure notes; ADR-0020 promoted to Accepted.
- Additive event-log changes (no SCHEMA_VERSION bump).

### Added — Sprint 6 (Context Management)

- `cogito-protocol::context` — 4 traits (`Compactor`, `HistoryProjector`, `SystemPromptInjector`, `ToolFilterOverrider`) + `ContextConfig` + `ContextPipeline` + supporting types
- `EventPayload` variants: `ContextCompacted`, `SystemPromptInjected`, `ToolFilterOverridden`, `ContextDecisionRecorded` (additive, no `SCHEMA_VERSION` bump per ADR-0007)
- `EventPayload::category()` classifier (Conversation / HarnessMeta / ContextDecision)
- `ModelGateway::model_limits()` additive method + `ModelLimits` type
- `parse_context_window_suffix()` helper (`[1m]`/`[32k]`-style model id suffix parsing)
- `cogito-context` crate: `NoneCompactor`, `TruncateCompactor`, `StandardProjector`, `NoneInjector`, `NoneOverrider`, `build_pipeline` factory
- `OpenAiCompatProviderConfig.context_window_tokens: Option<u64>` fallback field
- `StepRecorder::record_context_compacted` / `record_system_prompt_injected` / `record_tool_filter_overridden` / `record_context_decision` methods (StoreError invariant checking + per-turn idempotency)
- `EventRecorder` trait in `cogito-protocol::store` (write-side abstraction so trait impls can persist events without depending on cogito-core)
- `StoreError::InvariantViolated` variant

### Changed — Sprint 6 (Context Management)

- H01 Turn Driver `ContextManaged` state from pass-through to real orchestration (4-trait pipeline + degrade-on-failure)
- H04 Prompt Composer delegates history projection to `dyn HistoryProjector`
- H05 Tool Surface honors per-turn `ToolFilterOverridden` event
- `AnthropicGateway` / `OpenAiCompatGateway` override `model_limits()`; OpenAI-compat strips `[<size>]` suffix from `params.model` before sending wire request to vLLM/SGLang
- `HarnessStrategy` gains `context: ContextConfig` field (default = all no-op)

### Decisions (ADR) — Sprint 6

- **ADR-0008** Context Management — Accepted 2026-05-23

### Added — Sprint 5 (Hook Pipeline 实化)

- `cogito-protocol::hook` — `HookHandler` + `HookProvider` + `HookDecision` + `HookLifecyclePoint` traits/types
- `cogito-protocol::metrics` — `MetricsRecorder` trait + `NoOpMetricsRecorder` default
- `EventPayload::HookRejected` event variant (additive, no schema_version bump)
- `CompositeHookPipeline` in `cogito-core::harness::hooks` with panic catch + metrics
- Reference hooks: `SensitiveContentHook` (AWS/GitHub/OpenAI key regex), `BashAuditHook` (tool.bash.invocations counter)
- All 5 H09 lifecycle points now wired in the FSM (pre_prompt + pre_dispatch + post_model + post_turn + on_error)
- Hook panic isolation: panicked hook → `HookDecision::Reject`, never crashes session loop
- P99 latency baseline in `docs/quality/v0.1-hook-latency.md`

### Deferred — Sprint 5

- `HookDecision::Modify` (variant reserved via `#[non_exhaustive]`; revisit when consumer use case surfaces)
- `cogito.toml [[hooks]]` configuration section (Sprint 12 / Plugin work will provide the unified config path)

### Added — Sprint 4 (MCP sync tools)

- `cogito-mcp` crate: rmcp 1.5 client wrapper + `ToolProvider`
  adapter. Stdio and streamable-HTTP transports; bearer-token auth
  via env var. OAuth deferred to a follow-up ADR.
- `McpToolProvider`: aggregates tools across configured MCP servers
  using `mcp__<server>__<tool>` qualified naming (sanitize disallowed
  chars, 64-char SHA-1 truncation, dedupe with warn).
- `McpStartupFailure`: unified channel for per-server failures
  (ConfigParse / BearerEnvMissing / DuplicateName / StartupTimeout /
  TransportError / HandshakeFailed). `#[non_exhaustive]`.
- `cogito-config`: `[[mcp_servers]]` section with lenient per-entry
  TOML deserialization; bad entries become `McpStartupFailure::
  ConfigParse` without poisoning the rest of the TOML parse.
- `cogito-cli chat`: startup banner prints per-server status on
  stderr (e.g., `[mcp] OK <name> ready (N tools)` and `[mcp] FAIL
  <name> skipped: <reason>` — using `✓` and `✗` glyphs in the actual
  banner output).
- H05 Tool Surface: emits `mcp.tool_count`, `mcp.tool_desc_total_bytes`,
  `builtin.tool_count` tracing fields per turn.
- ADR-0018: MCP integration architectural contract — license posture,
  transport scope, **MCP failures non-fatal to Runtime** principle
  (compiler-enforced via `McpProviderBuildResult` return type),
  namespacing, result mapping, schema trust posture, layer placement.

### Added — Sprint 4.5 (config-file loading)

- `cogito-config` crate: value types, `ConfigLoader` trait,
  `EnvConfigLoader`, layered partial merge, `${VAR}` interpolation,
  `FileConfigLoader` (feature `file`).
- `cogito-model::ProviderConfig` (tagged-union over provider kinds)
  + `build_gateway` factory.
- `cogito chat`: `--config <path>`; legacy ENV bridge preserves
  Sprint 2 invocations.

### Changed

- `cogito chat --model` now optional (falls back to
  `runtime.default_model` in `cogito.toml`).

### Added — Post-Sprint 3 (ADR-0016 TurnTrigger)

- `cogito-protocol::turn_trigger::TurnTrigger` — single-variant
  `#[non_exhaustive]` enum (`UserText(String)` in v0.1). Locks the
  shape of the abstraction so v0.2 `UserContent`, v0.3+
  `SkillInvocation`, and v0.6 `HookFired` land additively per ADR-0007
  track B (no `schema_version` bump). Re-exported at `cogito_protocol::TurnTrigger`.
- `cogito-core::runtime::SessionHandle::submit(TurnTrigger)` —
  canonical entry point for any new trigger source. See ADR-0016 §2.

### Changed — Post-Sprint 3 (ADR-0016 TurnTrigger)

- `cogito-core::runtime::SessionCommand::Input(NewMessage)` renamed to
  `SessionCommand::Trigger(TurnTrigger)`. Internal rename; the
  `NewMessage` struct is deleted (was never exposed outside
  `cogito_core::runtime::types`).
- `cogito-core::runtime::SessionHandle::send_user` renamed to
  `submit_user_text` and reduced to a 1-line shim around
  `submit(TurnTrigger::UserText(text.into()))`. The old name read as
  "send TO user" (verb+object) while the actual semantics are "submit
  text FROM the user"; the noun-style name aligns with `submit` and
  removes the directional ambiguity.
- `cogito-core::runtime::mod` no longer re-exports `NewMessage`; it
  re-exports `TurnTrigger` instead.

### Added — Sprint 2

- `cogito-protocol::gateway` — `ModelGateway` trait + `ModelInput` / `ModelOutput` /
  `ModelEvent` (gateway-preaggregation, X mode per design spec §Q1) / `ModelError` /
  `Message` / `ModelParams` / `StopReason` / `Usage`.
- `cogito-protocol::strategy` — `HarnessStrategy` + `ToolFilter` + `default_with_model`
  factory (v0.1 Mid field set per spec §Q2).
- `cogito-protocol::exec_ctx::ExecCtx` carrying a `tokio_util::sync::CancellationToken`.
- `cogito-protocol::tool::ToolProvider` trait (the doc-comment promise from Sprint 0/1).
- New `EventPayload` variants: `ContextManageEntered`, `ContextManageCompleted`,
  `PromptComposed { model, surface_size }`, `ModelCallStarted { model }`. All additive
  under `#[non_exhaustive]`; envelope `turn_id` is the single source of truth.
- `cogito-model::anthropic::AnthropicGateway` — streaming Messages API adapter with
  per-block buffering (text + partial-JSON for tool_use); cancellation via `ExecCtx`.
- `cogito-model::openai_compat::OpenAiCompatGateway` — Chat Completions adapter for
  vLLM / SGLang / Azure OpenAI / private LLM gateways. Tool-call accumulation across
  deltas; configurable `auth_header` / `auth_scheme`. **Not** the Responses API
  (that lands in Sprint 5).
- `cogito-model::sse` — shared SSE helper (built on `eventsource-stream`) + replay
  helpers for both decoders.
- Recorded SSE fixtures for replay tests (Anthropic + OpenAI-Compat, text-only +
  tool-use scenarios) under `crates/testing/cogito-test-fixtures/fixtures/sse/`.
- `cogito-tools::BuiltinToolProvider` + `BuiltinTool` trait.
- `cogito-tools::builtins::read_file` — UTF-8 reader, 1 MiB cap, `AlwaysSync` class.
- `cogito-tools::CompositeToolProvider` with `NamingPolicy::Strict` and `Prefixed`.
- `cogito-mock-model::MockModelGateway` — scripted playback for integration tests.
- `cogito-core::harness::prompt::compose` — H04 pure history projection
  (`Message::User { Vec<ContentBlock> }` + `Message::Assistant { Vec<ContentBlock> }`).
- `cogito-core::harness::tool_surface::surface` — H05 filter + `tool_order`-aware sort.
- `cogito-core::harness::tool_resolver` — H07 `{resolve, ToolInvocation, ResolvedCall}`
  with `jsonschema` 0.18 Draft validation.
- `cogito-core::harness::hooks::HookPipeline` — H09 no-op insertion points.
- `cogito-core::harness::resume::{replay, ResumeDecision}` — H03 stub
  (always `FreshTurn`; real decision table lands Sprint 3).
- `cogito-core::harness::stream_demux::demux` — H06 consume `Stream<ModelEvent>` +
  drive `StepRecorder` text-block lifecycle + accumulate ordered `ModelOutput`.
- `cogito-core::harness::dispatcher::{dispatch, DispatchOutcome}` — H08 sync path
  with `catch_unwind`. Async path returns structured `ToolResult::Error` until
  Sprint 4 wires `JobManager`.
- `cogito-core::harness::turn_driver/` — H01 FSM module:
  `state.rs` (Hybrid `TurnCtx` + `TurnState`),
  `deps.rs` (`TurnDeps`),
  `transitions/{init, context_managed, prompt_built, model_calling, model_completed, tool_dispatching}.rs`,
  `mod.rs` (`run` + `enter_turn`).
- `cogito-core::runtime::session_loop::run_session` — Topology I (per spec §Q4) with a
  biased `tokio::select!` over turn-result mpsc / mailbox / job-completion. Turn
  result delivery via mpsc instead of `JoinHandle` polling (avoids two `&mut state`
  borrows in the select macro).
- `cogito-core::runtime::Runtime` + `RuntimeBuilder` — `store / model / tools /
  strategy` setters + `build()`.
- `cogito-core::runtime::SessionHandle::{send_user, cancel_turn, shutdown, subscribe}`.
- `cogito-cli chat` — interactive REPL against Anthropic OR vLLM/SGLang-style
  OpenAI-compatible endpoints. Provider auto-inferred from `--model` prefix; env
  vars `ANTHROPIC_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_API_KEY`.
- Integration tests: `harness_prompt` (3), `harness_tool_surface` (3),
  `harness_tool_resolver` (4), `harness_stream_demux` (2), `harness_dispatcher` (2),
  `turn_driver_text_only` (1), `turn_driver_tool_call` (1), `session_e2e` (1),
  `anthropic_replay` (2), `openai_compat_replay` (2). All green under `cargo test`.

### Changed — Sprint 2

- `cogito-core::runtime::types::SessionId` was a Sprint 0 stub (`SessionId(pub String)`);
  removed in favour of re-exporting `cogito_protocol::ids::SessionId` (the ULID
  newtype). All runtime code now uses the protocol's canonical SessionId.
- `cogito-core::runtime::store_writer` cleared to an empty stub — the Sprint 1
  `StepRecorder` already owns `Arc<dyn ConversationStore>` and writes directly.
  A dedicated batching store-writer subtask is tracked for Sprint 4+ if needed.
- Workspace dep: `eventsource-stream = "0.2"` and `bytes = "1.7"` added.
- `cogito-protocol::ModelEvent::{ToolUseStarted, ToolUseCompleted}` use `tool_name`
  (not `name`) for cross-component consistency with `ContentBlock::ToolUse.tool_name`.

### Notes — Sprint 2

- v0.1 keeps `ContextManaged` as a pass-through; real H11 work (ADR-0008) is a
  dedicated post-Sprint-2 spike, not a numbered sprint.
- H03 real resume, H08 async path, `JobManager` wiring, real hooks, and TUI all
  remain Sprint 3+.
- The OpenAI-Compat adapter targets private deployments (vLLM/SGLang/Azure OpenAI).
  The OpenAI Responses API stays Sprint 5.

### Added — Sprint 1

- `cogito-protocol::event::ConversationEvent` with `schema_version: u32` and
  9-variant `EventPayload`. Adjacent-tag flattened envelope. `SCHEMA_VERSION = 1`.
- `cogito-protocol::store::ConversationStore` trait (`append`, `flush`, `close`,
  `latest_seq`, `replay`) + `StoreError`.
- `cogito-protocol::ids::{EventId, SessionId, TurnId}` ULID newtypes.
- `cogito-protocol::content::ContentBlock` (Text / ToolUse / ToolResult).
- `cogito-protocol::session::SessionMeta`.
- `cogito-store-jsonl` dev/debug-grade backend (one file per session,
  userspace flush only).
- `cogito-core::harness::step_recorder::StepRecorder` with content_block-
  boundary text batching.
- `cogito-test-fixtures::store_contract::run_store_contract` shared
  contract test suite.
- `cogito-test-fixtures::fixtures::canonical_sample_session` + checked-in
  `sample-v1.jsonl` fixture covering all 9 event variants.
- `cogito-gen-schema` internal tool + `docs/schemas/conversation-event-v1.json`
  artifact + CI drift gate.
- ADR-0007 (Event log as cross-language storage contract).
- `AGENTS.md` §2 text-delta lifecycle rewrite; new §7 `ConversationStore`
  scope rule.
- JSONL v1 spec at `docs/data-model/jsonl-v1.md`.
- H02 component doc: "Text block lifecycle" section.
- `append_throughput` criterion benchmark + `docs/quality/v0.1-jsonl-baseline.md`
  informational baseline.

### Compatibility

- `ConversationEvent` schema_version = 1; stable for the 0.x line.
  Future breaking changes will bump the version and ship a migration tool
  per ADR-0005 §4 #2.
- `ConversationStore` trait shape is stable for v0.1. 0.x breaking changes
  permitted with `CHANGELOG.md` entry; v1.0 freezes per ADR-0005 §5.

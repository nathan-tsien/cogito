# Roadmap

> Version-driven plan toward 1.0 GA and SaaS-ready 0.4. See
> `ARCHITECTURE.md` §"Version evolution path" for the full picture and
> ADR-0005 for the quality gates each version must meet.

## Current

> **v0.1 · Foundation** — Sprints 0–3 complete; Sprint 4 (Async Jobs) next.

## Version plan

### v0.1 · Foundation

**Goal**: production-grade core skeleton that runs a minimal end-to-end
turn against Anthropic with one tool, with full event sourcing, FSM Turn
Driver, panic isolation, and chaos-tested resume.

#### Sprint 0 · Project skeleton (0.5 day)
- [x] AGENTS.md, ARCHITECTURE.md, ROADMAP.md, ADR-0001/0002/0003/0004 written
- [x] CLAUDE.md added; ADR-0004 (Brain/Hands/Session) ratified
- [x] ADR-0005 (production scope) ratified
- [x] ADR-0006 (Runtime + H01 execution model) ratified
- [x] Workspace topology fixed per ADR-0004: dropped `cogito-conversation`, added `cogito-store-jsonl`, stripped Hands/Boundary/Session deps from `cogito-core`
- [x] Protocol types landed: `ExecutionClass`, `StreamEvent`, `JobCompletionEvent`, `JobManager::on_complete`, `TurnOutcome`, `TurnFailureReason` (12+ serde-roundtrip tests passing)
- [x] Runtime module scaffolded (stubs): `Runtime`, `RuntimeBuilder`, `SessionHandle`, per-session loop task (`runtime::session_loop::run_session` + `SessionShared`), `store_writer`
- [x] CI runs `just ci` (fmt + clippy + layer-check + test) + cargo-deny job
- [x] Toolchain aligned to MSRV 1.85 (edition 2024 requirement)

#### Sprint 1 · H02 Step Recorder + JSONL store (1.5 day)
- [x] `cogito-protocol` defines `ConversationEvent` with `schema_version: u32` + `Vec<ContentBlock>` payload (Text + ToolUse + ToolResult variants)
- [x] `cogito-protocol` defines `ConversationStore` trait
- [x] `cogito-store-jsonl` implementation: per-session file, `flush` per event, append-only (durability scope: dev/debug — see ADR-0007)
- [x] Contract test infrastructure (shared test consumed by every backend crate)
- [x] `cogito-core::harness::step_recorder` writes events
- [x] Text-block batching: per content_block boundary (matches Codex / Claude Code; see ADR-0007 + H02 doc)
- [x] Benchmark: `append_throughput` against JSONL; baseline at `docs/quality/v0.1-jsonl-baseline.md` (informational only, ADR-0005 §3 footnote)
- [x] ADR-0007 ratified (event log as cross-language contract)
- [x] JSON Schema artifact at `docs/schemas/conversation-event-v1.json` + CI drift gate
- [x] Canonical fixture at `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
- [x] JSONL v1 human-readable spec at `docs/data-model/jsonl-v1.md`
- [x] `AGENTS.md` §2 + §7 inviolable rules amended

#### Sprint 2 · Minimal Loop (2 days)
- [x] `read_file` tool only (in `cogito-tools`)
- [x] Anthropic adapter in `cogito-model` with streaming (also brought forward: OpenAI Chat Completions for vLLM/SGLang private deployments)
- [x] H01 Turn Driver state machine wired up (Init → ContextManaged → PromptBuilt → ModelCalling → ModelCompleted → ToolDispatching → Completed/Paused/Failed)
- [x] H04 Prompt Composer (basic — system + history + tool schemas)
- [x] H05 Tool Surface Builder (strategy-filtered list)
- [x] H06 Stream Demultiplexer (Anthropic + OpenAI-Compat events → cogito events; gateway-preaggregation X mode)
- [x] H07 Tool Call Resolver (JSON Schema validation, structured errors)
- [x] H08 Tool Dispatcher (sync path; panic catch around invoke)
- [x] H09 Hook Pipeline (no-op insertion points; real hooks in Sprint 6)
- [x] H10 Strategy Selector (`HarnessStrategy::default_with_model` factory; YAML registry in Sprint 5)
- [x] `MockModelGateway` for integration tests
- [x] `runtime::session_loop::run_session` Topology I + `Runtime::open_session` + `SessionHandle::{send_user, cancel_turn, shutdown}`
- [x] CLI `cogito chat` works end-to-end against Anthropic OR vLLM/SGLang with `read_file`

#### Sprint 3 · Resume Coordinator (2 days)

- [x] `EventPayload::ModelCallCompleted { stop_reason, usage }` variant added; schema artifact regenerated; fixture updated (per spec §4 Q1)
- [x] H03 Resume Coordinator decision table fully implemented (`harness::resume::replay()` covers all 9 decision-table rows from spec §5)
- [x] `ResumeDecision` shape: `{ point: ResumePoint, last_event_seq: Option<u64> }`; 6 `ResumePoint` variants (per spec §4 Q2 (revised in Q2 follow-up))
- [x] `Runtime::open_session(SessionMode::Resume)` walks the full recovery path (read store → replay → seq init → apply_resume_point)
- [x] EventId threading complete: Sprint 2 `recorded_event_id: "unknown"` stub cleaned up; all `record_*` methods return `Result<EventId, StoreError>`
- [x] Chaos test (`crates/cogito-core/tests/resume_chaos.rs`) drives crashes through the wired resume paths (see narrowing notes below)
- [x] All 4 oracles (prefix immutable / terminal equivalent / tool mapping equivalent / final text equivalent) pass for the boundaries the test exercises
- [x] `MockJobManager` exists in `cogito-test-fixtures` (Sprint 4 wires it into the actor; v0.1 returns `ShutdownOutcome::JobManagerUnavailable` for paused-job ResumePoint variants)
- [x] Chaos test total CI time < 10s (verified: 0.13s debug, <0.05s release)

**Sprint 3 closure — v0.1 narrowings:**

The following items were intentionally scoped down for v0.1 with explicit TODO markers in code:

- **`RestartCurrentTurn` downgrades to `FreshTurn` + warn** in `apply_resume_point` (`session_loop.rs`). Full `RestartCurrentTurn` requires recovering `user_input` from the persisted log; deferred to post-Sprint-3.
- **`ResumePausedJob` and `ResumeAfterJobCompletion` return `ShutdownOutcome::JobManagerUnavailable`** in `apply_resume_point`. v0.1 has no `JobManager` injection into Runtime — Sprint 4 will wire that in and activate these paths.
- **Chaos test covers 2 scenarios (`no_tool_short_turn`, `single_tool_happy_path`) × wired resume boundaries.** The `paused_async_job` scenario is unrunnable in v0.1; `tool_returns_error` was deferred. The test uses `PanicAt` (X-path-like) for crash injection because `NotifyAt + clean shutdown` writes a terminal event (defeating the chaos invariant); both proven to work via the existing infrastructure.
- **Latent: cancel-token disconnect** between `SessionState` and `SessionShared`. `SessionHandle::cancel_turn()` fires the original token; the session loop's `spawn_turn_driver` mints a new token per turn. Tracked via `TODO(cancel-token-disconnect)` in `session_loop.rs`. Pre-existing; not exercised by chaos tests.

These narrowings preserve the Sprint 3 invariants on the wired paths and document the remaining work clearly for the next sprint.

#### Sprint 4 · Async Jobs (2 days)
- [ ] `cogito-jobs` implements `JobManager` (tokio task + JSONL job log persistence)
- [ ] `cogito-jobs` provides cross-process job state persistence (mirrors event log structure; required by Sprint 3 ResumePausedJob path — see Sprint 3 spec §5.6)
- [ ] H08 Tool Dispatcher async path (handles `InvokeOutcome::Async(JobId)`)
- [ ] One real long task tool (`run_tests` or similar)
- [ ] Loop pauses on async, resumes on completion
- [ ] Mid-pause user input handling: queued, processed after current turn

#### Sprint 4.5 · 配置文件 + base_url override (0.5–1 day)

- [x] `cogito-config` crate(value types + ConfigLoader trait + EnvConfigLoader + merge)
- [x] `cogito-config` feature `file` → FileConfigLoader (`cogito.toml`)
- [x] `cogito-model::ProviderConfig` + `build_gateway` 工厂
- [x] `cogito-cli` 重构 `chat.rs`:`--config` 参数 + 三层 merge
- [x] Legacy ENV bridge:`cogito.toml` 缺席时合成 `default` provider
- [x] 单元/集成测试覆盖 merge、插值、搜索路径、CLI 流程
- [x] 文档:ADR-0017 落地、H10 doc 注脚、ROADMAP 更新

Closes GitLab Issue #1 sub-needs 1 + 2. Sub-need 3 (OpenAI Responses
adapter) remains scheduled for Sprint 5.

#### Sprint 5 · Multi-model Strategy (1 day)
- [ ] OpenAI adapter in `cogito-model` (Responses API; ContentBlock serialization to OpenAI's flat top-level items)
- [ ] H10 Strategy Selector with YAML config loading from `strategies/*.yaml`
- [ ] CLI `--model` flag selects strategy
- [ ] Per-strategy `model_params`, `allowed_tools`, `system_prompt`

#### Sprint 6 · Hook Pipeline + TUI (2 days)
- [ ] H09 Hook Pipeline with purity rule enforcement (see `docs/components/H09-hook-pipeline.md`)
- [ ] Two example hooks (sensitive content, bash audit)
- [ ] `MetricsRecorder` trait in protocol + default no-op
- [ ] Basic TUI with ratatui (replicates `cogito chat`)
- [ ] Per-hook P99 latency budget verified

#### Sprint 7 · v0.1 hardening (1 day)
- [ ] All component design docs cross-referenced and current
- [ ] CHANGELOG.md initial entry
- [ ] Tag `v0.1.0`

### Spike · Context Management (post-Sprint 2; ADR-0008)

**Goal**: design and ratify how `H11 Context Manage` actually works. The
architectural slot is locked by PR #6 (ADR-0006 amendment 2026-05-19);
the mechanism is still open. This spike is a dedicated initiative, not
a numbered sprint, because the work cuts across compaction, system
prompt lifecycle, tool injection, and TurnDriver integration.

- [ ] Research: Codex (`run_inline_auto_compact_task`), Claude Code (`/compact` + auto), Manus, other SaaS agent platforms — trigger policies and persisted shape
- [ ] Spec draft under `docs/superpowers/specs/`: H11 trigger policy, summarization model selection, replacement semantics, cascading compactions
- [ ] **ADR-0008**: Context Management — locks `ContextManager` trait, event variants (`ContextCompacted`, `ContextDecisionRecorded`, `SystemPromptInjected`, `ToolFilterOverridden`), trigger policy, summarization model selection
- [ ] `cogito-protocol`: additive `EventPayload` variants for context lifecycle (per `#[non_exhaustive]`, no schema_version bump)
- [ ] `cogito-core::harness`: H11 implementation; H01 `Init → ContextManaged` transition stops being a pass-through
- [ ] H04 history projection: honor `ContextCompacted` events
- [ ] Optional: `pre_context` / `post_context` hook lifecycle points if needed
- [ ] H03 Resume Coordinator: crash-mid-compaction recovery
- [ ] Chaos test: inject crash during summarization model call
- [ ] No version tag — feature lands into whichever v0.x is current when ready

### v0.2 · Storage + Multimodal

**Goal**: introduce `StorageSystem` as the third protocol pillar; lay the
foundation for multimodal content (`ContentBlock::Image`); ship one
multimedia tool to validate the full path.

- [ ] **ADR-0009**: `StorageSystem` trait + URI scheme + `ContentBlock::Image` variant _(renumbered from ADR-0007 by PR #6 — ADR-0007 now reserved for "Event log as cross-language contract", ADR-0008 for "Context Management")_
- [ ] **ADR-0010**: multimedia tool conventions (mime types, `outputs_model_visible_multimodal` flag, etc.) _(renumbered from ADR-0008)_
- [ ] `cogito-protocol`: add `StorageSystem` trait + `ContentBlock::Image`
- [ ] `cogito-storage-local` crate: `file://` + `http(s)://` (with local cache) + `blob://` (mapped to local dir)
- [ ] `ExecCtx.storage: Arc<dyn StorageSystem>` field
- [ ] `cogito-tools-multimedia` crate: `transcribe_audio` tool (uses Whisper API or local model — TBD)
- [ ] `cogito-mcp` crate: MCP client as `ToolProvider`
- [ ] TUI surface (if not landed in v0.1)
- [ ] Default secret redactor implementation
- [ ] Tag `v0.2.0`

### v0.3 · Subagent

**Goal**: support recursive Brain instances (subagent pattern) for
complex multi-role tasks.

- [ ] **ADR-0011**: Subagent execution model _(renumbered from ADR-0009)_
- [ ] `cogito-protocol`: add `BrainSpawner` trait + `SubagentSpawned` / `SubagentInputSent` / `SubagentCompleted` event variants
- [ ] `cogito-protocol`: extend `HarnessStrategy` with `spawnable_as_subagent`, `max_subagent_depth`
- [ ] `cogito-protocol`: extend session metadata with `parent_session_id`, `depth`, `role`
- [ ] `cogito-core::runtime`: implement `BrainSpawner` (recursive task hosting)
- [ ] `cogito-subagent` crate: `SubagentToolProvider` with 4 tools (`spawn_agent`, `wait_agent`, `send_input`, `cancel_agent`)
- [ ] Crash recovery: parent crash → child continues; child crash → parent gets `AsyncFailed`
- [ ] Depth limit enforcement
- [ ] Example strategies: `planner.yaml`, `coder.yaml`, `critic.yaml`
- [ ] Tag `v0.3.0`

### v0.4 · SaaS-ready

**Goal**: enable multi-replica deployment behind a consumer's gateway.

- [ ] **ADR-0012**: Sandbox lifecycle (lazy provisioning, pets-vs-cattle) _(renumbered from ADR-0010)_
- [ ] **ADR-0013**: Credential isolation (sandbox proxy pattern) _(renumbered from ADR-0011)_
- [ ] **ADR-0014**: TenantContext propagation _(renumbered from ADR-0012)_
- [ ] `cogito-store-postgres` crate: production multi-replica backend
- [ ] `cogito-storage-s3` crate: object storage backend
- [ ] `cogito-protocol`: add `TenantContext` optional field on `ExecCtx`
- [ ] `cogito-protocol`: add `MetricsRecorder` trait
- [ ] `cogito-observability-otel` crate: OpenTelemetry adapter (traces + metrics)
- [ ] Per-session resource budget enforcement (memory cap, CPU time cap)
- [ ] `cogito-sandbox` redesign: lazy provisioning + credential proxy
- [ ] Tag `v0.4.0`

### v0.5 · Multimedia breadth

**Goal**: full multimedia tool catalog; `model_visible` content path lit
end-to-end.

- [ ] `cogito-tools-multimedia` expansion:
  - [ ] `extract_frames(video_uri) -> Vec<image_uri>`
  - [ ] `summarize_video(video_uri) -> text`
  - [ ] `describe_image(image_uri) -> text`
  - [ ] `analyze_frame(image_uri, prompt) -> structured`
  - [ ] `synthesize_speech(text) -> audio_uri`
- [ ] `ContentBlock::Image` wired through `ModelGateway` adapters (Anthropic native; OpenAI image_url)
- [ ] `outputs_model_visible_multimodal` flag honored by H05 (filters tools incompatible with selected model)
- [ ] Tag `v0.5.0`

### v0.6 · Hardening

**Goal**: production-grade depth — load testing, migration tooling,
storage HTTP backend.

- [ ] Hook policy maturity: per-strategy hook config, hook composition rules
- [ ] Load test scaffolding: 1000 concurrent sessions per process target
- [ ] Soak test: 24h continuous run with no leaks / no degradation
- [ ] Event log migration tooling (v(N-1) → vN converter, with `replay_equivalence` test)
- [ ] **ADR-0015**: Storage HTTP wire protocol _(renumbered from ADR-0013 by PR #6)_
- [ ] `cogito-storage-http` crate: HTTP backend adapter
- [ ] Tag `v0.6.0`

### v1.0 · API freeze

**Goal**: public API stability commitment; first GA release.

- [ ] Public API audit: every exported symbol classified `stable` / `experimental` / `deprecated`
- [ ] Event log forward-compat switches to strict mode (every future version must read every past version)
- [ ] `#[non_exhaustive]` applied to every public enum
- [ ] Sealed marker traits for non-extensible traits
- [ ] CHANGELOG.md complete
- [ ] CHANGELOG migration guides for each breaking 0.x → 1.0 change
- [ ] Publish to crates.io
- [ ] Tag `v1.0.0`

## Future (v1.x+)

Captured in ARCHITECTURE.md §"Version evolution path":

- Resource Registry (P4) — long-lived OS resources (running processes, attached workspaces)
- Cross-brain hand sharing — multi-agent topology where brains pass tool handles to each other
- Real-time video stream processing — moving beyond batch URI to streaming
- Generative video — heavy GPU integration, displaced-content management
- MCP resources / prompts / sampling — expanding MCP support beyond tools

## What we explicitly do not do

These are out of cogito's scope regardless of version. The consumer
provides them (or a future SaaS layer wraps cogito to deliver them):

- Web UI / mobile clients
- Multi-tenant isolation (cogito provides `TenantContext` propagation in v0.4; enforcement is the consumer's)
- End-user authentication
- Quota / billing / metering ledger (cogito provides `MetricsRecorder` hooks; the ledger is the consumer's)
- Deployment artifacts (Docker / Helm / IaC)
- RAG / vector store (a Hand the consumer brings; not cogito core)
- Cross-session persistent memory (separate ADR if/when scoped)

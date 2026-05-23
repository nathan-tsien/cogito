# Roadmap

> Version-driven plan toward 1.0 GA and SaaS-ready 0.4. See
> `ARCHITECTURE.md` §"Version evolution path" for the full picture and
> ADR-0005 for the quality gates each version must meet.

## Current

> **v0.1 · Foundation** — Sprints 0–3 + 4.5 + 4.7 + 5 + 6 + 7 complete; Sprint 4
> (MCP sync tools) in flight; Sprints 8–10 reshaped per
> [2026-05-22 roadmap rebalance](docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
> (Hook impl + Context C2 trait freeze + Skill loader promoted into
> v0.1; Async Jobs / Multi-model / TUI / hardening renumbered;
> Storage+Multimodal deferred from v0.2 to v0.5).
> **Current sprint: Sprint 8 (Async Jobs).**

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
- [x] CI runs `make ci` (fmt + clippy + layer-check + test) + cargo-deny job
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
- [x] H09 Hook Pipeline (no-op insertion points; real hooks in Sprint 7)
- [x] H10 Strategy Selector (`HarnessStrategy::default_with_model` factory; YAML registry in Sprint 6)
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
- [x] `MockJobManager` exists in `cogito-test-fixtures` (Sprint 5 wires it into the actor; v0.1 returns `ShutdownOutcome::JobManagerUnavailable` for paused-job ResumePoint variants)
- [x] Chaos test total CI time < 10s (verified: 0.13s debug, <0.05s release)

**Sprint 3 closure — v0.1 narrowings:**

The following items were intentionally scoped down for v0.1 with explicit TODO markers in code:

- **`RestartCurrentTurn` downgrades to `FreshTurn` + warn** in `apply_resume_point` (`session_loop.rs`). Full `RestartCurrentTurn` requires recovering `user_input` from the persisted log; deferred to post-Sprint-3.
- **`ResumePausedJob` and `ResumeAfterJobCompletion` return `ShutdownOutcome::JobManagerUnavailable`** in `apply_resume_point`. v0.1 has no `JobManager` injection into Runtime — Sprint 5 will wire that in and activate these paths.
- **Chaos test covers 2 scenarios (`no_tool_short_turn`, `single_tool_happy_path`) × wired resume boundaries.** The `paused_async_job` scenario is unrunnable in v0.1; `tool_returns_error` was deferred. The test uses `PanicAt` (X-path-like) for crash injection because `NotifyAt + clean shutdown` writes a terminal event (defeating the chaos invariant); both proven to work via the existing infrastructure.
- **Latent: cancel-token disconnect** between `SessionState` and `SessionShared`. `SessionHandle::cancel_turn()` fires the original token; the session loop's `spawn_turn_driver` mints a new token per turn. Tracked via `TODO(cancel-token-disconnect)` in `session_loop.rs`. Pre-existing; not exercised by chaos tests.

These narrowings preserve the Sprint 3 invariants on the wired paths and document the remaining work clearly for the next sprint.

#### Sprint 4 · MCP sync tools (1.5–2 days)

**Goal**: pull `cogito-mcp` forward from v0.2 to give Brain a real catalog
of sync tools beyond the single built-in `read_file`, unblocking parallel
testing of Brain's tool-loop, prompt composition, and strategy selection.
Architecture-inspired by Codex's `rmcp-client` (Apache-2.0) — pattern-only
reimplementation, no source-code lift; `rmcp` itself is a normal upstream
dep (Apache-2.0, `modelcontextprotocol/rust-sdk`).

- [ ] `cogito-mcp` crate: thin wrapper over `rmcp` 1.5 with `transport-child-process` (stdio) + `transport-streamable-http-client-reqwest` (streamable-HTTP); bearer-token via env-var. OAuth flow deferred to a follow-up ADR.
- [ ] `McpToolProvider`: `ToolProvider` impl aggregating tools across configured servers via `mcp__<server>__<tool>` qualified naming (sanitize disallowed chars to `_`, 64-char cap with SHA-1 truncation suffix, dedupe with warn).
- [ ] `cogito-config`: `mcp_servers` section with `Stdio` / `StreamableHttp` transports + per-server `enabled_tools` / `disabled_tools` + startup/tool timeouts.
- [ ] `cogito-cli chat`: wire `McpToolProvider` into the `Runtime` builder via the existing `--config` path; tool surface visible to Brain.
- [ ] Integration test: exercise `tools/list` + `tools/call` end-to-end through `cogito chat` against a real streamable-HTTP MCP server with bearer auth.
- [ ] **ADR-0018**: MCP integration — transport scope, namespacing convention, deferred OAuth, license note (`rmcp` upstream + Codex pattern attribution).

#### Sprint 4.5 · 配置文件 + base_url override (0.5–1 day)

- [x] `cogito-config` crate(value types + ConfigLoader trait + EnvConfigLoader + merge)
- [x] `cogito-config` feature `file` → FileConfigLoader (`cogito.toml`)
- [x] `cogito-model::ProviderConfig` + `build_gateway` 工厂
- [x] `cogito-cli` 重构 `chat.rs`:`--config` 参数 + 三层 merge
- [x] Legacy ENV bridge:`cogito.toml` 缺席时合成 `default` provider
- [x] 单元/集成测试覆盖 merge、插值、搜索路径、CLI 流程
- [x] 文档:ADR-0017 落地、H10 doc 注脚、ROADMAP 更新

Closes GitLab Issue #1 sub-needs 1 + 2. Sub-need 3 (OpenAI Responses
adapter) remains scheduled for Sprint 6.

#### Sprint 4.7 · Thinking content (ADR-0019) (1.5 day)

**Goal**: first-class representation of model reasoning across protocol,
Brain, and provider adapters without bumping `SCHEMA_VERSION` and without
rewriting persisted JSONL. Closes the gap before Anthropic extended
thinking, OpenAI Responses reasoning items, or OpenAI-compat
`<think>`-tag models are exposed to users.

- [x] `cogito-protocol`: `ContentBlock::Thinking` + `EventPayload::ThinkingBlockRecorded` + `ModelEvent::ThinkingDelta` / `ThinkingBlockCompleted` + `StreamEvent::ThinkingDelta` (all additive, no `SCHEMA_VERSION` bump)
- [x] `cogito-core::harness`: H02 thinking-block buffer + flush; H06 routing; H04 projection
- [x] `cogito-model::anthropic`: decode `thinking_delta` + `signature_delta` + `redacted_thinking`; encode `ContentBlock::Thinking` back to wire (plain + redacted)
- [x] `cogito-model::openai_compat`: `<think>` two-state SSE parser + `reasoning_content` field reader (mutually exclusive); `include_prior_thinking` provider config (default `false`); encode wraps in `<think>...</think>` when opt-in
- [x] Resume-chaos: new `thinking_then_text_then_tool` scenario; all 4 oracles pass for every crash boundary
- [x] Docs: H02/H04/H06 component docs + AGENTS.md inviolable rules #8/#9 + `docs/data-model/jsonl-v1.md` additive entry
- [x] **ADR-0019**: Reasoning content modeling and event scope (Accepted 2026-05-22)

#### Sprint 5 · Hook Pipeline 实化 (1 day)

**Promoted from old Sprint 7 upper half** by 2026-05-22 rebalance —
Hooks need to be real before Skills (Sprint 7) and Plugins (v0.2
Sprint 12) can load hooks from disk.

- [x] H09 Hook Pipeline with purity rule enforcement (see `docs/components/H09-hook-pipeline.md`)
- [x] Two example hooks (sensitive content, bash audit)
- [x] `MetricsRecorder` trait in protocol + default no-op
- [x] `HookProvider` trait shape lets v0.2 Plugin add hooks without trait change (provider-aggregation pattern — see rebalance spec §7.2)
- [x] Per-hook P99 latency budget verified

#### Sprint 6 · Context Management — ADR-0008 + C2 trait freeze + first Compactor (2–2.5 days)

**Promoted from "post-Sprint-2 spike" to numbered sprint** by
2026-05-22 rebalance — enables team-parallel context strategy
contributions and unblocks Sprint 7 Skill injection into H11.

- [x] Research carryover: Codex (`run_inline_auto_compact_task`), Claude Code (`/compact` + auto), Manus, other SaaS agent platforms — trigger policies and persisted shape
- [x] **ADR-0008**: Context Management — freeze `Compactor` / `HistoryProjector` / `SystemPromptInjector` traits + event variants (`ContextCompacted`, `ContextDecisionRecorded`, `SystemPromptInjected`, `ToolFilterOverridden`) + trigger policy + summarization model selection rules
- [x] `cogito-protocol`: additive `EventPayload` variants for context lifecycle (per `#[non_exhaustive]`, no schema_version bump)
- [x] **New crate `cogito-context`** (umbrella): hosts all Compactor / HistoryProjector / SystemPromptInjector implementations as modules; future strategies (`compactor::summarize`, `compactor::sliding`, …) are added as modules, not new crates; `build_pipeline(&ContextConfig)` factory lives here
- [x] v0.1 ships only `cogito_context::compactor::truncate` as the reference Compactor
- [x] `cogito-core::harness`: H11 implementation; H01 `Init → ContextManaged` transition stops being a pass-through
- [x] H04 history projection: honor `ContextCompacted` events
- [x] H03 Resume Coordinator: crash-mid-compaction recovery
- [x] Chaos test: inject crash during summarization model call (skipped if v0.1 reference Compactor is truncate-only)

#### Sprint 7 · Skill loader (`cogito-skills`) — ADR-0020 (1.5–2 days)

**New sprint** — agentskills.io-compatible Skill loader that lets team
members ship knowledge packs as markdown + frontmatter, no Rust
required.

- [x] **ADR-0020**: Skill loader — locks K5 sigil activation (`$SkillName` + `/skill X` dual channel; no `load_skill` tool), scope precedence (Repo > User > Plugin > System), `SKILL.md` frontmatter schema (`name` / `description` / `disable-model-invocation` / `user-invocable`), bundled-scripts deferral (see ADR-0023)
- [x] **New crate `cogito-skills`** (Hands): SkillRegistry + scope-based filesystem discovery + frontmatter parser + sigil regex + `SkillProvider` trait impl
- [x] `cogito-protocol`: add `SkillProvider` trait + `EventPayload::SkillActivated { skill_name, source, recorded_event_id }` (additive, no schema_version bump)
- [x] H04 Prompt Composer: inject "Available Skills" block (name + description with character cap per skill)
- [x] H06 Stream Demultiplexer: detect sigil in `text_delta`; emit `ModelEvent::SkillActivationRequested`
- [x] H11 Context Manage: on `SkillActivationRequested`, inject full `SKILL.md` as user-role message before next turn (via `SystemPromptInjector` trait from Sprint 6)
- [x] CLI: `/skill <name>` slash command in `cogito chat` REPL
- [x] Sigil edge-case guardrails (see rebalance spec §7.1): match only registered skill names + system-prompt escape instruction
- [x] Smoke test: skill defined under `.cogito/skills/` activates via sigil + via slash; SKILL.md content reaches model
- [x] Resume-chaos: new scenario `text_then_skill_then_tool` — crash injection at boundaries with skill-activated context

#### Sprint 8 · Async Jobs (2 days)

**Renumbered from old Sprint 5** by 2026-05-22 rebalance. Required by
v0.3 Subagent S1 `wait_agent` semantics and v0.4 multi-replica resume.

- [ ] `cogito-jobs` implements `JobManager` (tokio task + JSONL job log persistence)
- [ ] `cogito-jobs` provides cross-process job state persistence (mirrors event log structure; required by Sprint 3 ResumePausedJob path — see Sprint 3 spec §5.6)
- [ ] H08 Tool Dispatcher async path (handles `InvokeOutcome::Async(JobId)`)
- [ ] One real long task tool (`run_tests` or similar)
- [ ] Loop pauses on async, resumes on completion
- [ ] Mid-pause user input handling: queued, processed after current turn

#### Sprint 9 · Multi-model Strategy + TUI (2 days)

**Merged from old Sprint 6 + old Sprint 7 lower half** by 2026-05-22
rebalance.

- [ ] OpenAI adapter in `cogito-model` (Responses API; ContentBlock serialization to OpenAI's flat top-level items)
- [ ] H10 Strategy Selector with YAML config loading from `strategies/*.yaml`
- [ ] CLI `--model` flag selects strategy
- [ ] Per-strategy `model_params`, `allowed_tools`, `system_prompt`
- [ ] Basic TUI with ratatui (replicates `cogito chat`)

#### Sprint 10 · v0.1 硬化 + tag v0.1.0 (1 day)

**Renumbered from old Sprint 8.** Includes the standalone
`cogito-store-jsonl` → `cogito-store` rename PR (see ADR-0024).

- [ ] All component design docs cross-referenced and current
- [ ] `cogito-store-jsonl` → `cogito-store` rename PR landed (see ADR-0024); JSONL becomes the default Cargo feature
- [ ] CHANGELOG.md initial entry
- [ ] Tag `v0.1.0`

### v0.2 · Extensibility

**Theme renamed from "Storage + Multimodal"** by 2026-05-22 rebalance.
Goal: pack Skills + Subagents + Hooks + MCP into a shippable Plugin
unit so team members ship domain capabilities as one directory, not as
scattered config edits. Subagent ships in minimal form (`delegate`
tool); Plugin ships local-path only.

#### Sprint 11 · Subagent (S2 minimal) — ADR-0011 v0.2 scope (1–1.5 days)

- [ ] **ADR-0011 v0.2 amendment**: Subagent minimal scope — `delegate(role, input) → output` tool; child session is an independent top-level session (no `parent_session_id` event tree); failure semantics = child failure → `ToolResult::Error`; no child-session state persisted in parent's event log
- [ ] **No new crate** — module lives in `cogito-core::runtime::subagent` (~200 LoC)
- [ ] `cogito-protocol`: add `BrainSpawner` trait (the v0.3 full surface is amendment-only)
- [ ] `cogito-core::runtime`: implement `BrainSpawner` (recursive task hosting; sync child-completion path only)
- [ ] `cogito-core::runtime::subagent`: `DelegateToolProvider` (impl `ToolProvider`) holding `Arc<dyn BrainSpawner>` via `ExecCtx`
- [ ] Strategy YAML loading: `delegate` arg `role` maps to a known strategy
- [ ] Integration test: parent session invokes `delegate`, child session runs to completion, parent receives final text

#### Sprint 12 · Plugin (P1 local-only) — ADR-0021 (1.5–2 days)

- [ ] **ADR-0021**: Plugin manifest + loader — locks TOML primary (`.cogito-plugin/plugin.toml`) + Claude-plugin JSON compat read (`.claude-plugin/plugin.json`); default artifact paths (`skills/` / `agents/` / `hooks/hooks.toml` / `mcp.toml` / `commands/`); `<plugin_id>:<artifact_name>` namespace for ALL bundled artifacts; per-plugin / per-artifact enable/disable; **v0.2 distribution scope = local path only**
- [ ] **New crate `cogito-plugin`** (Hands): manifest parsers (TOML + JSON compat) + filesystem scan + provider composition (SkillProvider + HookProvider + McpToolProvider + slash-command registry + strategy registry)
- [ ] `cogito-config`: `[[plugins]] path = "..."` entries; per-plugin `enabled` flag; `[[plugins.artifact_overrides]]` block
- [ ] Plugin id uniqueness check at startup (fatal if duplicate; same shape as MCP server name uniqueness from ADR-0018)
- [ ] `cogito-cli chat`: load plugins via existing `--config` path; wire into existing provider composites
- [ ] Integration test: local plugin contributing 1 skill + 1 hook + 1 MCP server + 1 subagent role → all four artifacts callable from Brain through normal provider abstractions

#### Sprint 13 · v0.2 硬化 + tag v0.2.0 (1 day)

- [ ] Cross-scope same-name collision tests (Repo-skill vs Plugin-namespaced skill)
- [ ] Resume-chaos: new scenario `plugin_skill_then_tool` — crash injection while a plugin-loaded skill is mid-activation
- [ ] CHANGELOG.md v0.2 entry
- [ ] Tag `v0.2.0`

### v0.3 · Distributed Collaboration

**Theme renamed from "Subagent"** by 2026-05-22 rebalance — v0.3 now
covers both the Subagent full upgrade AND Plugin git distribution.

- [ ] **ADR-0011 v0.3 amendment**: Subagent full — `BrainSpawner` trait + 4 tools (`spawn_agent` / `wait_agent` / `send_input` / `cancel_agent`) + `parent_session_id` / `depth` / `role` session metadata + parent-child crash semantics + `SubagentSpawned` / `SubagentInputSent` / `SubagentCompleted` event variants + depth-limit enforcement
- [ ] `delegate` tool retained as syntactic sugar for `spawn_agent` + sync `wait_agent` (no behavior change for v0.2 consumers)
- [ ] Decision point: whether to extract `cogito-subagent` crate from `cogito-core::runtime::subagent` (criterion: LoC > 1k + low dep overlap with rest of runtime)
- [ ] Example strategies: `planner.yaml`, `coder.yaml`, `critic.yaml`
- [ ] Crash recovery chaos scenarios: parent crash → child continues; child crash → parent gets `AsyncFailed`
- [ ] **ADR-0022**: Plugin distribution — git URL + commit/tag pin, `cogito.lock` file, `cogito plugin sync` command, network-failure-non-fatal-if-cached semantics
- [ ] `cogito-plugin`: git fetch + cache layout (`~/.cache/cogito/plugins/<content-hash>/`)
- [ ] `cogito-cli`: `cogito plugin sync` + `cogito plugin sync --check` + `cogito plugin list` + `cogito plugin update <id>`
- [ ] Lock-file schema (TOML)
- [ ] Tag `v0.3.0`

### v0.4 · SaaS-ready

**Goal**: enable multi-replica deployment behind a consumer's gateway.

- [ ] **ADR-0012**: Sandbox lifecycle (lazy provisioning, pets-vs-cattle) _(renumbered from ADR-0010)_
- [ ] **ADR-0013**: Credential isolation (sandbox proxy pattern) _(renumbered from ADR-0011)_
- [ ] **ADR-0014**: TenantContext propagation _(renumbered from ADR-0012)_
- [ ] `cogito-store --features postgres`: production multi-replica backend (folded into umbrella `cogito-store` crate per ADR-0024; was originally `cogito-store-postgres`)
- [ ] `cogito-storage-s3` crate: object storage backend
- [ ] `cogito-protocol`: add `TenantContext` optional field on `ExecCtx`
- [ ] `cogito-protocol`: add `MetricsRecorder` trait
- [ ] `cogito-observability-otel` crate: OpenTelemetry adapter (traces + metrics)
- [ ] Per-session resource budget enforcement (memory cap, CPU time cap)
- [ ] `cogito-sandbox` redesign: lazy provisioning + credential proxy
- [ ] Tag `v0.4.0`

### v0.5 · Storage + Multimodal

**Theme absorbed from old v0.2 + old v0.5** by 2026-05-22 rebalance.
Goal: introduce `StorageSystem` as the third protocol pillar, light
up `ContentBlock::Image` end-to-end, and ship the full multimedia
tool catalog in one cohesive version.

- [ ] **ADR-0009**: `StorageSystem` trait + URI scheme + `ContentBlock::Image` variant — moved from v0.2 to v0.5 by 2026-05-22 rebalance (URL-as-text via tools covered v0.1–v0.4)
- [ ] **ADR-0010**: multimedia tool conventions (mime types, `outputs_model_visible_multimodal` flag, etc.) — moved from v0.2 to v0.5
- [ ] `cogito-protocol`: add `StorageSystem` trait + `ContentBlock::Image`
- [ ] `cogito-storage-local` crate: `file://` + `http(s)://` (with local cache) + `blob://` (mapped to local dir)
- [ ] `ExecCtx.storage: Arc<dyn StorageSystem>` field
- [ ] `cogito-tools-multimedia` crate, full catalog:
  - [ ] `transcribe_audio(audio_uri) -> text`
  - [ ] `extract_frames(video_uri) -> Vec<image_uri>`
  - [ ] `summarize_video(video_uri) -> text`
  - [ ] `describe_image(image_uri) -> text`
  - [ ] `analyze_frame(image_uri, prompt) -> structured`
  - [ ] `synthesize_speech(text) -> audio_uri`
- [ ] `ContentBlock::Image` wired through `ModelGateway` adapters (Anthropic native; OpenAI image_url)
- [ ] `outputs_model_visible_multimodal` flag honored by H05 (filters tools incompatible with selected model)
- [ ] Default secret redactor implementation
- [ ] Tag `v0.5.0`

### v0.6 · Hardening + Plugin Marketplace spike

**Theme micro-extended** by 2026-05-22 rebalance to include the P3
marketplace spike (after v0.3 P2 git fetch had a release cycle of
real use).

- [ ] Hook policy maturity: per-strategy hook config, hook composition rules
- [ ] Load test scaffolding: 1000 concurrent sessions per process target
- [ ] Soak test: 24h continuous run with no leaks / no degradation
- [ ] Event log migration tooling (v(N-1) → vN converter, with `replay_equivalence` test)
- [ ] **ADR-0015**: Storage HTTP wire protocol _(renumbered from ADR-0013 by PR #6)_
- [ ] `cogito-storage-http` crate: HTTP backend adapter
- [ ] Plugin marketplace (P3) design spike — `marketplace.json` index protocol, HTTP marketplace backend, `cogito plugin install <name>@<marketplace>`, signing model (optional, can defer to v0.7)
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

# Roadmap

> Version-driven plan toward 1.0 GA and SaaS-ready 0.4. See
> `ARCHITECTURE.md` §"Version evolution path" for the full picture and
> ADR-0005 for the quality gates each version must meet.

## Current

> **v0.1 · Foundation — complete; tagged `v0.1.0` (2026-05-29).** All
> sprints 0–10 done. One tracked deferral carried forward: the Sprint 4
> live-server MCP happy-path integration test (see the Sprint 4 closure
> note); it was a v0.2 candidate but no in-process MCP test-server fixture
> materialized, so it is now carried forward to v0.3.
> **v0.2 · Extensibility — Sprints 11–12 shipped; Skill-support /
> Workspace workstream shipped; Sprint 13 in progress.** Sprint 11
> (Subagent S2 minimal); Sprint 12 (per-session provider injection —
> ADR-0028 — and the local-path plugin loader for Skills + MCP —
> ADR-0021). An unplanned but coherent **Complete Skill Support +
> Workspace seam** workstream then landed (PRs #35–#52, spec
> `docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`):
> the Workspace seam (ADR-0030/0031), the builtin file/search tool catalog
> (`write_file` / `list_dir` / `edit` / `grep` / `glob`), and skill-bundle
> reachability + dependency descriptor (ADR-0029/0032/0033, ADR-0023
> finalized). Now: Sprint 13 (v0.2 硬化 + tag `v0.2.0`).

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
- [x] Workspace topology fixed per ADR-0004: dropped `cogito-conversation`, added `cogito-store` (originally `cogito-store-jsonl`; renamed per ADR-0024), stripped Hands/Boundary/Session deps from `cogito-core`
- [x] Protocol types landed: `ExecutionClass`, `StreamEvent`, `JobCompletionEvent`, `JobManager::on_complete`, `TurnOutcome`, `TurnFailureReason` (12+ serde-roundtrip tests passing)
- [x] Runtime module scaffolded (stubs): `Runtime`, `RuntimeBuilder`, `SessionHandle`, per-session loop task (`runtime::session_loop::run_session` + `SessionShared`), `store_writer`
- [x] CI runs `make ci` (fmt + clippy + layer-check + test) + cargo-deny job
- [x] Toolchain aligned to MSRV 1.85 (edition 2024 requirement)

#### Sprint 1 · H02 Step Recorder + JSONL store (1.5 day)
- [x] `cogito-protocol` defines `ConversationEvent` with `schema_version: u32` + `Vec<ContentBlock>` payload (Text + ToolUse + ToolResult variants)
- [x] `cogito-protocol` defines `ConversationStore` trait
- [x] `cogito-store` (then named `cogito-store-jsonl`) implementation: per-session file, `flush` per event, append-only (durability scope: dev/debug — see ADR-0007)
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

- [x] `cogito-mcp` crate: thin wrapper over `rmcp` 1.5 with `transport-child-process` (stdio) + `transport-streamable-http-client-reqwest` (streamable-HTTP); bearer-token via env-var. OAuth flow deferred to a follow-up ADR.
- [x] `McpToolProvider`: `ToolProvider` impl aggregating tools across configured servers via `mcp__<server>__<tool>` qualified naming (sanitize disallowed chars to `_`, 64-char cap with SHA-1 truncation suffix, dedupe with warn).
- [x] `cogito-config`: `mcp_servers` section with `Stdio` / `StreamableHttp` transports + per-server `enabled_tools` / `disabled_tools` + startup/tool timeouts.
- [x] `cogito-cli chat`: wire `McpToolProvider` into the `Runtime` builder via the existing `--config` path; tool surface visible to Brain.
- [ ] Integration test: exercise `tools/list` + `tools/call` end-to-end through `cogito chat` against a real streamable-HTTP MCP server with bearer auth. **(Deferred — see closure note.)** Landed instead: failure-path integration tests covering the soft-skip / fault-containment invariants.
- [x] **ADR-0018**: MCP integration — transport scope, namespacing convention, deferred OAuth, license note (`rmcp` upstream + Codex pattern attribution).

**Sprint 4 closure — deferred item:**

The success-path integration test (live streamable-HTTP MCP server with
bearer auth, asserting `tools/list` + `tools/call` end-to-end through
`cogito chat`) was **deferred**: it requires a real MCP server binary or
an in-process test server, which was out of scope for the soft-skip
work that landed. What shipped instead (`crates/cogito-mcp/tests/integration.rs`)
covers the resilience invariants — missing bearer env yields a per-server
failure (not a runtime break), a failed server is contained without
affecting healthy servers, duplicate server names skip later entries, and
an all-servers-fail config still builds a usable runtime. The happy-path
acceptance test is tracked for a follow-up (candidate: a Sprint 10 or
v0.2 task once a lightweight in-process MCP test server fixture exists).

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

- [x] `cogito-jobs` implements `JobManager` (in-memory; jobs run as `tokio::task`s with `on_complete` sink registration)
- [x] Process-bounded jobs; conversation event log is the sole persistence; resume coordinator synthesizes `JobOutcome::Failed` for any open job whose process was lost. See `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`.
- [x] H08 Tool Dispatcher async path (handles `InvokeOutcome::Async(JobId)`)
- [x] One real long task tool (`RunTestsTool` wrapping `cargo nextest run`)
- [x] Loop pauses on async, resumes on completion
- [x] Mid-pause user input handling: single-slot queue (latest-wins, warn on overwrite); processed after current turn drains
- [x] `EventPayload::JobSubmitted { call_id, job_id, tool_name }` additive variant (no `SCHEMA_VERSION` bump); H03 reads `call_id` directly instead of walk-back
- [x] Fix cancel-token-disconnect: `SessionShared` and `SessionState` share `Arc<parking_lot::Mutex<CancellationToken>>` so `cancel_turn` works on every turn (closes Sprint 3 latent narrowing)
- [x] Resume-chaos: new `paused_async_job` scenario with three crash boundaries

#### Sprint 9a · Multi-model Strategy (2 days)

**Split from old Sprint 9** by 2026-05-27 spec/plan. Carries the
multi-model half of the original Sprint 9. TUI carries to Sprint 9b.

- [x] OpenAI Responses adapter in `cogito-model` (Responses API; ContentBlock serialization with native reasoning items per ADR-0019)
- [x] H10 Strategy Selector — markdown+frontmatter strategy registry via new crate `cogito-strategy` (FS-backed `StrategyRegistry` impl)
- [x] CLI `--strategy <name>` flag selects strategy; `--model` overrides strategy.model
- [x] Per-strategy `model_params`, `allowed_tools`, `system_prompt`, `context`
- [x] Three example strategies under `.cogito/strategies/` (coder, planner, reviewer)
- [x] **ADR-0026**: Strategy registry — markdown+frontmatter format, Repo > User scope precedence, supersedes ADR-0017 §13
- [x] Resume-chaos `strategy_with_tool_filter` scenario passes all 4 oracles

#### Sprint 9b · TUI (1 day)

**Split from old Sprint 9** by 2026-05-27 spec/plan. Replicates
`cogito chat` in a ratatui TUI; consumes the same `resolve_strategy`
helper landed in 9a.

- [x] Basic TUI with ratatui replicating `cogito chat`
- [x] `cogito-tui` reads the same FsStrategyRegistry; `--strategy` flag honored
- [x] Spec landed: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`

#### Sprint 10 · v0.1 硬化 + tag v0.1.0 (1 day)

**Renumbered from old Sprint 8.** Includes the standalone
`cogito-store-jsonl` → `cogito-store` rename PR (see ADR-0024).

- [x] All component design docs cross-referenced and current (H01–H11 reconciled with shipped reality; Status banners refreshed; decorative glyphs removed)
- [x] `cogito-store-jsonl` → `cogito-store` rename PR landed (see ADR-0024); JSONL becomes the default Cargo feature
- [x] **明示追加(非原排期)**: 核心工具 `bash` + `web_fetch`,以及 `cogito-sandbox` 的 `CommandExecutor` 接缝(`DirectExecutor` + `build_executor`)与 `[tools]` 配置段。详见 ADR-0027 / `docs/components/cogito-sandbox.md`
- [x] CHANGELOG.md initial `[0.1.0]` release entry
- [x] Tag `v0.1.0`

### v0.2 · Extensibility

**Theme renamed from "Storage + Multimodal"** by 2026-05-22 rebalance.
Goal: pack Skills + Subagents + Hooks + MCP into a shippable Plugin
unit so team members ship domain capabilities as one directory, not as
scattered config edits. Subagent ships in minimal form (`delegate`
tool); Plugin ships local-path only.

#### Sprint 11 · Subagent (S2 minimal) — ADR-0011 v0.2 scope (1–1.5 days)

- [x] **ADR-0011 v0.2 amendment**: Subagent minimal scope — `delegate(role, input) → output` tool; child session is an independent top-level session (no `parent_session_id` event tree); failure semantics = child failure → `ToolResult::Error`; no child-session state persisted in parent's event log. Linkage (`parent_session_id` / `parent_call_id` / `subagent_depth`) is recorded child-side in `SessionMeta`; a live observability bridge tags child `StreamEvent`s with the delegate call id (`StreamEvent::subagent_call_id`).
- [x] **No new crate** — module lives in `cogito-core::runtime::subagent`
- [x] `cogito-protocol`: add `BrainSpawner` trait (`run_to_completion(DelegateRequest) -> Result<String, SpawnError>`; injected via `ExecCtx.brain_spawner`). The v0.3 full surface is amendment-only.
- [x] `cogito-core::runtime`: implement `BrainSpawner` (via `RuntimeSpawner` newtype over `Arc<Runtime>`; sync child-completion path only, bounded by a 300s `CHILD_DRIVE_TIMEOUT` backstop)
- [x] `cogito-core::runtime::subagent`: `DelegateToolProvider` (impl `ToolProvider`, `AlwaysSync`) reading `Arc<dyn BrainSpawner>` from `ExecCtx`; depth guard via `DEFAULT_MAX_SUBAGENT_DEPTH = 3`
- [x] Strategy role mapping: `delegate` arg `role` resolves to a known strategy via `RuntimeBuilder::strategy_registry`
- [x] Integration test: parent session invokes `delegate`, child session runs to completion, parent receives final text (acceptance test); plus a depth test asserting recursion terminates at the limit with a depth-limit `ToolResult::Error`

#### Sprint 12 · Plugin (P1 Skills+MCP, local-only) + per-session injection — ADR-0021 + ADR-0028 (2–3 days)

**Scope reshaped during 2026-05-30 brainstorming** (design:
`docs/superpowers/specs/2026-05-30-sprint-12-saas-session-plugin-design.md`).
Two ordered pieces: (1) per-session provider injection — a SaaS-ready
capability pulled forward from v0.4 on explicit consumer direction
(see deviation note below); (2) plugin loader narrowed to Skills + MCP.
Hooks / subagent-roles (`agents/`) / slash-commands (`commands/`) are
**deferred** (a cogito hook is a pure no-I/O Rust gate, so its
data-format is product-form-dependent; `agents/` will reuse strategies;
no command registry exists yet) — their plugin directories are reserved
but not loaded.

Piece 1 — per-session provider injection (ADR-0028) — **shipped (PR #33)**:
- [x] **ADR-0028**: `SessionSpec` + `Runtime::open_session_with(id, mode, spec)`; legacy `open_session` delegates to an all-`None` spec
- [x] Per-session providers become mutable session state: `SessionCommand::UpdateSession` + `SessionHandle::update_session`, effective at the next turn boundary (TurnDeps already rebuilt per turn); a skills/strategy swap also rebuilds the context pipeline so H11 system-prompt injection sees the new provider
- [x] `tenant_id` / `user_id` stamped into `SessionMeta`; composition stays caller-side (core swaps whole Arcs)
- [x] Resume narrowing: caller re-supplies the current `SessionSpec`; core does not persist provider identity (self-describing multi-replica resume → v0.4)
- [x] Resume-chaos `session_spec_mutated_then_resume`: open spec A → mutate to B → crash boundaries → resume with B → all 4 oracles pass
- [x] Brain (H01–H11) unchanged

Piece 2 — plugin loader, Skills + MCP (ADR-0021) — **shipped (PR #34)**:
- [x] **ADR-0021**: TOML primary (`.cogito-plugin/plugin.toml`) + Claude-plugin JSON metadata-only fallback; `<plugin_id>:<artifact_name>` namespace; per-plugin / per-artifact enable/disable; plugin-id uniqueness fatal; **local path only**
- [x] **New crate `cogito-plugin`** (Hands): manifest parsers + local-path discovery + namespacing + overrides; `PluginSet::load → PluginContributions { skill_roots, mcp_servers }` (produces contributions; registries keep cross-scope merge)
- [x] `cogito-config`: `[[plugins]]` Reserved → Locked; `PluginEntry` defined in `cogito-plugin`, aggregated by `cogito-config` (mirrors `cogito-config → cogito-mcp`)
- [x] Skills folded via `ScanConfig.plugin_roots` (Plugin scope); MCP concatenated into one `build_mcp_provider`
- [x] `cogito-cli chat`: loads the plugin set once per run and folds skill roots + MCP servers into the Runtime's default providers (single-tenant CLI; the per-session `open_session_with` / `update_session` path is the consumer-server surface, exercised by ADR-0028's chaos test)
- [x] Acceptance tests (`cogito-plugin/tests/`): local plugin with 1 skill + 1 MCP server → skill reachable end-to-end through the real `SkillRegistry`, MCP namespaced at contribution level; mid-session "add a 2nd plugin" recomposition asserted at the contribution level (live `update_session` swap proven by ADR-0028's `session_spec_mutated_then_resume`)

**Deferred follow-up:** `plugin.id` format (`[a-z0-9-]+`, ADR-0021 §1) is not yet enforced in the manifest parser — acceptable for operator-authored local-path plugins in v0.2.

**Deviation note (2026-05-30):** ADR-0028 advances a v0.4 "SaaS-ready"
item (per-session provider injection) into v0.2. Reason: the consumer
needs multi-tenant, single-process, per-request (and mid-session
mutable) tool/skill surfaces now. The v0.4 entry below is correspondingly
narrowed to the remaining multi-replica / TenantContext / store work.

#### Skill-support + Workspace seam (unplanned; landed between Sprint 12 and 13)

**Not in the original v0.2 plan.** A coherent workstream that closed the
gap between "a Skill injects instructions" and "a Skill can reach its
bundled files, run scripts, and write artifacts" across the Local
(CLI/TUI) and SaaS profiles. Design:
`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`.
Recorded here after the fact for an accurate v0.2 release picture (specs
fragment the picture over time — durable docs must carry the decision).

- [x] **ADR-0029**: skill bundled-file path exposure — `SkillContent.root` points at the skill's own directory so the model can resolve relative references (PR #37)
- [x] **ADR-0030**: Workspace seam — lock the `Workspace` trait + `LocalWorkspace` (Phase 1, PRs #38/#40)
- [x] **ADR-0031**: Workspace provisioning & scoping (TUI vs SaaS); bash exec cwd unified on the session workspace root (§5, PR #42)
- [x] `read_file` migrated onto the Workspace seam; `write_file` tool wired through `ExecCtx` (PRs #40/#41)
- [x] Builtin file/search tool catalog over the Workspace seam: `list_dir` (#43), `edit` (#44), `grep` (#45), `glob` (#46); file tools wired into the TUI
- [x] **ADR-0032**: skill-bundle reachability via a read-only skill-root scope — `ExecCtx.skill_roots` + `read_file`/`list_dir` read-only skill scope, turned on (PRs #47/#48/#49)
- [x] **ADR-0033**: skill runtime dependencies — no custom descriptor (Phase 2, PR #50); ADR-0023 (bundled-script execution) finalized + script-bearing-skill e2e (PR #51)
- [x] Duplicate skill name is non-fatal, resolved by precedence (PR #52)

#### Sprint 13 · v0.2 硬化 + tag v0.2.0 (1 day)

- [x] Cross-scope same-name collision test (Repo-skill vs Plugin-namespaced skill): plugin skills are namespaced `<plugin_id>:<name>` (ADR-0021 §3), so a repo `review` and plugin `acme:review` coexist rather than collide — pinned by `repo_and_plugin_same_name_coexist_via_namespace` in `cogito-skills/tests/discovery.rs`. Same-scope duplicates resolve by precedence, non-fatal (`cogito-plugin` PR #52; `duplicate_name_in_same_dir_resolves_by_precedence_not_fatal`)
- [x] `plugin.id` format validation (`[a-z0-9-]+`, ADR-0021 §1) enforced in the manifest parser — closes the Sprint 12 deferred follow-up (`PluginError::InvalidId`)
- [x] Resume-chaos: new scenario `plugin_skill_then_tool` — crash injection while a plugin-loaded skill is mid-activation; all 4 oracles pass at both boundaries
- [ ] CHANGELOG.md v0.2 entry
- [ ] Tag `v0.2.0`

**Sprint 13 carry-forward to v0.3:** the Sprint 4 MCP happy-path
integration test (live streamable-HTTP MCP server with bearer auth,
asserting `tools/list` + `tools/call` end-to-end through `cogito chat`)
remains deferred — it needs an in-process MCP test-server fixture that
does not yet exist. Resilience invariants are already covered
(`crates/cogito-mcp/tests/integration.rs`).

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
- [ ] **Carried forward from v0.1/v0.2:** Sprint 4 MCP happy-path integration test — build an in-process streamable-HTTP MCP test-server fixture, then assert `tools/list` + `tools/call` end-to-end through `cogito chat` with bearer auth
- [ ] Tag `v0.3.0`

### v0.4 · SaaS-ready

**Goal**: enable multi-replica deployment behind a consumer's gateway.

> **Pulled forward:** per-session provider injection (`SessionSpec` /
> `open_session_with`, ADR-0028) landed early in v0.2 Sprint 12. v0.4
> now covers the remaining multi-replica / TenantContext / store work.
>
> **Cut (2026-06-03):** self-describing resume (rebuild a session's
> provider surface on any replica without the caller re-supplying the
> spec) is **dropped**. The consumer (praxis) owns gateway routing and
> re-supplies the `SessionSpec` on every resume, and disallows
> cogito-internal-initiated surface changes — so there is no surface
> mutation the consumer's gateway cannot reconstruct. Caller-re-supplies
> (ADR-0028 §5) stays the resume contract. (Draft ADR-0035 withdrawn.)

- [ ] **ADR-0012**: Sandbox lifecycle (lazy provisioning, pets-vs-cattle) _(renumbered from ADR-0010)_
- [ ] **ADR-0013**: Credential isolation (sandbox proxy pattern) _(renumbered from ADR-0011)_
- [x] **ADR-0014**: TenantContext — **Accepted Route A (2026-06-03): no `ExecCtx` propagation, no protocol change.** Tenant identity stays in `SessionMeta` (ADR-0028) for attribution; consumers bind tenant into per-session providers (ADR-0028). _(renumbered from ADR-0012)_
- [ ] `cogito-store --features postgres`: production multi-replica backend (folded into umbrella `cogito-store` crate per ADR-0024; was originally `cogito-store-postgres`)
- [ ] `cogito-storage-s3` crate: object storage backend
- [ ] ~~`cogito-protocol`: add `TenantContext` optional field on `ExecCtx`~~ — **dropped (ADR-0014 Route A)**
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

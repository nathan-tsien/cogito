# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.5] - 2026-06-20

**Stream/log message correlation key (ADR-0041).** A live subscriber could not
align a streamed assistant message with the *same* message in the persisted
event log: the broadcast carried no per-message identity, so an embedding
consumer that assembles the live block stream and also reads history had no
stable key to fold streaming deltas into the right message, nor to dedupe on
reconnect — surfacing downstream as a streamed assistant message with an
empty, unkeyable id stuck on a loading placeholder. Additive and
backward-compatible: new optional, serde-default fields plus one new broadcast
variant; no `SCHEMA_VERSION` bump, no persisted-event migration.

### Added

- **Per-message `MessageId`, minted at message-open.** An assistant message
  (one model call's output) now opens with a new
  `StreamEvent::AssistantMessageStarted { message_id, .. }`, and the same
  opaque `message_id` rides on the message's live delta/tool events
  (`TextDelta` / `ThinkingDelta` / `ToolDispatchStarted` / `ToolDispatchEnded`)
  and is stamped on its persisted composing events (`AssistantMessageAppended`
  / `ThinkingBlockRecorded` / `ToolUseRecorded`). A live subscriber and a
  history projection therefore derive the *same* per-message identity —
  reconnect/resubscribe dedup, or upserting an in-flight message into a store
  that is later read back. The id is opaque: it encodes no role or turn
  structure. The persisted fields are optional + serde-default (the ADR-0019
  additive precedent), so old logs read back with `None`; no `SCHEMA_VERSION`
  bump.
- **Auxiliary `turn_id` on the live stream.** Every turn-scoped `StreamEvent`
  now carries an optional `turn_id`, so a subscriber can attribute a streamed
  event to its turn in the persisted log (which already records `turn_id`).
  This is turn linkage, not a per-message identity — use `message_id` for the
  latter.

### Changed

- `StreamEvent::TurnPaused` / `TurnResumed` / `TurnCancelled` are now struct
  variants carrying the optional `turn_id`. The serialized wire form is
  unchanged when the field is absent (`{"kind":"turn_paused"}`); a `match` on
  the bare unit variant must widen to `{ .. }`.

## [0.2.4] - 2026-06-11

**Turn-lifecycle event correctness.** Two fixes to how a turn's terminal event
is recorded and surfaced. Additive: one optional, serde-default field on a
broadcast event; no new enum variant, no `SCHEMA_VERSION` bump, no
persisted-event change.

### Fixed

- **`TurnCompleted` recorded exactly once.** A completed turn was persisted and
  broadcast twice — the H01 FSM transition wrote `TurnCompleted`, then the
  session loop's terminal hook wrote it again — so every completed turn landed
  as a duplicate pair in the `ConversationStore` and on the broadcast channel,
  forcing embedding consumers to dedupe and making any event-log-derived
  accounting (turn counts, replay assertions) double-count. The FSM transition
  is now the sole writer of the terminal event, symmetric with the
  already-correct `Failed` path; the session loop's `Completed` arm is a no-op.
  Guarded by a new exactly-one-`TurnCompleted` regression test mirroring the
  existing exactly-one-`TurnFailed` one.

### Added

- **Turn-level `max_tokens` truncation signal (ADR-0040).** A turn whose final
  model call was cut off by `max_tokens` still completes, but was
  indistinguishable from a clean end-of-turn at the turn boundary — the
  `stop_reason` lived only in the `ModelCallCompleted` event and was never
  surfaced live, so a truncated half-answer was presented as final with no
  flag. `StreamEvent::TurnCompleted` now carries
  `stop_reason: Option<StopReason>` (optional, serde-default — the same shape as
  `subagent_call_id`, fully backward-compatible), letting a subscriber detect
  truncation without scanning model-call events, and the harness emits a
  `tracing::warn!` on truncation. No new `TurnOutcome`/`TurnFailureReason`
  variant — truncation is a completion caveat, not a failure, and the partial
  text stays as the final answer; the persisted event is unchanged (the
  `stop_reason` is already in the adjacent `ModelCallCompleted`). Strategy-level
  policy (fail / auto-continue) and replay-parity persistence are deferred to a
  future ADR.

### Docs / decisions

- **ADR-0040** (turn-level truncation signal) — Accepted, implemented.

## [0.2.3] - 2026-06-07

**Harness loop control.** Two additive changes that give the harness an honest
stop condition and an observability affordance for human-in-the-loop. No
protocol change beyond two additive, `#[non_exhaustive]` enum variants; no
`SCHEMA_VERSION` bump.

### Added

- **Agent-loop iteration budget (ADR-0038).** `HarnessStrategy::max_turns`
  (default 16) was declared but never enforced; the H01 inner loop could run
  unbounded. It is now enforced by replay-derived model-call count
  (`TurnCtx::model_calls`, re-seeded from the event log on every resume path so
  the budget survives a crash mid-turn). On hit, the turn stops with the new
  `TurnFailureReason::MaxTurnsExceeded { turns }` (additive variant). On-hit is
  a **fail** — an honest primitive; continue/summarize/raise-the-budget are
  consumer policy layered on the failure, not core behavior. Orthogonal to
  `MAX_CONSECUTIVE_TOOL_ERRORS` and `TurnTimedOut`. Covered by fresh-turn,
  async-pause/resume, and crash-resume tests.
- **`JobStatus::AwaitingInput` (ADR-0039).** A single observation-only
  `JobStatus` variant, reported solely by `JobManager::status()` — never on the
  turn FSM or the resume path, so it is off the safety-critical resume
  coordinator (no `TurnPaused` payload change, no resume-coordinator change). A
  HITL-capable `JobManager` MAY report it for a job parked on a human (an
  ask-user question or an approval decision) so operators and dashboards can
  distinguish "waiting on a person" from compute `Running`. Optional for
  implementations; the bundled `LocalJobManager` stays CLI-grade and never
  reports it.

### Docs / decisions

- **ADR-0038** (agent-loop iteration budget) — Accepted, implemented.
- **ADR-0039** (human-in-the-loop is a consumer flow over the suspension seam)
  — Accepted. Records the boundary: cogito owns the controllable-harness
  mechanism, not the HITL flow or its policy. Ask-user (`message_ask_user`) and
  tool-approval gates both reduce to the existing `InvokeOutcome::Async` →
  `Paused`/`TurnPaused` → resume-on-`JobCompletion` seam ("humans-as-jobs"); no
  HITL tool/UI/policy is added to core. Specifies the durable-`JobManager`
  contract a multi-replica consumer must satisfy so a turn parked on a human
  survives a process restart (the bundled `LocalJobManager` is in-process and
  will strand a parked turn across restart — documented, not fixed here).

## [0.2.2] - 2026-06-03

### Added

- **Local execution safety for the TUI (ADR-0037).** Three additive parts, no
  protocol change, no `SCHEMA_VERSION` bump:
  - `RuntimeBuilder::hooks(Vec<Arc<dyn HookHandler>>)` — the H09 hook pipeline
    is now consumer-injectable (it was hardcoded to `Vec::new()` with no
    setter). Runtime-level; cloned into each session's pipeline. Mirrors
    `RuntimeBuilder::metrics()`.
  - `CommandGuardHook` (`cogito-core::harness::hooks::command_guard`) — a
    builtin H09 `pre_dispatch` guard that rejects a curated denylist of
    catastrophic `bash` commands (recursive-force `rm` on `/` `~` `$HOME` or
    system dirs, fork bomb, `mkfs`, `dd of=/dev/...`, redirect to a raw block
    device, `chmod -R` on root). Project-local deletes are allowed. It is a
    denylist accident guard, **not a security boundary** (trivially bypassable;
    stops mistakes, not adversaries).
  - `EnvPolicy::Allowlist` on `cogito-sandbox::DirectConfig` +
    `default_safe_env_allowlist()` — start a child `sh -c` from an empty
    environment and copy in only a curated set (`PATH HOME LANG LC_ALL LC_CTYPE
    TMPDIR USER LOGNAME SHELL TERM PWD`), default-deny secrets. Default stays
    `InheritAll` (`#[serde(skip)]`, `cogito.toml` schema unchanged).
  - Wired TUI-only (`cogito-tui` `runtime_build.rs`): the guard is injected and
    the `Direct` sandbox env is scrubbed by default. CLI chat is unchanged in
    this cut. Multi-tenant isolation / Credential Broker stay DEFERRED
    (ADR-0012/0013).

## [0.2.1] - 2026-06-03

**Downstream integration snapshot.** A small, additive Runtime-surface release
that closes a same-process integration gap for embedding consumers. Brain
(H01–H11) is unchanged; `cogito-protocol` is unchanged;
no `ConversationEvent` `SCHEMA_VERSION` bump. All additions are
backward-compatible at the API level. Stability posture stays 0.x: the public
surface may still evolve in a later minor — pin a commit/tag for integration.

**Added**

- `Runtime::close_session(id, deadline)` and `Runtime::get_session(id)`
  (`cogito-core::runtime`): a session can now be re-`Resume`d within the *same*
  `Runtime`. The `sessions` registry was insert-only, so a crashed/closed
  session could not be reopened in-process; `close_session` drives shutdown then
  deregisters, and releases the session's `ConversationStore` resources on actor
  exit (flush + close). Driven by issue #55 (a same-process resume requirement
  from a downstream integration). Pure Runtime surface, no protocol change. See
  ADR-0034.
- `RuntimeBuilder::metrics(Arc<dyn MetricsRecorder>)`: the `MetricsRecorder`
  seam is now consumer-injectable (it was hardcoded to `NoOpMetricsRecorder`
  with no setter, so the trait was unreachable). The default stays
  `NoOpMetricsRecorder`; `open_inner` clones the runtime's recorder into each
  session's `SessionState` and hook pipeline. First step of the observability
  extension point. See ADR-0036.

**Docs / decisions**

- ADR-0034 (Runtime session-registry lifecycle) — Accepted, implemented.
- ADR-0014 (TenantContext) — Accepted, Route A: tenant identity stays in
  `SessionMeta`, no `ExecCtx` propagation; consumers bind tenant into
  per-session providers. No protocol change.
- ADR-0036 (observability extension point) — setter shipped; OTel adapter crate
  and metric density deferred / incremental.
- ADR-0012 / ADR-0013 (sandbox lifecycle / credential isolation → Credential
  Broker) — drafted, DEFERRED (gated on the consumer's untrusted-code /
  bash-exposure decision).
- ADR-0011 (subagent full lifecycle) — v0.3 amendment drafted, deferred (sync
  `delegate` from v0.2 is sufficient for near-term consumer integration).
- ADR-0022 (plugin distribution) — draft finalized, pending review.

## [0.2.0] - 2026-06-02

**v0.2 Extensibility.** Pack domain capabilities into shippable units and
serve many tenants from one process. Subagents arrive in minimal form
(synchronous `delegate`), per-session provider injection (`SessionSpec` /
`open_session_with`) lets one process give every tenant its own
tool/skill/strategy surface, and a local-path Plugin loader bundles Skills
+ MCP servers behind a single namespaced directory. A subsequent
Skill-support + Workspace workstream then made Skills fully operational:
the `Workspace` seam, a builtin file/search tool catalog
(`write_file` / `list_dir` / `edit` / `grep` / `glob`), and skill-bundle
reachability so an activated skill can reach its bundled files and run its
scripts. Brain (H01–H11) stays unchanged throughout; all event-log changes
are additive — no `ConversationEvent` `SCHEMA_VERSION` bump. See the
per-sprint entries below.

### Sprint 13 · v0.2 hardening (2026-06-02)

**Added**

- `cogito-plugin`: `plugin.id` format validation (`[a-z0-9-]+`, ADR-0021 §1)
  enforced in the manifest parser via the new `PluginError::InvalidId`
  variant (empty or out-of-charset ids are rejected for both the
  `.cogito-plugin/plugin.toml` and `.claude-plugin/plugin.json` paths).
  Closes the Sprint 12 deferred follow-up.
- `cogito-skills`: `repo_and_plugin_same_name_coexist_via_namespace` test —
  pins that a Repo skill and a Plugin skill declaring the same bare name do
  not collide, because plugin skills are namespaced `<plugin_id>:<name>`
  (ADR-0021 §3): a repo `review` and a plugin `acme:review` are both
  registered and independently resolvable.
- Resume-chaos: new `plugin_skill_then_tool` scenario — a plugin-loaded
  skill (`SkillSource::Plugin`, addressed via the `$acme:review` namespaced
  sigil) activates across a two-turn flow with `PanicAt` crash injection at
  both `ModelCallCompleted` boundaries. All four oracles plus the
  skill-activation-idempotency oracles pass, proving the H06 sigil / H11
  injection path is source-agnostic under resume.

**Notes**

- The skill chaos helpers (`build_skill_runtime` / `skill_run_to_completion`
  / `skill_run_with_y_fault` / `assert_turn2_suffix_has_skill_body`) were
  parameterized by `SkillProvider` + skill name so the User-scope
  (`text_then_skill_then_tool`) and Plugin-scope (`plugin_skill_then_tool`)
  scenarios share one code path.
- **Carried forward to v0.3:** the Sprint 4 MCP happy-path integration test
  (live streamable-HTTP server, bearer auth, `tools/list` + `tools/call`
  end-to-end) remains deferred — it needs an in-process MCP test-server
  fixture that does not yet exist. Resilience invariants are already covered
  (`crates/cogito-mcp/tests/integration.rs`).

### Complete Skill Support + Workspace seam (2026-06-01)

Unplanned but coherent workstream (PRs #35–#52) that closed the gap between
"a Skill injects instructions" and "a Skill can reach its bundled files, run
its scripts, and write artifacts" — identically across the Local (CLI/TUI)
and SaaS profiles. Design:
`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`.

**Added**

- **`Workspace` seam** (ADR-0030): the `Workspace` trait + `LocalWorkspace`
  host-filesystem implementation, threaded through `ExecCtx`. File tools
  read and write through the seam rather than touching the host directly,
  so the SaaS profile can swap in an isolated per-tenant workspace.
- **Workspace provisioning & scoping** (ADR-0031): TUI vs SaaS provisioning
  rules; bash/job execution cwd unified on the session workspace root (§5).
- **Builtin file/search tool catalog** over the Workspace seam: `write_file`,
  `list_dir`, `edit`, `grep`, `glob`. `read_file` migrated onto the seam.
  File tools wired into the TUI.
- **Skill bundled-file path exposure** (ADR-0029): `SkillContent.root` points
  at the activated skill's own directory so the model can resolve relative
  references in the body (`scripts/`, etc.).
- **Skill-bundle reachability** (ADR-0032): `ExecCtx.skill_roots` plus a
  read-only skill-root scope for `read_file` / `list_dir`, turned on, so an
  activated skill's bundled files are readable without widening the writable
  workspace.
- **Skill runtime dependencies** (ADR-0033): no custom descriptor — runtime
  deps reuse existing mechanisms. ADR-0023 (bundled-script execution)
  finalized with a script-bearing-skill end-to-end test.

**Changed**

- Duplicate skill name within a scope is non-fatal and resolved by
  precedence (alphabetically-first directory wins the tie), mirroring the
  non-fatal MCP duplicate handling.

### Sprint 12 · Per-session injection (ADR-0028) + plugin loader (ADR-0021) — v0.2 (2026-05-31)

**Added**

- **Per-session provider injection** (ADR-0028). `SessionSpec`
  (`tools` / `skills` / `strategy` / `tenant_id` / `user_id`, all optional)
  plus `Runtime::open_session_with(id, mode, spec)`; the legacy
  `open_session` delegates to an all-`None` spec (byte-for-byte equivalent
  for existing callers). One process can now serve many tenants, each with
  its own tool/skill/strategy surface.
- `SessionHandle::update_session(spec)` swaps live providers mid-session via
  `SessionCommand::UpdateSession`, effective at the next turn boundary (each
  turn rebuilds `TurnDeps`). A skills or strategy swap also rebuilds the
  session context pipeline so H11 system-prompt skill injection uses the new
  provider, not only the live sigil-detection path.
- `tenant_id` / `user_id` from the spec are stamped into `SessionMeta` on a
  fresh session. Resume re-supplies the current `SessionSpec`; the core never
  persists provider identity (a provider is code, not state).
- **`cogito-plugin`** (new Hands crate, ADR-0021): local-path plugin loader.
  Parses `.cogito-plugin/plugin.toml` (primary) or `.claude-plugin/plugin.json`
  (metadata-only fallback), discovers bundled Skills + MCP servers, namespaces
  every artifact `<plugin_id>:<name>`, applies per-plugin / per-artifact
  enable/disable overrides, and treats a duplicate plugin id as fatal.
  `PluginSet::load → PluginContributions { skill_roots, mcp_servers }` — the
  loader produces contributions; the existing registries keep cross-scope
  precedence. Hooks / `agents/` / `commands/` directories are reserved but
  not loaded.
- `cogito-skills`: `PluginSkillRoot` + `ScanConfig.plugin_roots`; a Plugin
  discovery scope that namespaces each skill `<plugin_id>:<name>`.
- `cogito-config`: `[[plugins]]` section (Reserved → Locked) parsed into
  `RuntimeConfig.plugins`; `PluginEntry` is owned by `cogito-plugin` and
  aggregated by `cogito-config` (mirrors the `cogito-config → cogito-mcp`
  edge). A malformed `[[plugins]]` entry is fatal at finalize.
- `cogito-cli chat`: loads the plugin set once per run and folds plugin skill
  roots + namespaced MCP servers into the Runtime's default providers.

**Notes**

- Brain (H01–H11) is unchanged; all of ADR-0028 lives in the runtime layer,
  and `cogito-core` gains no dependency on `cogito-plugin` (layer-check
  enforced). All additions are additive — no `ConversationEvent`
  `SCHEMA_VERSION` bump.
- ADR-0028 pulls one slice of the v0.4 "SaaS-ready" theme forward on explicit
  consumer direction; fully self-describing multi-replica resume remains v0.4.
- Deferred follow-up: `plugin.id` format (`[a-z0-9-]+`, ADR-0021 §1) was not
  yet enforced in the manifest parser at Sprint 12 — closed in Sprint 13
  (see `PluginError::InvalidId` above).

### Sprint 11 · Subagent (S2 minimal) — v0.2 (2026-05-30)

**Added**

- **Synchronous `delegate(role, input) -> output` subagent tool** (ADR-0011
  v0.2 minimal scope). A child runs as an independent top-level session and
  its final assistant text is returned to the parent. No new crate — the
  module lives in `cogito-core::runtime::subagent`.
- `cogito-protocol`: new `subagent` module — `BrainSpawner` trait
  (`async fn run_to_completion(&self, DelegateRequest) -> Result<String, SpawnError>`),
  `DelegateRequest`, and `SpawnError` (`UnknownRole` / `OpenFailed` /
  `ChildFailed` / `Timeout`).
- `cogito-protocol`: `ExecCtx` gains `call_id: Option<String>` (the model's
  tool-call id, set by the dispatcher before `invoke`), `subagent_depth: u32`,
  and `brain_spawner: Option<Arc<dyn BrainSpawner>>`.
- `cogito-protocol`: `SessionMeta` gains `parent_session_id`,
  `parent_call_id`, and `subagent_depth` (linkage recorded child-side only).
- `cogito-protocol`: `StreamEvent` gains `subagent_call_id: Option<String>`
  on `TurnStarted` / `TurnCompleted` / `TurnFailed` / `TextDelta`, so a
  child's events can be attributed to the originating `delegate` call on the
  parent's broadcast.
- `cogito-core`: `DelegateToolProvider` (`AlwaysSync`) plus the
  `DELEGATE_TOOL_NAME` and `DEFAULT_MAX_SUBAGENT_DEPTH` (= 3) constants;
  `RuntimeBuilder::strategy_registry(...)` setter for role-to-strategy
  resolution. `Runtime` implements `BrainSpawner` via a `RuntimeSpawner`
  newtype.
- `cogito-config`: `ToolsConfig.max_subagent_depth: Option<u32>` (default 3).

**Notes**

- All additions are **additive** — no `ConversationEvent` `SCHEMA_VERSION`
  bump (ADR-0007 / 0019). The new `SessionMeta` fields are default-skipped,
  so top-level JSONL stays byte-identical. The JSONL store layout remains
  flat (`<root>/<session_id>.jsonl`).
- The v0.3 full `BrainSpawner` lifecycle (spawn / wait / send_input / cancel
  + parent-child event tree) remains future work (ADR-0011 v0.3 amendment).

## [0.1.0] - 2026-05-29

First tagged release — **v0.1 Foundation**. A production-grade, embeddable
Agent Runtime core (the "Harness") that drives a full event-sourced turn
against Anthropic / OpenAI Responses / OpenAI-compatible backends with an
FSM Turn Driver (H01), immediate-write Step Recorder (H02), chaos-tested
crash resume (H03), context management (H11 / ADR-0008), a hook pipeline
(H09), skills, async jobs, MCP sync tools, multi-model strategies, and
CLI + TUI surfaces. Brain / Hands / Session layering per ADR-0004;
`ConversationEvent` schema_version = 1. See the per-sprint entries below
for detail.

### Sprint 10 · v0.1 hardening (2026-05-29)

**Changed**

- **Renamed crate `cogito-store-jsonl` → `cogito-store`** (ADR-0024).
  The crate is now named after its Session-layer role, not a backend.
  The JSONL implementation moved into module `cogito_store::jsonl`,
  gated by the default Cargo feature `jsonl`; `JsonlStore` is re-exported
  at the crate root, so consumers change `use cogito_store_jsonl::JsonlStore`
  to `use cogito_store::JsonlStore`. Future backends (`postgres` in v0.4,
  `sqlite` later) land as additional feature-gated modules instead of new
  workspace crates — the planned v0.4 `cogito-store-postgres` crate is now
  `cogito-store --features postgres`. No on-disk format or schema change.

**Docs**

- Reconciled the component design docs (H01–H11) with shipped reality:
  removed stale "Sprint 2 pass-through" / "not yet shipped" / "Sprint 3
  P2.5 will unify" framing for work that is now complete (real H11
  context orchestration in Sprint 6, the `ModelCallCompleted` recorder
  and `Result<EventId, StoreError>` signatures in Sprint 3, tool-filter
  override in Sprint 6). Refreshed the per-component Status banners and
  removed decorative status glyphs.
- Reconciled ROADMAP Sprint 4 (MCP) status with the merged code: the
  five completed items are marked done; the live-server happy-path
  integration test is documented as a tracked deferral (failure-path /
  soft-skip coverage shipped instead).

### Sprint 9b · TUI (2026-05-28)

**Added**

- `cogito-tui` lifted from stub to working multi-pane ratatui surface.
  Chat scrollback on the left, per-turn tool-call tree on the right,
  bottom status bar, multi-line input with `Shift+Enter` newline, slash
  command discovery popup, `Ctrl-T` to toggle tools pane,
  `Ctrl-C`/`Ctrl-D` cancel/exit. Full flag parity with `cogito chat`
  including `--strategy`, `--list-strategies`, `--session-id`,
  `--mode resume`. Spec: `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md`;
  ROADMAP entry: Sprint 9b.
- `tui-textarea` and `tracing-appender` added to workspace dependencies
  (transitive consumers: `cogito-tui` only).
- `assert_cmd` promoted from `cogito-cli` dev-dep to workspace dep.
- `cogito-core` gains a `test-support` feature exposing
  `SessionHandle::test_stub()` for pure-state Surface tests.

**Changed**

- Workspace layout table (AGENTS.md, ARCHITECTURE.md, CLAUDE.md):
  `cogito-tui` row's "When" column updated from v0.2 to v0.1.

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

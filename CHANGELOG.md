# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- `cogito-core::runtime::SessionHandle::send_user` is now a 1-line
  shim around `submit(TurnTrigger::UserText(text.into()))`. Behavior
  preserved; existing call sites (`cogito-cli`, integration tests,
  chaos tests) unchanged.
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

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

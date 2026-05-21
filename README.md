# cogito

> *"Cogito, ergo sum."* — Descartes
>
> A production-grade **Agent Runtime core**, packaged as an embeddable
> Rust library.

cogito is the *thinking part* of an agent — the orchestration core that
drives one iteration of the agent loop. Consumer Rust services depend on
it and run it in-process to gain agent-loop capability inside their
product. cogito decides; it does not deploy, serve traffic, authenticate
users, or render UI — those are the consumer's responsibility (or a
future SaaS layer wrapping cogito).

## What's inside

- **Brain** — 11-component Harness (`H01` Turn Driver … `H11` Context
  Manage) implemented as an explicit FSM, the only coordinator inside
  Brain
- **Session** — event-sourced `ConversationStore` trait + a v0.1 JSONL
  backend; every state transition is persisted before it happens, so any
  Brain instance can resume any session
- **Boundary** — `ModelGateway` trait with streaming Anthropic Messages
  and OpenAI-Compat (vLLM / SGLang / Azure / private gateways) adapters
- **Hands** — `ToolProvider` / `JobManager` / `HookHandler` /
  `StorageSystem` / `BrainSpawner` traits; v0.1 ships a built-in
  `read_file` tool and a composable provider
- **Runtime** — dependency injection, panic isolation, per-session actor;
  `Runtime::open_session` → `SessionHandle::{send_user, cancel_turn,
  shutdown, subscribe}`
- **Surface** — `cogito-cli chat` runs an end-to-end loop against
  Anthropic or any OpenAI-compatible endpoint

Brain may import **only** `cogito-protocol`. Hand crates are wired in by
the Runtime layer and injected as trait objects (ADR-0004). This is a
build-enforced rule, not a convention.

## Status

**v0.1 · Foundation** — Sprints 0-3 complete (event-sourced JSONL store,
H01 FSM Turn Driver, Anthropic + OpenAI-compat gateways, H03 Resume
Coordinator with chaos tests). Sprint 4 (Async Jobs) up next. See
`ROADMAP.md` for the full version-driven plan toward `v1.0` GA.

## Quick start

### 1. Install prerequisites

```bash
# Rust 1.85+ (edition 2024 — see rust-toolchain.toml)
# Install via https://rustup.rs if you don't already have it.
rustc --version

# Optional but recommended (Makefile auto-detects nextest):
cargo install cargo-nextest
```

### 2. Configure credentials

```bash
cp .env.example .env
# Edit .env: set ANTHROPIC_API_KEY, or OPENAI_BASE_URL + OPENAI_API_KEY
# for vLLM / SGLang / SenseNova / Azure-OpenAI / private gateways.
```

`.env` is git-ignored. `make` auto-loads it; the `just` recipes do not.

### 3. Build, test, chat

The project ships **two equivalent task runners**:

- `Makefile` — full feature surface (`.env` auto-load, `make help`,
  per-provider chat targets, env sanity-check). **Recommended for
  day-to-day local dev.**
- `justfile` — minimal recipes, no `.env` auto-load. **Recommended in
  CI and inside `just ci` (the canonical gate).**

```bash
make help               # discover every target with a one-line summary
make env-check          # print resolved env vars (no secrets) for debugging
make test               # cargo nextest run --workspace
make test CRATE=cogito-core
make ci                 # full local gate: fmt + clippy + layer-check + test
make chat               # interactive REPL against the OpenAI-compat endpoint
make chat-anthropic     # interactive REPL against Anthropic
```

Equivalent `just` calls:

```bash
just test cogito-core   # positional crate arg (NOT -p)
just ci                 # canonical CI gate (matches GitHub Actions)
just chat               # uses CLI defaults; for credentials prefer make chat
```

### 4. Debugging

- `make env-check` — verify which credentials and defaults are active
  without leaking secrets.
- `RUST_LOG=cogito=debug make chat` — verbose harness tracing.
- `just inspect <session_id>` — dump a session's event log (JSONL).
- `just replay <session_id>` — re-play a session from its event log.
- Per-session JSONL files land under `./sessions/` by default; remove
  them with `make sessions-clean`.

## Documentation

- `AGENTS.md` — operating manual for AI coding agents (read first)
- `ARCHITECTURE.md` — the 11-component Brain, FSM, and layer map
- `ROADMAP.md` — current sprint and version plan
- `CHANGELOG.md` — what shipped, per version
- `docs/components/H0X-*.md` — per-component design notes
- `docs/adr/` — architecture decision records (ADR-0001 … ADR-0007)
- `docs/data-model/` — event-log JSONL v1 specification
- `docs/schemas/` — JSON Schema artifacts (CI drift gate)
- `docs/experiments/` — sprint-by-sprint experiment reports

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).

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

**v0.1 · Foundation** — Sprint 2 (Minimal Loop) complete; Sprint 3
(H03 Resume Coordinator + chaos tests) up next. See `ROADMAP.md` for
the full version-driven plan toward `v1.0` GA.

## Quick start

```bash
# Prerequisites: Rust 1.85+ (edition 2024 — see rust-toolchain.toml)
cargo install just cargo-nextest

just ci         # fmt-check + clippy + layer-check + test
just test       # cargo nextest run
just chat       # interactive REPL against Anthropic or OpenAI-Compat
```

Set `ANTHROPIC_API_KEY` (or `OPENAI_API_KEY` + base URL for
vLLM / SGLang / SenseNova) and `just chat` gives you a working agent
loop with the `read_file` tool.

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

MIT OR Apache-2.0

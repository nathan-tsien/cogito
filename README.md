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
- **Session** — event-sourced `ConversationStore` trait + a JSONL
  backend; every state transition is persisted before it happens, so any
  Brain instance can resume any session
- **Boundary** — `ModelGateway` trait with streaming Anthropic Messages
  and OpenAI-Compat (vLLM / SGLang / Azure / private gateways) adapters
- **Hands** — `ToolProvider` / `JobManager` / `HookHandler` /
  `StorageSystem` / `BrainSpawner` traits; built-in file/search tools
  (`read_file`, `write_file`, `edit`, `grep`, `glob`, `bash`) plus MCP,
  plugin, and subagent providers, composed at the Hands layer
- **Runtime** — dependency injection, panic isolation, per-session actor;
  `Runtime::open_session` → `SessionHandle::{submit, submit_user_text,
  cancel_turn, shutdown, subscribe}`
- **Surface** — `cogito-cli chat` and a `cogito-tui` terminal UI run an
  end-to-end loop against Anthropic or any OpenAI-compatible endpoint

Brain may import **only** `cogito-protocol`. Hand crates are wired in by
the Runtime layer and injected as trait objects (ADR-0004). This is a
build-enforced rule, not a convention.

## Status

**v0.2 · Extensibility** — current release (`v0.2.0`). Builds on the v0.1
foundation (event-sourced JSONL store, H01 FSM Turn Driver, Anthropic +
OpenAI-compat gateways, H03 Resume Coordinator with chaos tests) with
subagents, a local-path plugin loader (Skills + MCP), per-session provider
injection, a workspace seam, and a built-in file/search tool catalog.

See `ROADMAP.md` for the version-driven plan toward `v1.0` GA and
`CHANGELOG.md` for what shipped.

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

`.env` is git-ignored; `make` auto-loads it.

### 3. Build, test, chat

All local dev flows through `Makefile`. `.env` is auto-loaded if
present; `make help` lists every target with a one-line summary.

```bash
make help               # discover every target with a one-line summary
make env-check          # print resolved env vars (no secrets) for debugging
make test               # cargo nextest run --workspace
make test CRATE=cogito-core
make ci                 # full local gate: fmt-check + clippy + layer-check + test
make chat               # interactive REPL (provider/model from cogito.toml)
```

Provider, model, and MCP server selection live in `cogito.toml` (search
path: `$COGITO_CONFIG`, `./cogito.toml`, `$XDG_CONFIG_HOME/cogito/config.toml`).
For a one-off override, invoke the CLI directly:
`cargo run -p cogito-cli -- chat --model X --provider Y`.

### 4. Debugging

- `make env-check` — verify which credentials and defaults are active
  without leaking secrets.
- `RUST_LOG=cogito=debug make chat` — verbose harness tracing.
- Per-session JSONL files land under the configured `runtime.session_root`
  (default `./sessions`); remove them with
  `make sessions-clean SESSION_ROOT=<path>`.

### 5. MCP servers

`cogito chat` can mount any number of MCP (Model Context Protocol)
servers via the `[[mcp_servers]]` array in `cogito.toml`. Both stdio
and streamable-HTTP transports are supported.

```toml
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "uvx"
args = ["mcp-server-filesystem", "/tmp"]

[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"
```

Their tools surface as `mcp__<server>__<tool>` in the model's tool
catalog. MCP failures (missing binary, env var unset, handshake
timeout, ...) are **never fatal**: cogito prints a per-server status
banner on stderr at startup and continues with whatever tools came
up.

See [ADR-0018](./docs/adr/0018-mcp-integration.md) for the
architectural contract.

## Documentation

- `AGENTS.md` — operating manual for AI coding agents (read first)
- `ARCHITECTURE.md` — the 11-component Brain, FSM, and layer map
- `ROADMAP.md` — version-driven plan toward `v1.0` GA
- `CHANGELOG.md` — what shipped, per version
- `docs/components/H0X-*.md` — per-component design notes
- `docs/adr/` — architecture decision records
- `docs/data-model/` — event-log JSONL v1 specification
- `docs/schemas/` — JSON Schema artifacts (CI drift gate)
- `docs/experiments/` — experiment reports

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).

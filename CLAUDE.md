# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative docs — read these first

This repo already has detailed agent-facing documentation. Before making changes, consult:

- **`AGENTS.md`** — the operating manual for AI coding agents. Inviolable design principles, coding standards, what to do when finishing a task, what's not OK. Read every time.
- **`ARCHITECTURE.md`** — the 11-component Harness design (H01-H11), dependency constraints, workspace layout, turn states.
- **`ROADMAP.md`** — the current sprint. **Only work on the current sprint** unless explicitly directed otherwise.
- **`docs/components/H0X-*.md`** — per-component design notes. Read the doc for any component you're touching.
- **`docs/adr/`** — architecture decision records.
- **`docs/configuration/overview.md`** — holistic map of the configuration story (sections, sources, merge, secret handling, crate layout). Start here when anything related to config / `cogito.toml` / `RuntimeConfig` / provider selection comes up.

If `AGENTS.md` and this file conflict, `AGENTS.md` wins.

## What this project is

**cogito** is a **production-grade Agent Runtime core, packaged as an embeddable Rust library**. Consumer Rust services depend on it and run it in-process to gain agent-loop capability inside their product.

- **Harness = the thinking part** of an agent (orchestration core that drives one turn), separated from execution (hands) and memory (session).
- Naming convention: `cogito` = "thinking"; the runtime never *acts* directly, it decides.
- Version-driven roadmap: v0.1 foundation → v0.2 storage/multimodal → v0.3 subagent → v0.4 SaaS-ready → ... → v1.0 GA. See `ADR-0005` for production scope and quality gates.

## Inviolable design rules (summary; see AGENTS.md §"Inviolable design principles")

1. **H01 Turn Driver is the only coordinator.** H02–H11 never call each other. Calling H05 from H04 is a bug, not a shortcut.
2. **H02 Step Recorder writes events immediately.** No batching, except `text_delta` events (≤200ms or ≤500 chars, then flush).
3. **State lives in Conversation Service, not Harness memory.** If a Harness instance crashes mid-turn, a new instance must resume from the event log. No cross-turn state in structs. Ask: "can this be rebuilt from the event log?"
4. **Turn Driver is a state machine**, not a function chain. States: `Init → PromptBuilt → ModelCalling → ModelCompleted → ToolDispatching → {Completed | Paused | Failed}`. Each transition writes an event *before* transitioning.
5. **Tool failures are structured `ToolResult::Error`**, not panics or `unwrap`s.
6. **Brain only sees Hands / Session / Boundary through Protocol traits.** `cogito-core::harness` may import **only** `cogito-protocol`. Concrete crates (`cogito-store-jsonl`, `cogito-model`, `cogito-tools`, `cogito-sandbox`, `cogito-jobs`, `cogito-mcp`, `cogito-subagent`, `cogito-storage-local`) are wired in by the Runtime layer and injected as trait objects. If you want to `use cogito_tools::…` inside `harness/`, add a trait to `cogito-protocol` instead. Hooks (H09) follow the same rule: pure policy gates, no I/O — side effects go through `ToolProvider`/`JobManager`. See **ADR-0004** for the full layer map.

## Commands

Use `just` recipes — don't invent your own:

```bash
just fmt                 # cargo fmt --all
just fix [crate]         # clippy --fix + fmt (optionally scoped)
just test [crate]        # cargo nextest run (optionally scoped)
just bench               # criterion benchmarks
just chaos               # resume_chaos tests (slow, release mode)
just ci                  # fmt-check + clippy + test (the CI gate)
just chat                # cogito-cli chat  (available from Sprint 2)
just inspect <session>   # dump a session's event log
just replay <session>    # replay a session
```

Prereqs: Rust 1.85+ (rustup; edition 2024 requirement — see `rust-toolchain.toml`), `cargo install just cargo-nextest`.

When finishing a task:
1. `just fmt && just fix <crate>` → clean
2. `just test -p <crate>` → green
3. Update `docs/components/H0X-*.md` if component behavior changed
4. If a sprint goal was completed, write/update the experiment report under `docs/experiments/`

**Patience**: cargo commands can be slow due to lock-file resolution. Don't kill them by PID — that corrupts the lock file.

## Workspace layout

Each crate maps to exactly one layer in the Brain / Hands / Session design (ADR-0004). v0.1 crates listed; later crates (storage, subagent, multimedia, SaaS-ready stores) land in subsequent versions per ARCHITECTURE.md §"Version evolution path".

| Crate | Layer | When | Role |
|---|---|---|---|
| `cogito-protocol` | Protocol | v0.1 | All traits + events + `Vec<ContentBlock>` + value types. No internal deps. |
| `cogito-core` | Brain + Runtime (will split) | v0.1 | `harness/` is Brain (H01–H10), may only `use cogito_protocol::*`; `runtime/` hosts Brain + implements `BrainSpawner` (v0.3+). |
| `cogito-store-jsonl` | Session | v0.1 | Per-session JSONL backend; sole v0.1 store. |
| `cogito-store-postgres` | Session | v0.4 | Production multi-replica backend. |
| `cogito-model` | Boundary | v0.1 | `ModelGateway` impls (Anthropic + OpenAI) with ContentBlock serialization. |
| `cogito-tools` | Hands | v0.1 | Builtin tools + `CompositeToolProvider` utility. |
| `cogito-sandbox` | Hands (internal primitive) | v0.1 | Subprocess sandbox; not visible to Brain. |
| `cogito-jobs` | Hands | v0.1 | `JobManager` impl. |
| `cogito-mcp` | Hands | v0.2 | MCP client; another `ToolProvider`. |
| `cogito-storage-local` | Hands (Storage) | v0.2 | Local FS / HTTP-cache / `blob://` backend. |
| `cogito-tools-multimedia` | Hands | v0.2+ | Audio / video / image tools. |
| `cogito-subagent` | Hands | v0.3 | `SubagentToolProvider` with 4 tools. |
| `cogito-cli` | Surface | v0.1 | CLI entry point. |
| `cogito-tui` | Surface | v0.2 | TUI. |
| `crates/testing/cogito-test-fixtures` | Testing | v0.1 | Test fixtures. |
| `crates/testing/cogito-mock-model` | Testing | v0.1 | Mock `ModelGateway`. |

**Brain importing a Hand directly is a build error.** Don't bloat `cogito-core` either: if something could live in `cogito-protocol` or another crate, put it there. (Codex learned this the hard way.) Adding a new crate requires explicit approval.

## Coding standards (workspace-wide)

- Rust 2024 edition, MSRV 1.85 (edition 2024 stabilized in 1.85)
- `unsafe_code = "forbid"`, `missing_docs = "warn"` at workspace level
- Clippy: `pedantic` (warn) plus `unwrap_used`, `expect_used`, `panic`, `dbg_macro` all **deny**
- `print_stdout` / `print_stderr` warn — use `tracing` instead
- Errors: `thiserror` for libraries, `anyhow` for binaries
- All workspace deps go through `[workspace.dependencies]`; members declare `{ workspace = true }`
- `RUSTFLAGS=-Dwarnings` via `.cargo/config.toml` — warnings break the build
- **All code comments (doc comments `///`, module docs `//!`, inline `//`) are written in English.** Chinese is reserved for design docs (`docs/superpowers/specs/`), ADRs, commit messages, and human-facing conversation. Rationale: code is read by future maintainers and AI agents who default to English; mixing languages in source hurts grep and review.
- **No decorative numerals or markers in comments / prose.** Don't use `①②③…`, `★`, `►`, ✓, ✗, ⚠ etc. as step markers, bullets, or emphasis — neither in source comments nor in user-facing docs (`README.md`, `ARCHITECTURE.md`, `docs/components/`, `docs/adr/`). Use plain `1.` / `2.` for numbered steps and `-` for bullets. Rationale: these glyphs break `grep`, render inconsistently across editors / terminals, and add no information that an ASCII number doesn't. Box-drawing characters (`┌─┐│▼►`) inside fenced ASCII-art diagrams are exempt — they carry layout, not decoration.
- **Tagged-config factories belong in the crate that owns the implementations.** When a configuration value selects one of N concrete trait objects (e.g., `ProviderConfig` → `Arc<dyn ModelGateway>`), put the `match`-on-tag dispatch in the crate that owns the trait impls (e.g., `cogito_model::build_gateway`), **not** in Surface crates (`cogito-cli`, `cogito-tui`, consumer's Server code). Surface code calls one function and receives the trait object. Rationale: adding a new variant (e.g., Sprint 6 `OpenAiResponses`) should require editing only the owning crate; surfaces stay untouched, the `serde(tag = "kind")` enum surfaces config errors at deserialization time, and the dispatch table doesn't fork across surfaces. Applies to: `ModelGateway` factory (`cogito-model`), and any future config-driven trait-object factories (`ToolProvider`, plugin, subagent).

## Testing requirements

- Every component has a unit test module.
- Every contract (trait) has a contract test that all implementations must pass (e.g., SQLite and in-memory conversation stores must agree).
- Integration tests live in `crates/*/tests/`.
- Resume chaos tests: `crates/cogito-core/tests/resume_chaos.rs` (run via `just chaos`).
- New features require new tests. **Never `#[ignore]` a test to make it pass.**

## When uncertain

Valid: ask the human, write the simplest thing and flag for review, add `// TODO: design decision needed`, or write an ADR in `docs/adr/` proposing a decision.

Not valid: inventing architecture, adding crates without asking, skipping tests because "this is experimental", importing an Agent framework, marking tests `#[ignore]` to dodge failures.

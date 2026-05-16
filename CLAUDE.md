# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative docs — read these first

This repo already has detailed agent-facing documentation. Before making changes, consult:

- **`AGENTS.md`** — the operating manual for AI coding agents. Inviolable design principles, coding standards, what to do when finishing a task, what's not OK. Read every time.
- **`ARCHITECTURE.md`** — the 10-component Harness design, dependency constraints, workspace layout, turn states.
- **`ROADMAP.md`** — the current sprint. **Only work on the current sprint** unless explicitly directed otherwise.
- **`docs/components/H0X-*.md`** — per-component design notes. Read the doc for any component you're touching.
- **`docs/adr/`** — architecture decision records.

If `AGENTS.md` and this file conflict, `AGENTS.md` wins.

## What this project is

**cogito** is an experimental Rust workspace that validates a 10-component Harness design for AI agent systems. It is *not* a product; it is a controlled experiment to verify the design before building a production agent platform.

- **Harness = the thinking part** of an agent (orchestration core that drives one turn), separated from execution (hands) and memory (session).
- Naming convention: `cogito` = "thinking"; the runtime never *acts* directly, it decides.
- Do **not** mark anything "production-ready" — this is an experiment.

## Inviolable design rules (summary; see AGENTS.md §"Inviolable design principles")

1. **H01 Turn Driver is the only coordinator.** H02–H10 never call each other. Calling H05 from H04 is a bug, not a shortcut.
2. **H02 Step Recorder writes events immediately.** No batching, except `text_delta` events (≤200ms or ≤500 chars, then flush).
3. **State lives in Conversation Service, not Harness memory.** If a Harness instance crashes mid-turn, a new instance must resume from the event log. No cross-turn state in structs. Ask: "can this be rebuilt from the event log?"
4. **Turn Driver is a state machine**, not a function chain. States: `Init → PromptBuilt → ModelCalling → ModelCompleted → ToolDispatching → {Completed | Paused | Failed}`. Each transition writes an event *before* transitioning.
5. **Tool failures are structured `ToolResult::Error`**, not panics or `unwrap`s.
6. **Brain only sees Hands / Session / Boundary through Protocol traits.** `cogito-core::harness` may import **only** `cogito-protocol`. Concrete crates (`cogito-conversation`, `cogito-model`, `cogito-tools`, `cogito-sandbox`, `cogito-jobs`, `cogito-mcp`) are wired in by the Runtime layer and injected as trait objects. If you want to `use cogito_tools::…` inside `harness/`, add a trait to `cogito-protocol` instead. Hooks (H09) follow the same rule: pure policy gates, no I/O — side effects go through `ToolProvider`/`JobManager`. See **ADR-0004** for the full layer map.

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

Prereqs: Rust 1.83+ (rustup), `cargo install just cargo-nextest`.

When finishing a task:
1. `just fmt && just fix <crate>` → clean
2. `just test -p <crate>` → green
3. Update `docs/components/H0X-*.md` if component behavior changed
4. If a sprint goal was completed, write/update the experiment report under `docs/experiments/`

**Patience**: cargo commands can be slow due to lock-file resolution. Don't kill them by PID — that corrupts the lock file.

## Workspace layout

Each crate maps to exactly one layer in the Brain / Hands / Session design (ADR-0004):

| Crate | Layer | Role |
|---|---|---|
| `cogito-protocol` | Protocol | Events, traits, shared types. No internal cogito deps. |
| `cogito-core` | Brain + Runtime (will split) | `harness/` is Brain (H01–H10), may only `use cogito_protocol::*`; `runtime/` hosts Brain. |
| `cogito-conversation` | Session | Event log; implements `ConversationStore` |
| `cogito-model` | Boundary | Model Gateway; implements `ModelGateway` |
| `cogito-tools` | Hands | Builtin tools; implements `ToolProvider` |
| `cogito-sandbox` | Hands | Subprocess sandbox; implements `Sandbox` |
| `cogito-jobs` | Hands | Async jobs; implements `JobManager` |
| `cogito-mcp` | Hands | MCP client; another `ToolProvider` (Sprint 5+) |
| `cogito-cli` | Surface | CLI entry point |
| `cogito-tui` | Surface | TUI (Sprint 6+) |
| `testing/cogito-test-fixtures` | Testing | Test fixtures |
| `testing/cogito-mock-model` | Testing | Mock `ModelGateway` |

**Brain importing a Hand directly is a build error.** Don't bloat `cogito-core` either: if something could live in `cogito-protocol` or another crate, put it there. (Codex learned this the hard way.) Adding a new crate requires explicit approval.

## Coding standards (workspace-wide)

- Rust 2024 edition, MSRV 1.83
- `unsafe_code = "forbid"`, `missing_docs = "warn"` at workspace level
- Clippy: `pedantic` (warn) plus `unwrap_used`, `expect_used`, `panic`, `dbg_macro` all **deny**
- `print_stdout` / `print_stderr` warn — use `tracing` instead
- Errors: `thiserror` for libraries, `anyhow` for binaries
- All workspace deps go through `[workspace.dependencies]`; members declare `{ workspace = true }`
- `RUSTFLAGS=-Dwarnings` via `.cargo/config.toml` — warnings break the build

## Testing requirements

- Every component has a unit test module.
- Every contract (trait) has a contract test that all implementations must pass (e.g., SQLite and in-memory conversation stores must agree).
- Integration tests live in `crates/*/tests/`.
- Resume chaos tests: `crates/cogito-core/tests/resume_chaos.rs` (run via `just chaos`).
- New features require new tests. **Never `#[ignore]` a test to make it pass.**

## When uncertain

Valid: ask the human, write the simplest thing and flag for review, add `// TODO: design decision needed`, or write an ADR in `docs/adr/` proposing a decision.

Not valid: inventing architecture, adding crates without asking, skipping tests because "this is experimental", importing an Agent framework, marking tests `#[ignore]` to dodge failures.

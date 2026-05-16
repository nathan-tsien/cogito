# AGENTS.md

This file is the **operating manual for AI coding agents** working on this codebase.
Read this first, every time, before making changes.

## What this project is

**cogito** is an experimental Rust project that validates a 10-component
Harness design for AI agent systems.

It is **not** a production agent. It is a controlled experiment.

The goal: validate that the Harness design works in practice before
building the production Agent Platform.

See `ARCHITECTURE.md` for the full design.
See `ROADMAP.md` for the current sprint and what to focus on.

## Inviolable design principles

These cannot be violated. If you find yourself wanting to violate one, **stop and ask**.

### 1. H01 Turn Driver is the only coordinator inside Harness

Components H02–H10 do not call each other. They are called by H01.
Calling H05 from H04 is a bug, not a shortcut.

### 2. H02 Step Recorder writes events immediately

No batching. No buffering across components. The only exception is
`text_delta` events, which may be batched for ≤200ms or ≤500 chars,
then flushed.

### 3. State lives in Conversation Service, not in Harness memory

If a Harness instance crashes mid-turn, a new instance must be able
to resume by reading the event log. Do not add fields to structs
that hold cross-turn state. If you find yourself wanting to cache,
ask: "can this be rebuilt from the event log?"

### 4. Turn Driver is a state machine, not a function chain

States: `Init`, `PromptBuilt`, `ModelCalling`, `ModelCompleted`,
`ToolDispatching`, `TurnCompleted`, `TurnPaused`, `TurnFailed`.

Each transition writes an event *before* transitioning.

### 5. Tool failures are structured errors, not panics

When a tool call fails (bad args, schema mismatch, runtime error),
return a `ToolResult::Error` with an LLM-readable message.
Do not propagate panics or `unwrap`s to the Harness layer.

### 6. Brain may only see Hands / Session / Boundary through Protocol

`cogito-core::harness` is Brain. It may import **only** `cogito-protocol`.
Concrete crates (`cogito-conversation`, `cogito-model`, `cogito-tools`,
`cogito-sandbox`, `cogito-jobs`, `cogito-mcp`) are imported by the
Runtime layer and injected into Brain as trait objects.

If you find yourself wanting to write `use cogito_tools::…` inside
`harness/`, the answer is to **add a trait to `cogito-protocol`**, not
to relax the import. The same rule applies to hooks: a `HookHandler`
does not get to do I/O — if it needs to, it goes through a
`ToolProvider` or `JobManager` like any other Hand.

See ADR-0004 for the full layer map and import rules. See
`docs/components/H09-hook-pipeline.md` for the hook purity rule.

## Coding standards

- **Edition**: Rust 2024, MSRV 1.83
- **Lints**: `#![warn(clippy::pedantic)]` at crate root
- **Forbidden**: `unwrap_used`, `expect_used`, `panic` (denied by clippy)
- **Errors**: `thiserror` for libraries, `anyhow` for binaries
- **Unsafe**: forbidden (`unsafe_code = "forbid"` in workspace lints)
- **Docs**: all public items have doc comments

## Testing requirements

- Every component has a unit test module
- Every contract (trait) has a contract test that all implementations pass
- Integration tests live in `crates/*/tests/`
- Chaos tests for resume correctness live in `crates/cogito-core/tests/resume_chaos.rs`
- **New features require new tests. No exceptions.**

## Workspace rules

- Add a new dependency only after checking it's not already declared in workspace `Cargo.toml`
- Always use `{ workspace = true }` in member crates
- **Don't add to `cogito-core` what could live elsewhere.** If you can't decide,
  ask: "would this make sense in `cogito-protocol` or a new crate?"
  (Codex learned this lesson the hard way. We're starting clean.)

## Commands you should know

Run these instead of inventing your own:

```bash
just fmt              # rustfmt
just fix              # clippy --fix + fmt (add a crate name to scope)
just test             # nextest, faster than cargo test
just bench            # criterion benchmarks
just chaos            # run chaos tests (slow)
just ci               # the CI gate locally
```

## What to do when you finish a task

1. `just fmt && just fix <crate>` — ensure clean
2. `just test -p <crate-you-touched>` — verify tests pass
3. Update `docs/components/H0X-*.md` if you changed component behavior
4. If you completed a sprint goal, write/update the experiment report
   in `docs/experiments/`
5. **Do NOT mark anything as "production-ready".** This is an experiment.

## When you're uncertain

These are all valid moves:

- Ask the human to clarify which design principle applies
- Write the simplest thing that works and flag it for review
- Add a `// TODO: design decision needed` comment
- Stop and write a design note in `docs/adr/` proposing a decision

What's **not** OK:

- Inventing your own architecture
- Adding new crates without asking
- Skipping tests because "this is just experimental"
- Importing a framework that violates the "no Agent framework" rule
- Marking tests as `#[ignore]` to make them pass

## Patience note

When running `cargo` commands (`just fix`, `cargo test`), be patient.
Rust lock-file resolution can be slow. Don't kill the command by PID —
that corrupts the lock file. Wait it out.

## Current sprint

See `ROADMAP.md`. The current sprint is the only thing you should be
working on unless explicitly directed otherwise.

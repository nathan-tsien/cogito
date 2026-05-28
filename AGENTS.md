# AGENTS.md

This file is the **operating manual for AI coding agents** working on this codebase.
Read this first, every time, before making changes.

## What this project is

**cogito** is a **production-grade Agent Runtime core, packaged as an embeddable Rust library.**
Consumer Rust services depend on it and run it in-process to gain
agent-loop capability inside their product.

cogito provides:

- **Brain**: 11-component Harness (H01–H11; H11 Context Manage slot reserved 2026-05-19, mechanism pending ADR-0008)
- **Session**: event-sourced `ConversationStore` trait + reference backend
- **Hands / Boundary**: trait surface for tools, model gateway, jobs, hooks, storage
- **Subagent (v0.3+)**: recursive Brain via `BrainSpawner`

cogito does NOT provide deployment artifacts, inbound transport, authentication,
multi-tenant isolation enforcement, or Web UI — those are the consumer's
responsibility (or a future SaaS layer wrapping cogito).

See `ARCHITECTURE.md` for the full design, `ROADMAP.md` for the current
version and sprint, and `ADR-0005` for production scope + quality gates.

## Authoritative docs — read these first

- `ARCHITECTURE.md` — the 11-component Harness design, dependency constraints, workspace layout.
- `ROADMAP.md` — the current sprint and version evolution path.
- `docs/components/H0X-*.md` — per-component design notes.
- `docs/adr/` — architecture decision records.
- `docs/configuration/overview.md` — holistic map of the configuration story (sections, sources, merge, secret handling, crate layout).
- `docs/adr/0026-strategy-registry.md` — what strategies are, why
  cogito-strategy is a separate crate from cogito-config.

## Inviolable design principles

These cannot be violated. If you find yourself wanting to violate one, **stop and ask**.

### 1. H01 Turn Driver is the only coordinator inside Harness

Components H02–H11 do not call each other. They are called by H01.
Calling H05 from H04 is a bug, not a shortcut.

### 2. H02 Step Recorder writes events immediately

No batching. No buffering across components. `StreamEvent::TextDelta`
is live-only (never persisted by H02). Persistence happens at the
wire-protocol content_block boundary: when the demultiplexer signals
`text_block_complete`, H02 writes one `AssistantMessageAppended`
carrying the full block text. This matches Codex and Claude Code,
both of which align persistence with content_block boundaries. No
timer-based or size-based batching exists.

### 3. State lives in Conversation Service, not in Harness memory

If a Harness instance crashes mid-turn, a new instance must be able
to resume by reading the event log. Do not add fields to structs
that hold cross-turn state. If you find yourself wanting to cache,
ask: "can this be rebuilt from the event log?"

### 4. Turn Driver is a state machine, not a function chain

States: `Init`, `ContextManaged`, `PromptBuilt`, `ModelCalling`,
`ModelCompleted`, `ToolDispatching`, `TurnCompleted`, `TurnPaused`,
`TurnFailed`. (`ContextManaged` added 2026-05-19 by PR #6 as ADR-0006
amendment; v0.1 ships as pass-through, real H11 implementation pending
ADR-0008.)

Each transition writes an event *before* transitioning.

### 5. Tool failures are structured errors, not panics

When a tool call fails (bad args, schema mismatch, runtime error),
return a `ToolResult::Error` with an LLM-readable message.
Do not propagate panics or `unwrap`s to the Harness layer.

### 6. Brain may only see Hands / Session / Boundary through Protocol

`cogito-core::harness` is Brain. It may import **only** `cogito-protocol`.
Concrete crates (`cogito-store-jsonl`, `cogito-model`, `cogito-tools`,
`cogito-sandbox`, `cogito-jobs`, `cogito-mcp`) are imported by the
Runtime layer and injected into Brain as trait objects.

If you find yourself wanting to write `use cogito_tools::…` inside
`harness/`, the answer is to **add a trait to `cogito-protocol`**, not
to relax the import. The same rule applies to hooks: a `HookHandler`
does not get to do I/O — if it needs to, it goes through a
`ToolProvider` or `JobManager` like any other Hand.

See ADR-0004 for the full layer map and import rules. See
`docs/components/H09-hook-pipeline.md` for the hook purity rule.

### 7. `ConversationStore` is Brain's command + single-session replay trait

Methods on `ConversationStore` (`cogito-protocol::store`) MUST be
scoped to: (a) writing one event, (b) reading events for one
explicitly-named session. Adding any cross-session, cross-tenant, or
user-history query method to this trait is a design error.

Cross-session / catalog access for external (Go/Python/Node) services
is served by reading the underlying storage directly (JSONL files in
v0.1 dev/debug; Postgres tables in v0.4 production). See ADR-0007 for
the principle and ADR-0014 (v0.4) for the `TenantContext` model.

### 8. Thinking content ordering inside an assistant message

Within one assistant turn's `Message::Assistant.content` array,
`ContentBlock::Thinking` MUST precede `Text` and `ToolUse` blocks.
Brain enforces this by walking event-log entries in `seq` order in
H04 — providers emit `thinking` blocks first, so seq order produces
the correct ordering automatically. Reordering or dropping
`Thinking` blocks invalidates the next-turn signature check on
Anthropic and the reasoning-item continuity on OpenAI Responses.
Per ADR-0019 §4.

### 9. Persisted JSONL is append-only and never rewritten

cogito never rewrites already-persisted event-log files in place,
regardless of how the events were originally shaped. This applies
to backfilling, migration, normalization, and any other server-side
rewrite. Old sessions with provider-specific quirks (e.g.
`<think>…</think>` baked into `AssistantMessageAppended.text` from
pre-ADR-0019 builds) stay byte-for-byte as written. New shapes
coexist with old shapes in storage; readers handle both. The same
rule applies if a future ADR introduces yet another reasoning
representation: cogito appends forward, never rewrites backward.
Per ADR-0019 §5.3.

## Coding standards

- **Edition**: Rust 2024, MSRV 1.85 (edition 2024 stabilized in 1.85)
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
make fmt                       # rustfmt
make fix [CRATE=<name>]        # clippy --fix + fmt (add a crate to scope)
make test [CRATE=<name>]       # nextest, faster than cargo test
make bench                     # criterion benchmarks
make chaos                     # run chaos tests (slow)
make ci                        # the CI gate locally
```

## What to do when you finish a task

1. `make fmt && make fix CRATE=<crate>` — ensure clean
2. `make test CRATE=<crate-you-touched>` — verify tests pass
3. Update `docs/components/H0X-*.md` if you changed component behavior
4. Update `CHANGELOG.md` if you added a public-API change
5. If you completed a sprint or version milestone, update `ROADMAP.md`'s checklist

## When you're uncertain

These are all valid moves:

- Ask the human to clarify which design principle applies
- Write the simplest thing that works and flag it for review
- Add a `// TODO: design decision needed` comment
- Stop and write a design note in `docs/adr/` proposing a decision

What's **not** OK:

- Inventing your own architecture
- Adding new crates without asking
- Skipping tests because "we're early" or "this is just a sprint thing"
- Importing a framework that violates the "no Agent framework" rule
- Marking tests as `#[ignore]` to make them pass
- Marking work "production-ready" without the relevant ADR-0005 quality gate evidence

## Patience note

When running `cargo` commands (`make fix`, `cargo test`), be patient.
Rust lock-file resolution can be slow. Don't kill the command by PID —
that corrupts the lock file. Wait it out.

## Current version and sprint

See `ROADMAP.md`. The current version + sprint is the only thing you
should be working on unless explicitly directed otherwise. cogito is
version-driven (v0.1 → v0.2 → ... → v1.0); each version has a clear
theme and gate-able exit criteria in `ADR-0005`.

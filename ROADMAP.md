# Roadmap

## Current sprint

> **Sprint 0**: Project skeleton. No real functionality yet.
> Goal: empty crates compile, CI green, ARCHITECTURE.md reviewed.

## Sprint plan

### Sprint 0 · Skeleton (0.5 day)
- [x] AGENTS.md, ARCHITECTURE.md, ROADMAP.md written
- [ ] All 12 crates created with stub `lib.rs` / `main.rs`
- [ ] Workspace `Cargo.toml` builds clean
- [ ] CI workflow runs `just ci`
- [ ] `cargo test` passes (empty)

### Sprint 1 · H02 Step Recorder (1 day)
**Validates**: event log immediate-write performance, text-delta batching
- [ ] `cogito-protocol` defines `ConversationEvent`
- [ ] `cogito-conversation` SQLite + in-memory implementations
- [ ] Contract test ensures both implementations agree
- [ ] `cogito-core::harness::step_recorder` writes events
- [ ] Text-delta batching: 200ms or 500 chars
- [ ] Experiment E01: 10K events, measure latency P50/P99
- [ ] Report: `docs/experiments/E01-step-recorder-perf.md`

### Sprint 2 · Minimal Loop (2 days)
**Validates**: H01 + H04 + H06 + H07 with one tool
- [ ] `read_file` tool only
- [ ] Anthropic adapter in `cogito-model`
- [ ] CLI `cogito chat` works end-to-end
- [ ] Experiment E02: 20 test cases, measure tool error rate

### Sprint 3 · Resume Coordinator (2 days)
**Validates**: H03 + state machine correctness
- [ ] Turn Driver as explicit state machine
- [ ] Resume decision table fully implemented
- [ ] Chaos test injects crashes at every state transition
- [ ] Experiment E03: resume correctness 100%

### Sprint 4 · Async Jobs (2 days)
**Validates**: H08 sync/async mixed dispatch
- [ ] Job manager with persistence
- [ ] One real long task (`run_tests`)
- [ ] Loop pauses on async, resumes on completion
- [ ] Experiment E04: long task + new message during wait

### Sprint 5 · Multi-model Strategy (1 day)
**Validates**: H10 strategy selector
- [ ] OpenAI adapter in `cogito-model`
- [ ] `strategies/*.yaml` config files
- [ ] CLI `--model` flag works
- [ ] Experiment E05: same tasks, two models, compare

### Sprint 6 · Hook Pipeline + TUI (2 days)
**Validates**: H09 hook performance, basic TUI viability
- [ ] Two example hooks (sensitive content, bash audit)
- [ ] Basic TUI with ratatui (replicates `cogito chat`)
- [ ] Experiment E06: hook latency impact

### Sprint 7 · Integration Report (1 day)
- [ ] Cross-cutting analysis of all experiments
- [ ] Updated Harness design v1.1 in main docs
- [ ] Demo walkthrough

## What we're NOT doing (no matter how tempting)

- Web UI
- Multi-tenant
- Authentication
- Quotas
- Production deployment
- Skill autonomous creation (Hermes-style)
- Vector store / RAG
- Persistent memory across sessions

These belong in the production platform, not the validation experiment.

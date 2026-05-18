# Architecture Decision Records

We use Michael Nygard's ADR format. Each ADR is a short markdown file
recording an architectural decision and its context.

## Index

- [0001](./0001-rust-workspace-layout.md) — Rust workspace layout
- [0002](./0002-event-sourcing-conversation.md) — Event-sourced conversation log
- [0003](./0003-state-machine-turn-driver.md) — Turn Driver as explicit state machine
- [0004](./0004-brain-hands-session-boundaries.md) — Brain / Hands / Session crate boundaries
- [0005](./0005-production-scope-and-quality-gates.md) — Production scope, quality gates, SLO posture, compatibility commitments
- [0006](./0006-runtime-h01-execution-model.md) — Runtime + H01 Turn Driver execution model

## Template

```markdown
# ADR-XXXX: Title

## Status
Proposed | Accepted | Deprecated | Superseded by ADR-YYYY

## Context
What is the issue? What forces are at play?

## Decision
What did we decide?

## Consequences
What becomes easier? What becomes harder? What did we give up?
```

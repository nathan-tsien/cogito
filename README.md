# cogito

> *"Cogito, ergo sum."* — Descartes
>
> An experimental Rust project validating a 10-component Harness design
> for AI agent systems.

cogito is the *thinking part* of an agent — the orchestration core that
drives one iteration of the agent loop, separated from execution (hands)
and memory (session). This project exists to **validate that design**
before building a production agent platform.

## Status

🚧 Sprint 0 — Project skeleton

## Why "cogito"

The Harness is the agent's reasoning subsystem — the part that decides,
not the part that acts. Naming it `cogito` keeps that boundary clear:
this codebase is about *thinking*, not *doing*.

## Quick start

```bash
# Prerequisites
# - Rust 1.83+ (via rustup)
# - cargo install just cargo-nextest

just test       # run all tests
just chat       # interactive CLI (available from Sprint 2)
```

## Documentation

- `ARCHITECTURE.md` — overall design and the 10 components
- `AGENTS.md` — operating manual for AI coding agents
- `ROADMAP.md` — current sprint and what to do next
- `docs/components/` — per-component design notes
- `docs/adr/` — architecture decision records
- `docs/experiments/` — sprint-by-sprint experiment reports

## License

MIT OR Apache-2.0

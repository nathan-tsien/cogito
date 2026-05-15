# ADR-0001: Rust workspace layout

## Status

Accepted

## Context

We need to validate the Harness design before building production. The
implementation language must support:

- High concurrency (many sessions)
- Low-latency streaming
- Single-binary deployment
- Strong type safety for state machines

We're inspired by OpenAI Codex's Rust rewrite, which proved Rust is viable
for production agent systems.

## Decision

Use a Cargo workspace with **12 member crates** (vs Codex's ~70). The
smaller number reflects our experimental scope: we only build what is
needed to validate the design.

Workspace dependencies are pinned in the root `Cargo.toml` and inherited
via `{ workspace = true }`. Clippy lints are workspace-wide with
`unwrap_used`, `expect_used`, `panic` denied.

## Consequences

- **Easier**: shared dependency versions, consistent lints, fast clean
  rebuilds with targeted `-p` testing
- **Harder**: every new crate requires updating the workspace manifest
- **Given up**: we cannot publish individual crates independently without
  loosening workspace versioning later

We accept this because the experiment shouldn't be optimizing for crates.io
publication.

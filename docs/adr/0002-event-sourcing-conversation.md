# ADR-0002: Event-sourced conversation log

## Status

Accepted

## Context

The Harness must be resumable: any instance can pick up any session and
continue from where it left off. This requires that all state needed for
resumption be persistent and externalized from the Harness.

Anthropic's Managed Agents architecture uses an append-only event log as
the single source of truth. We adopt the same pattern.

## Decision

The Conversation Service is an append-only log of `ConversationEvent`s.
Every state transition inside the Harness writes an event *before* the
transition completes. Events are immutable once written.

Two implementations are required:
- `SqliteConversationStore` (production-shaped, what we'll port to Postgres)
- `InMemoryConversationStore` (tests only)

Both must pass a shared contract test (`store_contract.rs`).

## Consequences

- **Easier**: resume after crash, debug by replay, audit for free
- **Harder**: every operation needs a corresponding event type
- **Given up**: simple in-memory mutable state — we trade ergonomics for
  recoverability

We accept this because resumability is the whole point of the experiment.

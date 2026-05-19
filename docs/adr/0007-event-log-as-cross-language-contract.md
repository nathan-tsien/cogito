# ADR-0007: Event log as cross-language storage contract

## Status

Accepted

## Context

cogito ships as an embeddable Rust library. The first SaaS deployment
profile (ADR-0005 §2) co-locates a Rust process running cogito with one
or more **non-Rust services** (Go HTTP API, Python analytics, Node BFF)
that need to consume the conversation event log for user-facing query,
audit, billing, and dashboards.

These external readers cannot consume a Rust trait. They consume the
**storage itself** — JSONL files in v0.1 dev/debug deployments, the
Postgres schema in v0.4+ production deployments, and any future
backend (S3, Kafka, …).

Earlier brainstorming (2026-05-18 Q2) proposed two Rust traits
(`ConversationStore` + `ConversationCatalog`) to serve both Brain-side
writes and external-side reads. That framing was wrong: only the
Brain-side path can be served by a Rust trait. The external-side path
is necessarily storage-level.

## Decision

The `ConversationStore` Rust trait (`cogito-protocol::store`) serves
**Brain's command path + single-session replay only**. Methods on this
trait MUST be scoped to:

1. Writing one `ConversationEvent`.
2. Reading events for one explicitly-named `SessionId`.

Any cross-session, cross-tenant, or user-facing query capability —
"list conversations for user U", "search across tenants", "aggregate
billing per day" — is exposed via the **storage-level contract**, not
via Rust traits.

### Storage-level contracts cogito commits to

| Backend | Public contract | First shipped |
|---|---|---|
| `cogito-store-jsonl` (dev/debug) | JSONL line format documented at `docs/data-model/jsonl-v1.md` | v0.1 |
| `cogito-store-postgres` (production) | SQL DDL at `crates/cogito-store-postgres/migrations/0001_init.sql` | v0.4 |
| Future backends (S3, Kafka) | Backend-specific format docs | TBD |

Each storage contract is governed by `ConversationEvent::schema_version`
(ADR-0005 §4 #2). The same versioning and migration rules apply
regardless of which storage backend a reader is using.

### Additive variants for context-management lifecycle

The ADR-0006 amendment of 2026-05-19 (this PR) reserves the H11 Context
Manage component slot in a future-ADR-0008 initiative. ADR-0008 will
introduce additional `EventPayload` variants — at minimum
`ContextCompacted`, and likely `ContextDecisionRecorded`,
`SystemPromptInjected`, `ToolFilterOverridden`. Per the forward-
compatibility rules above and the `#[non_exhaustive]` attribute on
`EventPayload`, these are **additive variants** and do NOT bump
`schema_version`.

External readers (Go / Python / Node services) MUST tolerate unknown
`type` values: skip the line, log a warning, or fall back to the
generic envelope, but never crash. This is the consumer's side of the
forward-compatibility bargain.

### What this means for cogito's deliverables

- `ConversationEvent` Rust types live in `cogito-protocol::event`.
- A JSON Schema artifact (`docs/schemas/conversation-event-v1.json`)
  is generated from those types via `cogito-gen-schema` and committed
  to the repo. CI enforces no drift. External Go/Python/Node services
  can use this schema for typed deserialization or code generation.
- A canonical fixture (`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`)
  covers all 9 `EventPayload` variants and serves as a worked example
  for both internal contract tests and external readers.
- The JSONL line format spec at `docs/data-model/jsonl-v1.md` is a
  human-readable companion to the JSON Schema.

### Inviolable design rule added to `AGENTS.md`

> `ConversationStore` is Brain's command + single-session replay trait.
> Adding any cross-session, cross-tenant, or user-history query method
> to this trait is a design error. Cross-session / catalog access for
> external services is served by reading the underlying storage
> directly (per this ADR).

## Consequences

- **Easier**: external readers do not depend on the Rust compilation
  unit; they consume a stable, language-neutral artifact (JSONL bytes /
  Postgres rows / JSON Schema). Cogito's release cadence does not
  block their development.
- **Easier**: `ConversationStore`'s surface stays minimal and
  evolvable independently from query/catalog concerns.
- **Harder**: the JSONL line format and Postgres DDL become public API
  with the same SemVer obligations as Rust public symbols. Changes
  require the migration tooling outlined in ADR-0005 §4 #2.
- **Harder**: new read access patterns cannot be added by extending
  the Rust trait — they require schema design that works for SQL +
  file-scan + future backends.

## Follow-on work

- v0.1 Sprint 1: deliver the JSON Schema artifact + fixture + JSONL
  spec doc; commit the inviolable rule.
- v0.4: deliver the Postgres DDL as the second canonical storage
  contract; lock its forward-compatibility with the same
  `schema_version` mechanism.
- v0.4 onward: any new storage backend ships with its own contract doc
  alongside its implementation.

## References

- ADR-0001 (workspace layout)
- ADR-0002 (event sourcing)
- ADR-0005 (production scope + quality gates §4 #2 schema_version)
- Spec `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md` §4
- `AGENTS.md` (new inviolable rule under §"Inviolable design principles")

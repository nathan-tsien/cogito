# ADR-0024: Crate naming consolidation

## Status

Proposed — placeholder (finalized as part of the
`cogito-store-jsonl → cogito-store` rename PR, landing before v0.1.0
tag).

Captures the naming principle ratified in the
[2026-05-22 roadmap rebalance spec](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
§4.5 + §7.7.

This ADR is **optional** — the same content could be folded into an
amendment of ADR-0005 ("Production scope and quality gates"). The
choice between "new ADR-0024" vs "ADR-0005 amendment" is finalized
when the rename PR is opened.

## Context

cogito's original workspace layout (ADR-0001) named storage crates
after their backend implementation:

- `cogito-store-jsonl` — Session-store backend using JSONL files
- `cogito-store-postgres` — planned for v0.4, Postgres backend

Same pattern was about to be repeated for Context Management
implementations (`cogito-context-truncate`, `cogito-context-summarize`,
…) until the 2026-05-22 rebalance caught it.

**Problem**: Encoding backend names into crate names forces every new
backend to be a new crate. This:

- Multiplies workspace member count (CLAUDE.md "adding a new crate
  requires explicit approval" gets repeatedly violated, or the rule
  loses force)
- Spreads related code across multiple `Cargo.toml`s
- Makes "choose backend by config" require splitting the dispatch
  factory across crate boundaries (violates CLAUDE.md "tagged-config
  factories belong in the crate that owns the implementations")

**Correct pattern**: name the crate by **layer / role**, not
backend; multiple backends live as **modules** or **features** within
the same crate.

Concrete example (from the rebalance):

- `cogito-context` umbrella crate contains
  `compactor::{truncate, summarize, sliding}` + `projector::{...}` +
  `injector::{...}`. One crate, one factory, many strategies.

The same principle is now retroactively applied to the existing
`cogito-store-jsonl` crate.

## Decision

### 1. Naming principle (forward-going)

**Crate names label layers and roles, not implementations.** A crate
hosting trait `X` and N implementations of `X` is named after `X`
(or its layer/role), with implementations as modules or
feature-gated submodules:

```
cogito-<role>/
  src/
    lib.rs                 # trait re-export + build_<role> factory
    <impl_a>/              # e.g. jsonl/, truncate/, anthropic/
    <impl_b>/              # e.g. postgres/, summarize/, openai/
```

Features gate optional impl deps:

```toml
[features]
default = ["<canonical-impl>"]
<impl_a> = []
<impl_b> = ["dep:<heavy-dep>"]
```

**Adopted by**:
- `cogito-model` (already follows: `anthropic`, `openai_compat`
  modules) — confirms the pattern
- `cogito-context` (v0.1 Sprint 6 — new, per spec §4.1)
- `cogito-store` (this ADR — rename from `cogito-store-jsonl`)

**Exempt**: Surface crates (`cogito-cli`, `cogito-tui`) and crates
that fundamentally represent a single concept with no plural
implementations (`cogito-protocol`, `cogito-sandbox`, `cogito-jobs`).

### 2. `cogito-store-jsonl` → `cogito-store` rename

```
crates/cogito-store-jsonl/      crates/cogito-store/
  src/                             src/
    lib.rs                           lib.rs        # trait re-export + build_store
    ...                              jsonl/        # current jsonl content moved here
                                       mod.rs
                                       ...
```

`Cargo.toml`:

```toml
[features]
default = ["jsonl"]
jsonl = []
postgres = ["dep:tokio-postgres", "dep:bb8-postgres"]  # v0.4
sqlite = ["dep:rusqlite"]                              # future
```

### 3. v0.4 plan adjustment

The v0.4 ROADMAP item `cogito-store-postgres` (separate crate)
becomes `cogito-store --features postgres` (module inside the umbrella
crate, gated by Cargo feature). ROADMAP.md + ARCHITECTURE.md are
updated as part of the rename PR.

### 4. Accepted ADR handling

Accepted ADRs that reference `cogito-store-jsonl` (notably ADR-0006
and ADR-0007) are **not modified**. ADR text is immutable historical
record. This ADR maintains the **historical-name map**:

| Old crate name | New crate name | Equivalent path/feature |
|---|---|---|
| `cogito-store-jsonl` | `cogito-store` (default) | `cogito-store::jsonl` |
| `cogito-store-postgres` (planned, v0.4) | `cogito-store --features postgres` | `cogito-store::postgres` |

Future readers of older ADRs should consult this map.

### 5. Documentation update scope (rename PR)

In-scope (must be updated):

- `Cargo.toml` (workspace + each member's manifest)
- `Cargo.lock`
- `Makefile`
- All `.rs` files (`use cogito_store_jsonl::*`, type names)
- All test files (`crates/cogito-core/tests/*`)
- Non-immutable docs: `ROADMAP.md`, `ARCHITECTURE.md`, `CLAUDE.md`,
  `AGENTS.md`, `CHANGELOG.md`, `docs/components/H02-step-recorder.md`,
  spec files under `docs/superpowers/specs/`
- ADR docket table in `docs/adr/README.md` (index entry only; existing
  ADR text unchanged)

Out-of-scope (must NOT be touched):

- ADR-0006 body
- ADR-0007 body
- ADR-0019 body
- Any other Accepted ADR's body

### 6. Rename PR shape

- **Title**: `refactor(workspace): cogito-store-jsonl → cogito-store (jsonl as default feature)`
- **Single PR**, not split — atomic rename minimizes broken-intermediate-state risk
- **CI must be green** at the head commit; no partial broken state pushed
- Target landing: any time before v0.1.0 tag (i.e., during or just
  before Sprint 10)

## Consequences

**Easier**:
- Adding a new store backend in the future is a feature flag + module,
  not a workspace approval cycle
- One `build_store` factory; one place to extend (CLAUDE.md tagged-
  config principle satisfied)
- Same Cargo.lock entry across backends — easier dependency review

**Harder**:
- The rename touches 30+ files; review must verify nothing slipped
  through (CI is the safety net)
- Newcomers who read older Accepted ADRs see `cogito-store-jsonl` and
  must consult this ADR's map — minor docs friction
- Cargo features add some build-matrix complexity (CI must build with
  `--no-default-features --features postgres` etc., once postgres
  lands)

**Given up**:
- Separate Cargo.toml dep listing for each backend — slightly less
  visible per-backend dep weight (compensated by feature gates)
- The conceptual cleanliness of "one crate per fully-isolated
  responsibility" — accepted, because in practice the responsibility
  is "be a Session store", not "be a JSONL store"

## References

- Rebalance spec: [`docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md`](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md) §4.5 + §7.7
- ADR-0001 — original workspace layout (sets the pattern this ADR
  refines)
- ADR-0004 — Brain / Hands / Session layers
- ADR-0005 — Production scope (this ADR may fold into an amendment
  here instead of standalone — TBD at PR open)
- CLAUDE.md §"Coding standards" — "Tagged-config factories belong in
  the crate that owns the implementations"

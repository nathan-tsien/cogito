# ADR-0005: Production scope, quality gates, SLO posture, compatibility commitments

## Status

Accepted

## Context

cogito's positioning changed during the v0.1 design phase: from
"controlled experiment that validates a 10-component Harness design" to
"**production-grade Agent Runtime core, packaged as an embeddable Rust
library**." This ADR ratifies that positioning and locks the quality
gates, SLO posture, and compatibility commitments that follow.

The pivot was driven by a concrete first-customer profile: a single
product feature backend (chat / IDE / code assistant / multimodal task
agent) at hundreds-to-thousands of concurrent sessions per process, with
SLO + oncall + canary release processes, and event logs durable for the
product's lifetime.

The previous "experimental" framing in AGENTS.md, README.md, ROADMAP.md,
and ARCHITECTURE.md is **retired**. ADR-0001 / 0002 / 0003 / 0004 carry
over unchanged — the architectural decisions in those ADRs are correct
under either framing; production-grade only changes the quality bar and
the "done" definition.

## Decision

### 1. Form: embeddable Rust library (not a daemon, not SaaS core)

cogito ships as a Cargo workspace that consumer Rust services depend on
and run in-process. cogito does not own:

- a process model (consumer spawns it as part of their service)
- inbound transport (no HTTP / gRPC inbound API)
- end-user authentication
- multi-tenant isolation enforcement (only propagation via `TenantContext`)
- deployment artifacts (no Docker / Helm)
- quota / billing ledgers

These are the consumer's concerns or a future SaaS-wrapper layer.
Future-SaaS-readiness is preserved via the trait-based pluggability of
`ConversationStore`, `StorageSystem`, `JobManager`, and `MetricsRecorder`
(all extensions point in v0.4).

### 2. First production consumer target

- **Workload shape**: single product feature backend; hundreds–thousands
  of concurrent sessions per process; SLO + oncall + canary release
  process; event logs durable for product lifetime.
- **Scaling model**: per-process replica capacity is the primary scaling
  unit; consumer runs K replicas behind a load balancer with `session_id`
  sticky routing. cogito does not coordinate across processes.

### 3. SLO posture

Initial provisional numbers; **locked after Sprint 1 measurements** in
v0.1, after which they become budgetary constraints for all subsequent
component work:

| Metric | Provisional target | Owner |
|---|---|---|
| P99 step record write latency | < 5 ms | H02 + `ConversationStore` impl |
| P99 TTFT overhead from cogito above raw `ModelGateway` TTFT | < 50 ms | H01 + H04 + H06 |
| P99 sync tool dispatch overhead | < 10 ms | H08 |
| Idle session memory footprint | < 1 MiB | Runtime |
| Concurrent sessions per process | ≥ 1000 active (degradation-free) | Runtime + tokio scheduler |

Sprint 1 must include a benchmark suite that measures the first metric
to lock its real number. Subsequent sprints add measurements for their
respective components.

### 4. Quality gates (six gates, CI-enforced where mechanical)

1. **Rust API stability**
   - Pre-1.0: SemVer 0.x.y; breaking changes allowed in minor versions; documented in `CHANGELOG.md`.
   - At 1.0: SemVer strict. `#[non_exhaustive]` on every public enum; sealed marker traits for non-extensible traits; "stability tier" doc comments (`stable` / `experimental` / `deprecated`) on each public symbol.

2. **Event log compatibility**
   - Every `ConversationEvent` carries `schema_version: u32` from day 1.
   - 0.x: pragmatic compatibility. Breaking changes allowed if accompanied by a migration tool and upgrade runbook. CI runs "v(N-1) writer → vN reader → semantic equivalence" tests against the previous release.
   - 1.0: strict forward-compatibility. Every future version reads every past version. Migration becomes a code-internal concern, not a user concern.

3. **Observability**
   - Every state-machine transition emits a `tracing` span with a stable name (named in component docs).
   - Metrics flow through the `MetricsRecorder` trait in `cogito-protocol` (no hard Prometheus dep).
   - Optional `cogito-observability-otel` crate provides OpenTelemetry adapters (v0.4).
   - Structured log schema documented and versioned alongside event schema.

4. **Failure isolation**
   - Panic catch boundary at H01 turn entry — a panicking tool / hook / model adapter fails the **current turn**, never brings down the process or affects other sessions.
   - tokio task panics are caught by Runtime.
   - Per-session resource budgets (memory cap, time cap) enforced at Runtime; exceeding fails the session, not the process.

5. **Security**
   - `cargo audit` + `cargo deny` in CI.
   - Secret redaction via a `Redactor` trait applied to events before persistence (default no-op in v0.1; default redactor in v0.2; consumer-supplied policy at any version).
   - LLM provider credentials never logged.
   - **Threat model document** required before 1.0.
   - **Credential isolation pattern** (sandbox proxy; tokens never reachable from sandbox-executed code) defined in ADR-0013 at v0.4 (renumbered from ADR-0011 by PR #6). v0.1–v0.3 ship with documented hazard notice in the sandbox component doc.

6. **Test depth**
   - Existing: unit + contract + integration + resume_chaos.
   - Adding through 0.x:
     - **Load test** at v0.6: 1000 concurrent sessions per process.
     - **Soak test** at v0.6: 24h continuous run, no leaks or degradation.
     - **`ConversationStore` backend compatibility test** at v0.4: contract test parameterized over Postgres versions.
     - **Event parsing fuzz test** at v0.4: ensures malformed payloads don't crash readers.

### 5. Compatibility commitments

| Surface | 0.x policy | 1.0 policy |
|---|---|---|
| Rust public API | SemVer 0.x.y; breaking allowed in minor; CHANGELOG entry required | SemVer strict |
| `ConversationEvent` schema | Breaking allowed with migration tool + runbook; `schema_version` carries forward | Strict forward-compat: every future version reads every past version |
| `ContentBlock` variants | Additive only (new variants OK); removing variants = major version | Same |
| `StorageSystem` URI resolvability | Not guaranteed across time (backend's concern) | Same |
| Storage HTTP wire protocol (ADR-0015, lands v0.6) | Independent versioning from event log; documented matrix in ADR-0015 | Same |
| Trait shapes (`ToolProvider`, `JobManager`, etc.) | Breaking allowed in minor; default-method additions are non-breaking | SemVer strict |
| Strategy YAML schema | Additive fields only (existing fields keep meaning) | Same; deprecation cycle for removals |

### 6. What we explicitly do not do (regardless of version)

- Web UI / mobile clients
- Multi-tenant isolation enforcement (only context propagation; enforcement is consumer's)
- End-user authentication
- Quota / billing / metering ledgers (cogito emits metrics; ledger is consumer's)
- Deployment artifacts (Docker / Helm / IaC)
- RAG / vector store (consumer's Hand if needed)
- Cross-session persistent memory (would need its own ADR)
- Inbound HTTP / gRPC transport (consumer wraps cogito with their own API surface)

## Consequences

- **Easier**: every component author knows what "done" looks like. Quality gates are mechanically checkable in CI. Provisional SLO numbers anchor performance discussions instead of relitigating them per sprint.
- **Harder**: 1.0 is a real commitment — public API audit + stability tier annotations + migration tooling. Cannot drift past 0.x indefinitely.
- **Given up**: the freedom to "rewrite the experiment" if a design choice proves wrong. From v0.1 onward, breaking changes require a migration tool. ADRs become the place to revisit decisions, not arbitrary refactoring.

## Follow-on work

- v0.1 Sprint 1: lock the first SLO number (step record write latency).
- v0.2 onward: each version adds its quality-gate evidence to `docs/quality/` (load tests, migration tools, audit reports).
- 1.0: full public API stability commitment.
- Sandbox / credential isolation work (ADR-0012 / ADR-0013) at v0.4 (renumbered from ADR-0010/ADR-0011 by PR #6).

## References

- ADR-0001 (workspace layout)
- ADR-0002 (event sourcing)
- ADR-0003 (state-machine Turn Driver)
- ADR-0004 (Brain / Hands / Session boundaries)
- ARCHITECTURE.md §"Version evolution path"
- AGENTS.md (operating manual)

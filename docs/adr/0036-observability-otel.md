# ADR-0036: Observability extension point (MetricsRecorder seam + traces)

## Status

Proposed (draft, 2026-06-03). Reframed from "build an OpenTelemetry adapter
crate" to "lock the observability **extension point**; let adapters and metric
density grow incrementally."

Decision item 1 (the injectable `RuntimeBuilder::metrics()` setter) is
**implemented (2026-06-03)** — `MetricsRecorder` is now consumer-injectable;
default stays `NoOpMetricsRecorder`. The remaining items (additive-evolution
discipline, incremental metric density, the optional OTel adapter crate) stay
as drafted below.

Consumer-directed path: cogito core exposes the observability seam and stays
open; the concrete OTel adapter and the metric taxonomy are layered on / added
during development, not built up front. This is the same core-responsibility
stance as ADR-0014 (expose a seam, the consumer implements).

## Context

cogito has two observability channels today:

- **Metrics** — the `MetricsRecorder` trait (`cogito-protocol::metrics`),
  shipped in Sprint 5 with a `NoOpMetricsRecorder` default. It is threaded
  into `SessionState` and the hook pipeline. **But two gaps make it not yet a
  usable extension point:**
  1. It is **hardcoded to `NoOpMetricsRecorder`** at the build site
     (`runtime/builder.rs`); `RuntimeBuilder` exposes **no public setter**, so
     a consumer cannot inject its own recorder.
  2. Its vocabulary is minimal — `record_hook_invocation` + `record_counter` —
     and only the `bash_audit` example hook calls it. Core turn/model/tool
     lifecycle points do not emit metrics yet. (The `StepRecorder.record_*`
     calls in the harness write the durable *event log*, a different concern.)
- **Traces / logs** — `tracing` is already used across the runtime/harness
  (~21 call sites). cogito follows the `tracing` convention (CLAUDE.md:
  `print_stdout`/`print_stderr` warn).

Building a full `cogito-observability-otel` crate now is premature and, given
the seam, largely unnecessary: a consumer can implement `MetricsRecorder`
against its own telemetry the moment the seam is injectable, and OTel traces
are a subscriber concern the consumer owns.

## Decision

Lock the extension point; defer the adapter.

1. **Make `MetricsRecorder` injectable (the missing entry point).** DONE
   (2026-06-03): `RuntimeBuilder::metrics(Arc<dyn MetricsRecorder>)` setter
   added; the default stays `NoOpMetricsRecorder`; `open_inner` clones the
   runtime's recorder into each session's `SessionState` and hook pipeline
   instead of the previously hardcoded no-op. Mirrors the existing builder
   setters. Without it the trait was dead.

2. **`MetricsRecorder` evolves additively — forever.** Every new method ships
   a default (no-op) body, so adding instrumentation never breaks a consumer's
   existing `impl`. This is the rule that makes "refine / add metrics during
   development" safe. Optionally widen the primitive vocabulary now
   (`record_gauge`, `record_histogram` with name + value + labels) so consumers
   can express real telemetry; additive, cheap.

3. **Metric density grows incrementally.** Emit `metrics.record_*` at turn
   start/end, model-call latency, token usage, tool dispatch, and error points
   as development proceeds — not all up front. Each addition is additive per
   rule 2.

4. **Traces stay consumer-owned.** cogito emits spans/events via `tracing` and
   grows `#[instrument]` coverage incrementally. To export OTel traces, the
   **consumer installs an OpenTelemetry `tracing-subscriber` Layer and owns the
   global subscriber** — cogito never calls `set_global_default`. No cogito
   API is required for this.

5. **`cogito-observability-otel` adapter crate — DEFERRED / optional.** Once
   the seam is injectable, a consumer can wire its own `MetricsRecorder`
   directly; cogito need not ship the OTel crate. Build it later only as a
   convenience, if a consumer wants a ready-made adapter.

## Consequences

What becomes easier:

- The observability extension point becomes real with a few lines (the
  setter), and the additive-evolution rule lets the team add metrics over time
  without coordinating breaking changes with consumers.
- praxis (or any consumer) plugs in its own telemetry today; cogito core stays
  minimal and does not take an OTel dependency.

What we give up / accept:

- No turnkey OTel out of the box until (if) the adapter crate is built — the
  consumer writes a thin `MetricsRecorder` impl. Acceptable and consistent with
  the open-core stance.
- The metric taxonomy (names, labels, which lifecycle points) is intentionally
  not frozen here; it accretes with development. The only frozen contract is
  the seam shape and the additive-evolution rule.

## Open questions

- Whether to widen the primitive vocabulary (counter/gauge/histogram) in the
  same change as the setter, or add primitives lazily when the first real
  metric needs them. Leaning: add the setter now; add primitives when first
  needed (still additive).

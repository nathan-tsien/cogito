# ADR-0036: OpenTelemetry observability adapter

## Status

Proposed (draft, v0.4).

Day-2 concern. praxis can integrate cogito **today** with the shipped
`NoOpMetricsRecorder` default and its own `tracing` subscriber; nothing
here blocks SaaS onboarding. This ADR locks the design of an optional
adapter crate so that, once praxis runs multiple replicas behind its own
gateway and wants per-tenant latency/error visibility, the wiring is
already decided.

## Context

Two observability seams already exist in the tree and are deliberately
left at no-op / consumer-owned defaults:

1. **Metrics — `MetricsRecorder`** (`cogito-protocol::metrics`, frozen
   Sprint 5). The trait is small and synchronous:

   ```rust
   pub trait MetricsRecorder: Send + Sync {
       fn record_hook_invocation(&self, point: HookLifecyclePoint,
                                  hook_name: &str, duration: Duration, allowed: bool);
       fn record_counter(&self, name: &str, labels: &[(&str, &str)]);
   }
   ```

   The runtime today hardcodes `Arc::new(NoOpMetricsRecorder)` in
   `runtime/builder.rs` and shares that one Arc between the hook
   pipeline (`CompositeHookPipeline::with_handlers_and_metrics`) and the
   per-turn `TurnDeps.metrics` slot. Call sites are `composite.rs`
   (hook invocations) and the Sprint 6 context pipeline (counters).

2. **Tracing — the `tracing` facade.** The runtime and Brain emit
   `tracing::warn!` / `tracing::error!` events at many sites
   (`session_loop.rs`, `model_completed.rs`, `subagent.rs`,
   `handle.rs`, `context_managed.rs`). cogito **never** calls
   `tracing_subscriber::set_global_default` — it only emits. The
   process owner installs the subscriber.

This ADR does **not** re-introduce the `MetricsRecorder` trait (already
shipped) and does **not** add a span taxonomy to the Brain by fiat.
It defines a single new Surface-layer adapter crate that turns these two
already-emitted signals into OpenTelemetry traces and metrics, and
fixes the ownership boundary so cogito stays embeddable.

## Decision

### 1. New crate: `cogito-observability-otel` (Surface / adapter)

A standalone, opt-in crate that depends only on `cogito-protocol`
(for `MetricsRecorder` + `HookLifecyclePoint`), `tracing`,
`tracing-opentelemetry`, and the `opentelemetry` / `opentelemetry-otlp`
stack. It ships **no** dependency on `cogito-core`, so embedding it is
purely additive and a consumer who wants a different backend (Prometheus,
Datadog, a hand-rolled recorder) simply does not add it.

It exposes two independent pieces, usable separately:

- `OtelMetricsRecorder` — an `impl MetricsRecorder` that maps trait
  calls onto OTel instruments.
- `otel_layer()` — a composable `tracing_subscriber::Layer` (a
  `tracing-opentelemetry` `OpenTelemetryLayer`) the consumer adds to
  **its own** subscriber.

### 2. Trace-exporter ownership: consumer owns the global subscriber

cogito ships a `Layer`, never a subscriber, and **never calls
`set_global_default`**. This is the same posture as today (cogito only
emits `tracing` events). The consumer composes:

```rust
// praxis side, once at process start
let subscriber = tracing_subscriber::registry()
    .with(fmt_layer)                 // praxis's own
    .with(cogito_observability_otel::otel_layer(tracer)); // cogito's
tracing::subscriber::set_global_default(subscriber)?;
```

Rationale: a library that grabs the global subscriber cannot be embedded
beside the consumer's own telemetry. The `Layer` form lets praxis
interleave cogito spans with its gateway/HTTP spans under one trace tree
and one exporter pipeline it controls (sampling, batching, endpoint,
resource attributes).

### 3. Metrics wiring: inject the recorder, do not hardcode it

To deliver OTel metrics the runtime must accept a `MetricsRecorder`
instead of always constructing `NoOpMetricsRecorder`. Today
`runtime/builder.rs` hardcodes the no-op. This ADR calls for a single
additive builder seam:

```rust
RuntimeBuilder::with_metrics(Arc<dyn MetricsRecorder>) // default: NoOpMetricsRecorder
```

The resolved Arc replaces the hardcoded `NoOpMetricsRecorder` and feeds
both the hook pipeline and `TurnDeps.metrics`, exactly as the two are
kept in sync today. No Brain change, no protocol change. praxis builds
`OtelMetricsRecorder`, wraps it in an Arc, and passes it once.

Whether the recorder should also be a per-session override on
`SessionSpec` (so a tenant can carry its own sink) is left open — see
Open questions.

### 4. Metric taxonomy (from the two existing trait methods only)

The adapter emits exactly what the frozen trait already feeds it; it
does not invent new instrumentation points in the Brain.

- `record_hook_invocation` →
  - histogram `cogito.hook.duration` (unit: seconds), and
  - counter `cogito.hook.invocations`
  - attributes: `hook.point` (the `HookLifecyclePoint` rendered as a
    stable lowercase string), `hook.name`, `hook.allowed` (bool).
- `record_counter(name, labels)` →
  - counter named `cogito.<name>` (the trait's free-form `name`
    prefixed with the `cogito.` namespace; existing names like the
    Sprint 6 context-decision counters pass through unchanged),
  - the trait's `&[(&str,&str)]` labels become OTel attributes verbatim.

Instrument names use the `cogito.` prefix and dotted OTel convention.
The adapter creates instruments lazily and caches them by name; the
trait's synchronous/non-blocking contract is honoured by relying on the
OTel SDK's own buffering (the trait doc already permits "buffer + drain
off-task").

### 5. Span taxonomy: derived from existing `tracing` spans, not mandated here

The trace `Layer` exports whatever spans the runtime already opens. This
ADR does **not** mandate a new span tree across H01–H11; doing so would
touch the Brain and is out of scope for a day-2 adapter. As a separate,
additive follow-up the runtime MAY add `#[tracing::instrument]` spans at
the natural turn boundaries that already correspond to the state machine
(turn, model call, tool dispatch). If/when that lands, the recommended
names are `cogito.turn`, `cogito.model_call`, `cogito.tool_dispatch`,
matching the metric namespace. That work is explicitly gated behind its
own review (Open questions) and is not required for the adapter to be
useful — error/warn events already export today.

### 6. Tenant-label policy (soft-depends on ADR-0014)

`tenant_id` and `user_id` are already stamped into `SessionMeta` by
ADR-0028. The adapter itself holds no tenant context — it only forwards
the labels it is handed. Policy:

- The adapter attaches `tenant.id` / `user.id` attributes **only when
  the caller supplies them** through the label slice / span fields.
  cogito does not read `SessionMeta` from inside the adapter.
- High-cardinality protection is the consumer's call: praxis decides
  whether `user.id` is an attribute (cardinality risk) or dropped /
  hashed. The adapter ships a constructor flag
  `OtelMetricsRecorder::new(opts)` with `emit_user_id: bool` (default
  `false`) so the unbounded-cardinality dimension is opt-in.
- Full, uniform tenant-context propagation into every span/metric is the
  province of **ADR-0014 (TenantContext propagation)**, still a reserved
  v0.4 ADR. Until 0014 lands, tenant labelling is best-effort at the
  call sites that already have the values. This ADR does not pre-empt
  0014's mechanism.

## Consequences

**Easier**:
- praxis gets OTel traces + metrics by adding one crate and one builder
  call, with zero Brain or protocol change.
- Multi-replica visibility (per-tenant hook latency, decision counters,
  error rates) lands behind praxis's own gateway and exporter.
- The exporter, sampler, and resource attributes stay under consumer
  control — cogito remains a well-behaved embedded library.

**Harder**:
- One real (non-trivial) code change is required: the
  `RuntimeBuilder::with_metrics` seam, replacing the hardcoded
  `NoOpMetricsRecorder`. It is additive and defaulted, but it is the
  one place this ADR is not purely "new crate".
- Rich span coverage is deliberately deferred; out of the box the trace
  export is thin (events + any future instrumented spans) until the
  Brain-span follow-up is approved.

**Given up**:
- A cogito-owned global subscriber / turnkey "just works" telemetry —
  rejected on embeddability grounds (Alternatives considered).
- Tenant labelling as a cogito-enforced invariant — left to ADR-0014;
  here it is forward-only and opt-in.

## Alternatives considered

- **cogito owns the subscriber and calls `set_global_default`.**
  Rejected: breaks embedding next to praxis's own telemetry and
  contradicts the existing emit-only posture. A library must not seize
  the process-global subscriber.
- **Bake OTel into `cogito-core` behind a feature flag.** Rejected:
  pulls the OTel SDK into the Brain/Runtime dependency closure for every
  consumer and violates the layer map — observability export is a
  Surface concern, like `cogito-cli`. A separate adapter crate keeps
  `cogito-core` lean and lets a Prometheus/Datadog adapter sit beside it.
- **Add OTel-specific methods to `MetricsRecorder`.** Rejected: the
  trait is frozen and intentionally backend-agnostic; the two existing
  methods carry enough to build the metric set above.

## Open questions

- Should the resolved `MetricsRecorder` also be a per-session override on
  `SessionSpec` (ADR-0028), so each tenant can carry its own sink /
  exemplar context, or is one process-wide recorder with tenant
  *attributes* sufficient? Leaning process-wide; flagged for human call.
- Should the runtime add `#[tracing::instrument]` spans at the
  turn / model-call / tool-dispatch boundaries (section 5)? This touches
  `cogito-core` and wants its own review; not required for the adapter.
- Exact rendering of `HookLifecyclePoint` as a stable attribute string —
  needs a `Display`/`as_str` on the enum in `cogito-protocol` (additive)
  so the adapter does not encode the mapping privately.
- Interaction with ADR-0014 once it lands: does TenantContext become the
  single source the adapter reads, superseding the manual label-passing
  in section 6? To be reconciled when 0014 is drafted.

## References

- `cogito-protocol::metrics` — the frozen `MetricsRecorder` trait +
  `NoOpMetricsRecorder` default this adapter implements.
- `runtime/builder.rs` — the current hardcoded `NoOpMetricsRecorder`
  site that section 3 replaces with an injection seam.
- ADR-0028 (per-session provider injection) — stamps `tenant_id` /
  `user_id` into `SessionMeta`; the source of tenant labels.
- ADR-0014 (TenantContext propagation, reserved v0.4) — the owner of
  uniform tenant-context policy; section 6 soft-depends on it.
- ADR-0004 (Brain / Hands / Session boundaries) — places this adapter in
  the Surface layer, outside `cogito-core`.
- ADR-0005 (production scope + quality gates) — the SLO/observability
  posture this serves.

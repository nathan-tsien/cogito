# ADR-0014: TenantContext — identity stays in SessionMeta, no ExecCtx propagation

## Status

Accepted (2026-06-03). Resolves the v0.4 "tenant propagation" question.

Supersedes an earlier Route-B draft of this ADR (which proposed adding a
runtime `TenantContext` handle to `ExecCtx`). That draft is withdrawn; the
decision below is Route A — no protocol change.

## Context

cogito core is a Brain + Hands framework: it sediments the agent loop
(orchestration + tool/session/boundary seams) and deliberately stays **open**
so a consumer can build SaaS capabilities on top, rather than baking SaaS
policy into the core. The question for v0.4 was whether tenant identity needs
a second, runtime-propagated form delivered to tools/hooks at dispatch time.

What already exists:

- `tenant_id` / `user_id` are stamped into `SessionMeta`
  (`cogito-protocol::session`) at session open by ADR-0028, recorded once in
  the seq=0 `SessionStarted` event. This is durable, log-level identity:
  "who does this session belong to", available by reading the event log.
- Providers are injectable per session (ADR-0028 `SessionSpec.tools` /
  `.skills` / store / workspace). The consumer constructs these on its own
  request side at `open_session_with` time.

The consumer (praxis) owns gateway routing, brings its own
`ConversationStore`, builds its own per-session providers, and **disallows
cogito-internal-initiated surface changes** (no model-self-activated skill or
tool-added MCP server may alter the surface; surface composition is entirely
consumer-determined per request). So the consumer holds `tenant_id` /
`user_id` at the exact point where it constructs the providers.

A decisive runtime detail: cogito drives each turn on its own session-actor /
`TurnDriver` task (ADR-0006 actor model), **not** on the consumer's request
task. A `task_local` set on the consumer's HTTP request task therefore does
**not** propagate into the task where tools/hooks actually execute. So
"read tenant from the consumer's request scope" cannot mean an ambient
task-local that crosses into cogito; it must mean the tenant is captured into
the per-session provider the consumer injects.

## Decision

Route A. No new protocol surface.

1. **No `TenantContext` value type and no `ExecCtx` field.** `cogito-protocol`
   is unchanged. There is no second runtime copy of tenant identity.

2. **Tenant identity lives in `SessionMeta`** (`tenant_id` / `user_id`, already
   shipped by ADR-0028). This is the single source of truth, used for log
   attribution / audit / billing reconstruction, and is what observability
   (ADR-0036) reads to label traces and metrics by tenant.

3. **Consumers that need tenant at dispatch time bind it into the per-session
   provider** they inject through `SessionSpec` (ADR-0028). The consumer holds
   `tenant_id` request-side at `open_session_with`; it captures that into the
   `ToolProvider` / `HookProvider` (and, if needed, a per-session store handle)
   it constructs. The tool then reads tenant from its own closure, not from
   `ExecCtx`. Capture-at-open is correct because a session belongs to exactly
   one tenant for its lifetime; no per-turn ambient is needed. A task-local on
   the consumer's request task is **not** a supported mechanism (it does not
   cross into cogito's turn-driver task — see Context).

4. **Authentication and authorization of tools and execution are out of scope
   for cogito core.** In particular, MCP credential handling is not solved
   here; it is future work under a dedicated **Credential Broker** design
   (related to ADR-0013). The Brain (the Agent loop) never handles credentials
   or secrets — it decides; the Hands layer and the future broker seam carry
   auth. cogito core's job is to expose seams (ADR-0028 provider injection,
   `ExecCtx`, the future broker) that a SaaS layer composes, not to own tenant
   policy, quotas, rate limits, entitlements, or credentials.

## Consequences

What becomes easier:

- Zero protocol surface to maintain for tenancy: no new value type, no
  `ExecCtx` field, no hand-written `Debug` update, and no resume-time
  reconstruction rule.
- cogito core stays minimal and open — the consumer composes tenant scoping at
  the same seam where it already composes providers (ADR-0028), and layers its
  own enforcement above the core.

What we give up / the accepted constraint:

- A tool that is **not** consumer-constructed — a cogito builtin
  (`read_file` / `bash` / `grep` / ...) or a globally-shared MCP provider —
  cannot see tenant identity at dispatch time. This is acceptable: builtins do
  not need tenant, and the consumer owns the surface (and disallows
  cogito-internal surface changes), so any tenant-sensitive tool is
  consumer-constructed and can capture tenant itself.

## Alternatives considered

- **Route B — add `ExecCtx.tenant: Option<Arc<TenantContext>>`.** Propagate a
  runtime tenant handle to every tool/hook. Rejected: redundant with ADR-0028
  (the consumer already holds tenant where it builds providers), adds a
  permanently-maintained protocol field plus a resume-reconstruction subtlety,
  and its only unique benefit — tenant visible to tools the consumer did *not*
  construct — is moot when the consumer owns the entire surface. The reasoning
  mirrors the self-describing-resume cut (ADR-0035 withdrawn): when the
  consumer controls composition, cogito need not propagate identity.

- **Richer `TenantContext` (a free-form label map, quotas, entitlements).**
  Rejected on principle: that pulls SaaS policy into the core. Enforcement is
  the consumer's job (ROADMAP "What we explicitly do not do").

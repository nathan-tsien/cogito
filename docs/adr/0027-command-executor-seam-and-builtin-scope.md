# ADR-0027: `CommandExecutor` seam and the deliberately-small builtin tool set

**Status**: Accepted (2026-05-29)
**Date**: 2026-05-29
**Spec**: `docs/superpowers/specs/2026-05-29-core-tools-bash-webfetch-design.md`
**Related**: ADR-0004 (Brain / Hands / Session layering), ADR-0012 / ADR-0013
(v0.4 sandbox lifecycle + credential isolation — this ADR reserves their
seam), ADR-0018 (MCP integration), ADR-0023 (bundled script execution),
Sprint 8 (async Job semantics)

## Context

Until this change the builtin tool set was `read_file` (the sole sync
`BuiltinTool`) and `run_tests` (the async `cogito-jobs` demo). Two core
tools were added during Sprint 10 as an explicit addition (not the
original schedule): `bash` (arbitrary shell — the universal escape hatch)
and `web_fetch` (fetch a URL, hand HTML to the model as markdown).

Adding `bash` forced a positioning decision: **how should a tool spawn a
subprocess?** A sandbox is an *optional, policy-driven isolation
environment*, not a hard dependency of any tool. The same tool must work
whether isolation is on or off, because "do we isolate?" is a deployment /
policy question, not a tool question:

- **Host binary**: the operator may or may not enable isolation. Off =
  run directly on the host; on = run in a jailed environment (cwd jail,
  resource limits, no host-process contamination).
- **SaaS-embedded ApiServer**: a single multi-tenant process must not
  freely spawn children on the API-server host. Isolation must become a
  remote / per-tenant implementation, or host command execution is
  disabled outright.

Therefore `bash` cannot hard-depend on a concrete sandbox. It must depend
on an *execution abstraction* whose isolating-or-not behavior is decided by
the implementation injected at runtime.

A second decision rode along: **what belongs in the builtin set at all?**
cogito is an embeddable agent runtime whose philosophy is "the Brain only
decides; the hands are supplied by the consumer or come via MCP." A bloated
builtin catalog contradicts that.

## Decision

### Two-layer model: Tool abstraction vs `CommandExecutor`

cogito separates "tool" from "subprocess execution" into two layers. This
is the governing boundary for the whole change:

- **Layer 1 — Tool abstraction (Brain-visible; what H08 dispatches).**
  `ToolProvider` is the *only* thing Brain / H08 see. Everything the model
  can call is a tool: builtins (`read_file` / `web_fetch`), async tools
  (`run_tests` / `bash`), MCP tools (`mcp__server__tool`), subagents. H08
  does `provider.invoke(name, args, ctx) -> InvokeOutcome::{Sync, Async}`
  and drives the FSM. **H08 does not know `CommandExecutor` exists.**
- **Layer 2 — `CommandExecutor` (Hands-internal; the subprocess primitive
  beneath a tool).** "Run a subprocess in the policy-selected
  environment." It sits *below* the `ToolProvider` boundary and is invisible
  to Brain / H08. Only tools / subsystems that *need to spawn a process*
  use it. `bash` decides inside its own `invoke` whether to run
  synchronously or to spawn a background job; H08 only ever sees the final
  `InvokeOutcome`.

The trait lives in `cogito-protocol` (`CommandExecutor`, with `CommandSpec`
/ `CommandOutcome` / `CommandError`). It is runtime-only: never serialized
into the event log, not part of the cross-language wire contract, so adding
it does **not** bump `SCHEMA_VERSION`. The `env` policy and root directory
are construction-time concerns (they live on `SandboxConfig`), keeping the
per-call surface minimal. Per ADR-0004, `cogito-jobs` gaining a dependency
on `cogito-protocol` to reach the trait is compliant; the executor instance
is injected at the Surface layer.

### v0.1 ships only `DirectExecutor`

`cogito-sandbox` provides `DirectExecutor`, which runs `sh -c <command>` on
the host. It is **not a security boundary** (no namespaces / seccomp /
chroot). Real isolation — lifecycle, resource limits, remote / per-tenant
execution — is deferred to v0.4 (ADR-0012 / ADR-0013), which extends this
same seam. Selection is via a tagged-config factory
`build_executor(&SandboxConfig) -> Result<Arc<dyn CommandExecutor>, SandboxError>`;
per CLAUDE.md the
`match`-on-`kind` lives in the owning crate (`cogito-sandbox`) and is the
sole dispatch site. v0.1 has one tag: `Direct(DirectConfig { root,
inherit_env })`. See `docs/components/cogito-sandbox.md`.

### The builtin tool set is deliberately small

The builtin catalog holds exactly two kinds of thing:

1. **A reference implementation per execution mode**, for consumers to copy
   when writing their own tools:
   - Synchronous (`BuiltinTool -> ToolResult`): `read_file`, now joined by
     `web_fetch` (the "network tool" reference).
   - Async / long-running (`ToolProvider -> InvokeOutcome::Async` +
     `JobManager`): `run_tests`, now joined by `bash`'s background branch.
2. **Provider-free primitives** every agent needs that do not require
   choosing an external vendor: `bash`, `web_fetch`.

`web_search` is **deferred to MCP / a future factory**: it must pick an
external provider (Brave / Tavily / Google / Bing / SerpAPI…), each with
API keys, billing, and a different response shape. That is the textbook
case for "a hand the consumer brings" or an MCP server, not a provider-free
primitive. `web_fetch` deliberately **does not call any model** (it stays
provider-free and avoids coupling `ModelGateway`, which would break the
layering); turning fetched content into an answer is the model's job.

### Spawn-point ownership

`CommandExecutor` is the intended single funnel for subprocesses that
cogito itself spawns. v0.1 wires only `bash`; the rest are recorded as
known current state / future work, **deferred, not gaps**.

| Spawn point | Through `CommandExecutor`? | Notes |
|---|---|---|
| `bash` tool | Yes (this change) | First consumer of the seam. |
| `run_tests` tool | No (raw `tokio::process` today) | Working, green code; convergence to the seam is an optional later dedup, not done this round. |
| MCP stdio server connect | No (inside the rmcp client, one-shot at connect) | Not per-call; see below. cogito-mcp only hands `command` / `args` to rmcp's `transport-child-process`. |
| skill scripts (today) | Yes (via `bash`) | ADR-0023 B-defer: scripts are data; the model runs them with `bash`, so they already funnel through the seam with no special mechanism. |
| skill scripts (future ADR-0023 B/C) | Should be | A dedicated "execute script" path, when it lands, should funnel through `CommandExecutor` by the same principle. |

See spec §3.2–3.5 for the per-row reasoning.

### H08 dispatch and Adaptive (correction recorded here)

`bash` is the first tool with `ExecutionClass::Adaptive`, and it needs **no
dispatcher change**. Since Sprint 8 the dispatcher routes purely by the
*actual* `InvokeOutcome` returned per call (`Sync` -> `SyncResult`, `Async`
-> record `JobSubmitted`, register the completion sink, pause); the
descriptor's `ExecutionClass` is now only a surface advisory (e.g. H05
filtering). An Adaptive tool that returns `Sync` or `Async` per call works
out of the box. The stale "Adaptive deferred" wording in
`docs/components/H08-tool-dispatcher.md` is corrected alongside this ADR.

## Consequences

### Positive

- A single, policy-driven seam for cogito-originated subprocesses; v0.4
  isolation slots in behind the same trait with no Brain / tool change.
- Surfaces call `build_executor` once and inject the result; adding a new
  isolation variant edits only `cogito-sandbox`.
- The builtin set stays a teaching set + universal primitives, not a
  catalog the consumer must trim.
- Adaptive tools are supported with zero dispatcher work, validating the
  Sprint 8 routing decision.

### Negative / known boundaries (deferred, not gaps)

- **`run_tests` does not yet go through `CommandExecutor`.** It spawns via
  raw `tokio::process`. Converging it is optional dedup left for later; the
  code is green and changing it now buys nothing for v0.1.
- **MCP stdio child processes are not covered by the seam.** The spawn
  happens inside rmcp at connect time, not per-call, and not through
  `CommandExecutor`. The implication, especially for SaaS: a multi-tenant
  ApiServer must either use streamable-HTTP MCP only, or a future ADR must
  wrap rmcp's command spawn behind the executor too. Recorded as a known
  boundary; not solved in v0.1.
- **v0.1 is not a security boundary.** Command admission (blocking
  `rm -rf /`) and URL admission (blocking internal IPs / SSRF) are H09 hook
  responsibilities, not tool responsibilities. The real isolation /
  credential boundary is v0.4 (ADR-0012 / ADR-0013).

### Neutral

- One trait added to `cogito-protocol`; one previously-empty crate
  (`cogito-sandbox`) gains its first real implementation. No new crate.

## Alternatives considered

1. **`bash` depends directly on `cogito-sandbox`.** Rejected: violates
   ADR-0004 (a Hand tool reaching for a concrete sibling) and bakes in a
   single isolation strategy the SaaS form cannot use.
2. **Put `env` / `root` on every `CommandSpec`.** Rejected: they are
   construction-time policy, not per-call data; keeping them off the call
   surface keeps `CommandSpec` minimal.
3. **Ship `web_search` as a builtin now.** Rejected: needs an external
   provider + keys + a tagged-config factory — MCP / a later factory is the
   right home.
4. **Treat a non-zero exit code as a tool error.** Rejected: a non-zero
   exit is normal business information; `bash` returns it verbatim as
   `Output`. Only spawn failure / cancellation are `CommandError`; a
   timeout is a `CommandOutcome` with `timed_out = true` (surfaced as a
   `Timeout` tool error on the sync path, a `Failed` job on the background
   path).
5. **Make `bash`'s background branch a pollable detached daemon.**
   Rejected for v0.1: background here means "async long task, result
   delivered once on completion," bounded by `background_deadline`. A
   pollable/streamable daemon (`npm run dev`-style) needs the v1.x Resource
   Registry.

## References

- Spec: `docs/superpowers/specs/2026-05-29-core-tools-bash-webfetch-design.md`
- `docs/components/cogito-sandbox.md` (DirectExecutor + factory)
- `docs/components/H08-tool-dispatcher.md` (Adaptive routing, corrected)
- `docs/configuration/overview.md` §`[tools]`
- ADR-0004 (layering), ADR-0012 / ADR-0013 (v0.4 isolation + credentials),
  ADR-0018 (MCP), ADR-0023 (bundled script execution)
- Sprint 8 async-jobs spec:
  `docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md`

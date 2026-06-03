# ADR-0013: Credential isolation (sandbox proxy pattern)

## Status

Proposed (draft, v0.3/v0.4) — **needs more design; framed as a Credential
Broker.**

Direction set 2026-06-03: tool/execution authentication — especially **MCP
credential handling** — is not owned by cogito core. It is future work under a
dedicated **Credential Broker** seam: the Brain (the Agent loop) never sees or
handles credentials; it decides, and the broker (a Hands-layer seam) injects
auth at egress. cogito core stays a Brain + Hands framework that exposes this
seam for a SaaS layer to implement, rather than baking credential policy in.
This draft sketches the proxy mechanism; the broker abstraction (how tools and
MCP servers acquire scoped, short-lived credentials without the Brain or the
model ever touching them) needs its own design pass before ratification. See
ADR-0014 §Decision-4 for the core-responsibility framing.

Depends on ADR-0012 (sandbox lifecycle): this ADR extends the same
isolating-executor seam ADR-0012 introduces. Like ADR-0012, the whole
problem is **deferrable until cogito runs untrusted or attacker-reachable
code** — under the v0.1 `DirectExecutor` (no isolation, full env
inheritance) there is no credential boundary to speak of, and that is an
accepted, documented posture (ADR-0027). This ADR specifies *where the
seam goes* so that the day a consumer (praxis) does run multi-tenant or
model-authored code, the secrets do not already leak into it.

Relates to ADR-0014 (TenantContext): note ADR-0014 was Accepted as **Route A**
— tenant identity does **not** flow on `ExecCtx`. So per-tenant credential
selection keys off the tenant the consumer captured into the per-session
provider it injects (ADR-0028), not off an `ExecCtx.tenant` handle.

**DEFERRED (2026-06-03), not scheduled.** Gated on the same trigger as
ADR-0012 — cogito executing untrusted / attacker-reachable code (multi-tenant
tenant code, or `bash`/exec exposed to an attacker-influenceable model). The
seam location is settled; the broker abstraction and the hardening item below
are designed when that trigger fires or praxis answers the bash-exposure
question.

**Near-term hardening item — execution env policy (the cheap part of the
broker):** today `DirectExecutor` defaults to `inherit_env: true`, so the host
process environment — model API keys, DB URLs, MCP bearer tokens, cloud creds —
is visible to every `sh -c` command (model-authored bash, skill scripts). The
fix is **not** a bool flip to `inherit_env: false`: `env_clear()` also drops
`PATH` / `HOME` / `LANG` / `TMPDIR` and would break skill scripts and most
commands. The right shape is an **env policy** on `DirectConfig` /
`CommandSpec` — a curated allowlist (`PATH`, `HOME`, `LANG`, `TMPDIR`, plus
consumer-supplied vars) that is **default-deny for everything else (the
secrets)**, replacing the binary inherit-all / clear-all choice. Small design,
gated on the same bash-exposure question; specified here so it lands with the
broker rather than as a rushed default flip.

## Context

cogito today has three distinct credential flows. Only one of them is a
Hands concern; the ADR is about that one, but the boundary only makes
sense once all three are on the table.

1. **Boundary credentials — model API keys.** A `${ENV}` placeholder in
   `cogito.toml` is interpolated by `FileConfigLoader` into
   `ProviderConfig::Anthropic { api_key, .. }` /
   `OpenAiCompat { api_key, .. }`
   (`crates/cogito-model/src/provider_config.rs`). The key lives in
   process memory and `cogito-model`'s gateway attaches it as the
   `x-api-key` / `Authorization` header on the **outbound model call**.
   This credential is consumed entirely inside the Boundary layer; it
   never needs to reach a tool, a sandbox, or model-authored code. It is
   **out of scope** for this ADR except as a non-goal: do not route model
   keys through the Hands proxy.

2. **Hands credentials reaching a subprocess — the actual problem.** When
   a tool spawns a subprocess via `CommandExecutor`
   (`crates/cogito-protocol/src/command.rs`), the child's environment is a
   *construction-time* property of the executor, not per-call data:
   `CommandSpec` deliberately carries no `env` field; `SandboxConfig` /
   `DirectConfig` carry `inherit_env` (default `true`). Today
   `DirectExecutor` either inherits the **entire** cogito process
   environment (`inherit_env: true`) or wipes it (`env_clear()`). There is
   no in-between: a model-authored `bash` command either sees every secret
   in the parent environment (`ANTHROPIC_API_KEY`, `COMPANY_MCP_TOKEN`,
   cloud credentials, …) or sees nothing. For a single-tenant host binary
   running trusted code this is fine. For a SaaS-embedded multi-tenant
   ApiServer it is a credential-disclosure bug waiting to happen: tenant
   A's `bash` reading tenant B's token, or any tenant exfiltrating the
   server's own secrets.

3. **MCP egress credentials.** A streamable-HTTP MCP server's bearer token
   is resolved by name from the environment
   (`bearer_token_env_var` → `std::env::var`) inside
   `cogito-mcp/src/transport.rs`; the literal token is never allowed in
   config (`deny_unknown_fields` rejects a `bearer_token` field). For an
   **stdio** MCP server the token reaches the child via the explicit `env`
   HashMap *plus* whatever the cogito process environment leaks — and,
   critically, that spawn happens **inside rmcp's
   `transport-child-process`, not through `CommandExecutor`** (ADR-0027
   "known boundaries"). So stdio MCP is a second path by which a child
   process can inherit ambient secrets, and it does not pass the seam this
   ADR governs.

The forces:

- The agent loop legitimately needs *some* credentials to reach *some*
  egress (the MCP server it is allowed to call, an HTTP API a tool wraps).
  The goal is not "no secrets anywhere" but "the tool / child code never
  holds the raw secret; something it cannot read injects auth at the
  egress point."
- "Which secret, for which tenant" is policy that the runtime knows and
  model-authored code must not. That argues for a **proxy seam owned by
  the runtime, sitting between the tool/child and the network**, rather
  than env-var injection into the child.
- The seam must not perturb the Brain. H08 dispatches `ToolProvider`;
  `CommandExecutor` already lives below the tool boundary and is
  Brain-invisible (ADR-0027). Credential injection must stay below that
  same line.

## Decision

### 1. The principle: inject auth at egress, never hand the child the secret

Tools and sandboxed/model-authored code receive **no raw secrets**. A
runtime-owned proxy sits at the egress point and injects credentials there.
Concretely, two distinct egress shapes need two narrow seams, both in the
Hands layer, both invisible to the Brain:

- **Subprocess environment** (the `bash` / script path): a child gets a
  *scrubbed, explicitly-allowlisted* environment, never blanket
  inheritance, plus optionally a loopback egress proxy address instead of a
  token.
- **Network egress** (HTTP tools, MCP HTTP, and ideally MCP stdio): a
  credential-injecting forward proxy adds the `Authorization` header on the
  way out, so the client side holds only a proxy handle, not the bearer.

### 2. Subprocess seam: a `CredentialPolicy` resolved at executor construction, not per call

The `CommandExecutor` per-call surface stays minimal — `CommandSpec` gains
**no** `env` field (preserving ADR-0027 Alternative 2). Instead the
isolating executor that ADR-0012 introduces is constructed with a
credential policy that decides the child's environment:

```rust
// cogito-sandbox (construction-time policy on the isolating executor)
pub enum EnvPolicy {
    /// v0.1 behavior, single-tenant host only: child inherits the full
    /// parent environment. Documented as "not a security boundary."
    InheritAll,
    /// Child starts from empty and receives only these keys, copied from
    /// the parent environment. The default for any isolating executor.
    Allowlist(Vec<String>),
    /// Child starts from empty and receives exactly these key/values,
    /// resolved by the runtime (per-tenant) at spawn time. No ambient
    /// inheritance at all.
    Explicit(Box<dyn EnvResolver>),
}
```

`inherit_env: bool` on `DirectConfig` is reinterpreted, not removed:
`true` maps to `InheritAll`, `false` to `Allowlist(vec![])`. The new
variants are additive and only meaningful for the v0.4 isolating executor.
`EnvResolver` is keyed by the session/tenant identity already on `ExecCtx`
(`session_id`, plus `TenantContext` once ADR-0014 lands), so the same
executor serves many tenants without any tenant seeing another's keys.

This keeps the existing rule intact: env is a **construction/policy**
concern on `SandboxConfig`, resolved at spawn, never threaded through the
Brain-visible `CommandSpec`.

### 3. Network seam: a `CredentialProxy` the runtime injects, the child cannot bypass

For credentials whose only purpose is an outbound HTTP header (MCP bearer
tokens, future HTTP-tool auth), the preferred pattern is **not** to put the
token in the child at all, but to route the child's egress through a
runtime-owned forward proxy that adds the header:

```rust
// cogito-protocol (Hands-internal trait; not in the event log,
// no SCHEMA_VERSION impact — same discipline as CommandExecutor)
#[async_trait]
pub trait CredentialProxy: Send + Sync {
    /// The address the child should send egress to (e.g. a loopback
    /// http(s) proxy). The child sees only this; never the secret.
    fn egress_endpoint(&self) -> EgressEndpoint;
}
```

The implementation lives in the Hands layer (a new module in
`cogito-sandbox`, or — pending ADR-0012's "remote / per-tenant execution"
shape — a sidecar the isolating executor talks to). The proxy holds the
tenant→secret mapping; the child holds a `HTTP(S)_PROXY`-style endpoint.
This is the form that scales to a multi-tenant ApiServer where the child
must never be able to read the raw token even by dumping its own
environment.

For v0.4's first cut, a token-injecting variant (env-var injection of a
single, tenant-scoped, short-lived token resolved by `EnvResolver`) is an
acceptable interim when the full forward-proxy is too heavy; the trait
boundary lets either implementation slot in without touching tools.

### 4. Where it lives, and who is untouched

- Traits (`CredentialProxy`, `EnvResolver` signatures) → `cogito-protocol`
  Hands section, alongside `CommandExecutor`, runtime-only, no wire impact.
- Implementations + the `match`-on-tag factory → `cogito-sandbox`
  (extending `build_executor`), per the CLAUDE.md "tagged-config factories
  live in the owning crate" rule. Surfaces call `build_executor` once and
  inject the result, exactly as today.
- Brain (H01–H11): unchanged. H08 still sees only `ToolProvider`;
  credential injection is strictly below the tool boundary.
- H09 hooks: remain pure policy gates (admission decisions). They may
  decide *whether* a command/egress is allowed; they do not perform the
  injection — that is the proxy's job, consistent with the AGENTS.md rule
  that hooks have no I/O.

### 5. Close the MCP stdio gap as part of this work

The streamable-HTTP MCP path already keeps the literal token out of config
and resolves it at the Boundary; routing it through the `CredentialProxy`
(rather than reading `std::env::var` inline) is the v0.4 upgrade. The
**stdio** MCP path is the open hole flagged by ADR-0027: its child is
spawned inside rmcp and inherits ambient env. Closing it means either (a)
funneling rmcp's command spawn through the isolating `CommandExecutor` +
`EnvPolicy` (preferred, unifies the seam), or (b) declaring stdio MCP
unsupported in the multi-tenant profile and requiring streamable-HTTP MCP
there. This ADR records the requirement; the choice between (a) and (b) is
an open question pending ADR-0012's executor shape.

## Consequences

**Easier**:
- A multi-tenant ApiServer can run model-authored `bash` without exposing
  the server's or other tenants' secrets: the child gets an allowlisted /
  explicit env and an egress proxy, never the raw token.
- Per-tenant credential selection has an obvious home (the proxy /
  `EnvResolver`, keyed by `TenantContext` from ADR-0014).
- The model-key flow (Boundary) and the tool-egress flow (Hands) stay
  cleanly separated; no temptation to route model keys through tool env.

**Harder**:
- Two seams instead of one: env policy on the executor *and* a network
  proxy. Tools that today "just read an env var" must be taught to use the
  proxy endpoint instead, or accept that the var is absent under isolation.
- A forward proxy is real infrastructure (TLS interception or CONNECT
  handling, per-tenant routing); the interim env-injection variant is
  simpler but weaker (child can still read its own env).

**Given up**:
- Blanket env inheritance as the default for isolating executors — by
  design. `InheritAll` survives only for the explicitly-trusted
  single-tenant host profile.
- Nothing is given up for the v0.1/v0.2 trusted-host posture: until an
  isolating executor exists (ADR-0012) this ADR changes no running code.

## Alternatives considered

1. **Add `env` to `CommandSpec` and let the tool pass secrets per call.**
   Rejected: re-litigates ADR-0027 Alternative 2 (env is policy, not
   per-call data) and, worse, would put secret selection in tool code that
   may itself be model-influenced. The runtime, not the tool, must own
   which secret reaches which child.
2. **Keep `inherit_env` boolean only; document "don't put secrets in the
   environment of a multi-tenant server."** Rejected: unenforceable
   operational advice; the whole point of a SaaS embed is one process
   holding many tenants' secrets.
3. **Inject a long-lived bearer token into the child's env under
   isolation.** Rejected as the default: a child that can read its own
   environment can exfiltrate the token. Acceptable only as a short-lived,
   tenant-scoped interim (§3), behind the same trait, never as the end
   state.
4. **Solve this entirely at the H09 hook layer (block egress to
   unauthorized hosts).** Rejected as sufficient: hooks gate
   *whether* an action is allowed, but a gate does not inject auth and does
   not prevent the child from reading an ambient secret. Admission control
   (H09) and credential isolation (this ADR) are complementary, not
   substitutes.

## Open questions

These are genuinely unresolved and need human ratification, several
because they depend on ADR-0012's executor shape:

- Whether the network seam ships as a true forward proxy or as
  tenant-scoped short-lived env-token injection in the first v0.4 cut.
- Whether MCP stdio is funneled through `CommandExecutor` (unify the seam)
  or declared unsupported in the multi-tenant profile (§5, options a/b).
- The exact `EnvResolver` / `EgressEndpoint` signatures, which should be
  fixed together with ADR-0012's isolating-executor construction API so
  the policy is set in one place.
- Whether a `TenantContext`-keyed credential store belongs in
  `cogito-sandbox` or in a separate, consumer-supplied trait object
  (praxis brings its own secret manager) — the latter keeps cloud-SDK
  dependencies out of cogito, mirroring the config-loader stance in
  `docs/configuration/overview.md` §9.

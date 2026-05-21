# ADR-0018: MCP integration (`cogito-mcp`)

## Status

Accepted (2026-05-21).

## Context

Through Sprint 0–3 + 4.5, `cogito chat` exposes exactly **one** built-in
tool to Brain: `read_file`. Brain's tool-loop, prompt composition, and
strategy selection are exercised against a single-tool catalog; the
question "does Brain behave well when the catalog grows to N varied
tools?" is not concretely answered by the existing test surface.

Three converging needs make this gap painful:

1. **Brain validation in parallel** — Async-job infrastructure (the
   original Sprint 4 scope) is a substantial piece of work
   (`JobManager` + JSONL job log + cross-process resume + H08 async
   path), and while it lands, the rest of the team is blocked from
   end-to-end Brain testing against varied real tools.
2. **MCP (Model Context Protocol) is the converging standard** —
   Anthropic, OpenAI Codex, Cursor, Continue, and dozens of OSS
   servers (filesystem, git, shell, browser automation, …) speak it.
   The official Rust SDK `rmcp` (`modelcontextprotocol/rust-sdk`,
   Apache-2.0) reached crates.io 1.5 and is usable today.
3. **`cogito-mcp` already a placeholder crate** — ARCHITECTURE.md
   §"Version evolution path" scheduled MCP for v0.2 with a stub
   `cogito-mcp` crate. Pulling that forward to v0.1 Sprint 4 is a
   straightforward priority swap with the original Async Jobs sprint.

The renumber commit (2026-05-21) moved the schedule:

- Sprint 4 → **MCP sync tools** (this ADR)
- Sprint 5 → Async Jobs (was 4)
- Sprint 6 / 7 / 8 → renumbered from 5 / 6 / 7

Sprint 4.5 (config-file loading, ADR-0017) just shipped, so the
`[[providers]]` schema + `${ENV_VAR}` interpolation + layered merge
already exist; `[[mcp_servers]]` slots into the same machinery cleanly.

The user's first MCP server is streamable-HTTP with bearer auth,
which determines our v0.1 transport priorities. Stdio is included
for the broad OSS ecosystem (most public MCP servers ship as stdio
binaries).

A separate, sharp tension surfaced during spec review: **how should
MCP failures interact with Runtime startup?** Three positions were
considered (hard-fail any config error; hard-fail config but soft-skip
environmental errors; soft-skip everything). This ADR locks the third
position with a load-bearing mechanism.

## Decision

### 1. Use `rmcp` directly; treat Codex as architecture inspiration only

```toml
[dependencies]
rmcp = { version = "1.5", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client-reqwest",
    "schemars",
    "macros",
] }
```

`rmcp` is an ordinary upstream Cargo dependency
(`modelcontextprotocol/rust-sdk`, Apache-2.0). Cargo metadata handles
attribution automatically; no derivative-work obligations apply to
cogito as a whole.

Codex (`github.com/openai/codex`, Apache-2.0) has a 535-line
`rmcp-client` crate that is mature and well-architected. **We do not
copy source code from it.** We consult it for architectural patterns
(`Connecting`/`Ready` state machine; `PendingTransport` enum;
qualified-tool-name algorithm; per-server fault containment shape) and
re-implement in cogito's idioms (`thiserror`-based library errors,
`#[non_exhaustive]` enums, Brain/Hands edge enforcement, no manual
`mcp-types` middle layer).

`cogito-mcp/src/lib.rs` opens with a credit comment:

```rust
//! cogito-mcp — MCP client + ToolProvider adapter.
//!
//! Architecture-inspired by `openai/codex` `codex-rs/rmcp-client/`
//! (Apache-2.0, pattern-only reimplementation; no source-code lift).
//! Upstream protocol SDK: `rmcp` 1.5
//! (modelcontextprotocol/rust-sdk, Apache-2.0) — used as a normal
//! Cargo dependency.
```

This is professional credit, not a derivative claim.

### 2. Transport scope

**Stdio + streamable-HTTP, both first-class.**

| Transport | Source | Use case |
|---|---|---|
| stdio (`transport-child-process`) | local child process | OSS ecosystem (filesystem-mcp, git-mcp, …) |
| streamable-HTTP (`transport-streamable-http-client-reqwest`) | remote endpoint, optional bearer auth | hosted services, internal company MCP servers |

**Excluded from v0.1:**

- Legacy SSE-only transport. The MCP spec from 2025-03-26 onward
  deprecates SSE in favor of streamable-HTTP; `rmcp` 1.5 does not
  expose an SSE feature flag.
- OAuth login flow. `rmcp` provides an `auth` feature; Codex uses 922
  lines (`oauth.rs` + `perform_oauth_login.rs` + `auth_status.rs`) to
  implement it. The flow is sufficiently complex to deserve a
  dedicated follow-up ADR when a hosted MCP service requiring OAuth
  becomes a concrete need.

Bearer auth for streamable-HTTP uses an env-var indirection:

```toml
[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"  # NOT bearer_token = "<secret>"
```

The literal `bearer_token` field is **forbidden in config**
(`deny_unknown_fields` rejects it). This aligns with ADR-0017 §6
secret posture: secrets live in environment, files only reference
them.

### 3. MCP failures are non-fatal to Runtime

**This is the load-bearing architectural commitment of this ADR.**

Any failure originating in the MCP layer — at any of the following
points — becomes a `McpStartupFailure` entry; Runtime construction
proceeds normally with whatever MCP servers came up successfully:

| Failure point | Variant |
|---|---|
| `[[mcp_servers]][i]` fails to deserialize (unknown transport, unknown field, type mismatch) | `ConfigParse { index, error }` |
| `bearer_token_env_var` references a missing or empty env var | `BearerEnvMissing { name, env_var }` |
| Two entries share the same `name` | `DuplicateName { name, index }` |
| `initialize` + `tools/list` exceeds `startup_timeout_sec` | `StartupTimeout { name, timeout_sec }` |
| Stdio child fails to spawn / HTTP fails to connect / handshake RPC errors | `TransportError { name, error }` |
| rmcp returns a handshake-level error (protocol mismatch, server doesn't support tools) | `HandshakeFailed { name, error }` |

`McpStartupFailure` is the **unified channel** for all of these. It
is `#[non_exhaustive]` (future modes can be added without breaking
downstream `match` arms).

**Scope boundary.** This soft-skip principle applies **only to MCP**.
The following errors remain hard-fail (Runtime cannot be built):

| Error class | Behavior | Why |
|---|---|---|
| `[runtime]` field error | hard-fail at config load | runtime config is load-bearing |
| `[[providers]]` parse error | hard-fail at config load | without a model gateway, the agent cannot function |
| Provider env var missing (`api_key_env_var`) | hard-fail at gateway build | same — no provider, no agent |
| TOML file syntactically broken | hard-fail | nothing else can load |
| `[[mcp_servers]]` parse error | **soft-skip,** Runtime continues | this ADR's commitment |
| MCP env / handshake / etc. | **soft-skip,** Runtime continues | same |

The mental model: **without a provider, the agent is dead; without
MCP, the agent is lame.** Lame walks; dead doesn't.

**Compiler-enforced invariant.** To make the principle structurally
unavoidable, `cogito-mcp::build_mcp_provider` returns:

```rust
pub struct McpProviderBuildResult {
    pub provider: Option<Arc<dyn ToolProvider>>,
    pub failures: Vec<McpStartupFailure>,
}

pub async fn build_mcp_provider(
    cfgs: &[McpServerConfig],
) -> McpProviderBuildResult;   // NOT Result<_, McpError>
```

A Surface (`cogito-cli`, future `cogito-tui`, future consumer Server)
**cannot** propagate an MCP failure via `?` to abort Runtime
construction, because there is no `Result` to propagate. Failures are
collected, surfaced to the user via a startup banner, and life goes
on.

**Compensating visibility (mandatory).** Soft-skip without visibility
is a debuggability disaster: a user with a typo in `mcp_servers[2]`
silently loses an entire server and assumes the agent is broken.
Every Surface MUST emit a startup banner on stderr after Runtime
build:

```text
[mcp] ✓ filesystem ready (4 tools)
[mcp] ✓ company_api ready (12 tools)
[mcp] ✗ broken_server skipped: env var `COMPANY_MCP_TOKEN` is unset
[mcp] ✗ mcp_servers[3] skipped: unknown transport "websocket"
[mcp] note: 0 of N configured servers came up; running with builtin tools only   # only when all fail
```

This is part of the contract — a Surface that omits the banner
violates ADR-0018.

The lenient deserialization of `[[mcp_servers]]` is also part of this
commitment: `cogito-config::RuntimeConfigPartial.mcp_servers` is
`Option<Vec<toml::Value>>` (raw); per-entry typed deserialization
happens at finalize time, with failures lifted into
`McpStartupFailure::ConfigParse`. A typo in one entry cannot poison
the rest of the TOML parse.

### 4. Tool namespacing

Qualified tool name format:

```text
mcp__<server_name>__<tool_name>
```

- **Delimiter:** `__` (double underscore). Constrained by OpenAI
  Responses API tool-name regex `^[a-zA-Z0-9_-]+$`; this is the
  safest character we can use without risking provider rejection.
- **Sanitization:** any character outside `[a-zA-Z0-9_-]` is
  replaced with `_`.
- **Length cap:** 64 characters. Names exceeding this are truncated
  and a SHA-1 hash suffix (full hex digest, deterministic) replaces
  the tail.
- **Deduplication:** if two qualified names collide after
  sanitization+truncation (rare, but possible when a server names
  tools `foo.bar` and `foo_bar`), the later entry is skipped and a
  `warn!` event is emitted.
- **Builtin tool invariant:** Built-in cogito tool names MUST NOT
  start with the prefix `mcp__`. This is documented in
  `cogito-tools::provider::BuiltinToolProviderBuilder` and enforced
  by `debug_assert!` at registration time. The invariant guarantees
  that no qualified MCP name can ever collide with a builtin.

The algorithm is public knowledge (it is the de facto MCP-multi-server
naming convention shared across implementations); copying the
*pattern* — not the source — from Codex is unproblematic.

`split_qualified_tool_name(qname) -> Option<(server, tool)>` is the
inverse, used by `McpToolProvider::invoke` to route a call back to
the originating handle.

### 5. ToolResult mapping (MCP → cogito)

MCP `CallToolResult` carries:

```text
{ content: Vec<ContentBlock>, is_error: bool, structured_content?: Value }
```

where `ContentBlock` variants include `Text`, `Image`, `Resource`,
and (in newer spec) `Audio`.

Mapping into cogito's v0.1 `ToolResult`:

| MCP shape | cogito `ToolResult` |
|---|---|
| `is_error: true` | `Error { kind: InvocationFailed, message: <text blocks joined>, retryable: false }` |
| `is_error: false`, all `Text` blocks | `Output(vec![Value::String(text), ...])` (one element per block) |
| `is_error: false`, contains `Image` / `Resource` | `Output(...)` with image/resource blocks serialized as `{ "kind": "image" \| "resource", ... }` JSON objects. Multimodal model visibility lands when `ContentBlock` (in `cogito-protocol`) gains `Image` etc. via ADR-0009 (v0.2). |
| `is_error: false`, includes `structured_content` | The structured payload is appended to `Output` as `{ "kind": "structured", "data": ... }`. |

**Why `retryable: false` as default.** We have no insight into MCP
server internal state. A retry policy is the caller's call (Brain
via Strategy or H09 hook); conservative default avoids tight retry
loops against unhealthy servers.

Future `ToolErrorKind::McpServerError` (an additive variant on the
`#[non_exhaustive]` enum) may further classify when concrete patterns
emerge. v0.1 lumps everything into `InvocationFailed` to keep the
surface small.

### 6. Schema posture (H07 interaction)

MCP `Tool::inputSchema` is JSON Schema (Draft 2020-12), the same
spec cogito's `ToolDescriptor::schema` uses. **The MCP schema is
copied verbatim into the descriptor; no boundary sanity check is
performed.**

Rationale:

- MCP spec requires `inputSchema.type = "object"` for every tool. A
  spec-conforming server cannot violate this; guarding against
  violators is speculative.
- Failure radius is bounded: a malformed schema lets H07 accept
  invalid args → the MCP server rejects the call → `ToolResult::Error`
  → Brain handles via existing path. One bad tool call, not a session
  crash.
- CLAUDE.md guidance: "only validate at system boundaries (user
  input, external APIs)." MCP is a configured trusted dependency,
  not untrusted input.
- Additive escape hatch: if a real-world bad server appears, add
  `McpStartupFailure::SchemaInvalid` (the enum is `#[non_exhaustive]`,
  no breaking change).

H07 Tool Resolver continues using the existing `jsonschema` crate in
its default mode; MCP introduces no special validation path.

### 7. Observability

**Tracing (yes, additive).** H05 Tool Surface Builder emits, on each
turn's tool surface assembly:

```rust
tracing::info!(
    target: "h05.tool_surface",
    mcp.tool_count = ...,
    mcp.tool_desc_total_bytes = ...,
    builtin.tool_count = ...,
    "tool surface built"
);
```

This makes MCP-tool catalog size visible to ops without prescribing
a policy (no truncation, no length caps). When users hit prompt
budget pressure, the tracing field shows the cost.

**Event log (no, kept stable).** `EventPayload::ToolUseRecorded`
already carries `tool_name: String`, which encodes
`mcp__server__tool` — server provenance is grep-able from the event
log as-is. Adding an MCP-specific event variant would require an
ADR-0007 b-档 schema evolution; the cost outweighs the benefit at
v0.1. If a future use case (e.g., per-MCP-server billing analytics)
demands it, add `EventPayload::McpInvocationCompleted` then.

### 8. Layer placement

`cogito-mcp` is a **Hand crate**, peer to `cogito-tools`,
`cogito-model`, `cogito-jobs`. This follows ADR-0004.

| Crate | Allowed to `use cogito_mcp::*`? | Why |
|---|---|---|
| `cogito-protocol` | No | Lower layer (Brain may import). |
| `cogito-core::harness` (Brain) | **No** | Brain may only import `cogito-protocol`. |
| `cogito-core::runtime` | No | Receives `Arc<dyn ToolProvider>` from Surface; does not couple to MCP specifically. |
| `cogito-config` | **Yes — value types only** | `McpServerConfig` and `McpStartupFailure` are shared value types. Hand-to-Hand sharing is allowed per ADR-0004. |
| `cogito-cli` (Surface) | Yes | Surfaces compose providers. |
| `cogito-tui` / consumer Server (future Surface) | Yes | Same as CLI. |

The `scripts/check-layer.sh` script must accept
`cogito-config → cogito-mcp` (value-type only). If it currently
rejects this, the script's allowlist is extended; the principle is
unchanged.

### 9. What this ADR does NOT decide

- **OAuth flow.** Deferred to a follow-up ADR when a hosted MCP
  server requiring OAuth becomes a concrete need.
- **MCP resources / prompts / sampling.** Resources depend on
  `StorageSystem` (ADR-0009, v0.2). Prompts are conceptually closer
  to strategies; revisit post-v0.2. Sampling (server→client LLM
  call) is **explicitly out**: it violates ADR-0004 (a Hand asking
  Brain to think on its behalf inverts the dependency arrow).
- **`strict_mcp_startup: bool` configuration field.** Reserved as a
  v0.4 SaaS-ready concern; not introduced in v0.1 schema. Adding it
  later is additive (`#[serde(default)]`).
- **MCP server auto-reconnect on mid-session disconnect.** Not in
  v0.1; mid-call failures return `ToolResult::Error`. Revisit at the
  v0.6 hardening sprint.
- **Cross-language MCP provenance in event log.** No MCP-specific
  event variants in v0.1; revisit if a downstream consumer needs
  structured provenance beyond `tool_name`.

## Consequences

**Easier:**

- Brain's tool-loop, prompt composition, and strategy selection get
  exercised against a real, varied tool catalog without waiting for
  async-job infrastructure.
- Sprint 5 (Async Jobs) can land in its own time; it does not block
  parallel Brain validation.
- The unified `McpStartupFailure` channel + mandatory startup banner
  give ops a single, predictable place to look when "the agent
  doesn't see a tool I configured."
- The `McpProviderBuildResult` (vs `Result<_, _>`) signature makes
  the soft-skip commitment structurally enforced — a future
  refactorer cannot accidentally make MCP failures fatal.

**Harder / cost:**

- Runtime startup gains a network/IPC step (stdio spawn + handshake,
  or HTTP round-trip). Mitigated by per-server `startup_timeout_sec`
  (default 10s) + concurrent `JoinSet`.
- `cogito-config` finalize gets one more responsibility (per-entry
  TOML re-deserialization). Tested in isolation.
- Layer-check script may need a whitelist entry for
  `cogito-config → cogito-mcp`. Small one-time cost.

**Given up:**

- The simpler position "config errors should hard-fail." MCP config
  errors are now soft-skipped, traded for the banner + tracing
  visibility. A future operator who *wants* fail-fast must wait for
  v0.4 SaaS-ready and the `strict_mcp_startup` mode.

## References

- ADR-0004 §"Brain / Hands / Session boundaries" — `cogito-mcp` is a
  Hand crate; Brain may not import it.
- ADR-0007 §"Event log as cross-language contract" — why no MCP
  event variants in v0.1.
- ADR-0017 §6 — secret interpolation via env var, reused by
  `bearer_token_env_var`.
- ADR-0017 §3 — layered partial merge; `mcp_servers` array follows
  the same array-replace pattern as `providers`.
- ROADMAP §"Sprint 4 · MCP sync tools" (renumbered from "Async Jobs"
  on 2026-05-21).
- `docs/superpowers/specs/2026-05-21-sprint-4-mcp-sync-tools-design.md`
  — decision trajectory (Q1–Q13), implementation breakdown, testing
  matrix.
- `rmcp` crate: https://crates.io/crates/rmcp ·
  https://github.com/modelcontextprotocol/rust-sdk (Apache-2.0).
- Codex `codex-rs/rmcp-client/` (Apache-2.0) — architecture
  inspiration, pattern-only.
- MCP specification 2025-06-18:
  https://modelcontextprotocol.io/specification/2025-06-18
- CLAUDE.md §"Tagged-config factories" — why `transport`-tagged
  enum dispatch lives in `cogito-mcp`, not in Surface code.

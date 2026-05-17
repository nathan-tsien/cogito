# ADR-0004: Brain / Hands / Session crate boundaries

## Status

Accepted

## Context

cogito's stated design philosophy (README, AGENTS.md) borrows from
Anthropic's Managed Agents framing: **Brain** decides, **Hands** execute,
**Session** remembers. The three are decoupled so that any Brain instance
can pick up any session by reading the event log, and so that Hands can be
swapped or sandboxed independently of Brain logic.

ADR-0002 (event-sourced conversation) and ADR-0003 (state-machine Turn
Driver) make this philosophy real on the Session side. They do **not** yet
make it real on the **import graph**:

- `cogito-core` currently depends directly on `cogito-conversation`,
  `cogito-model`, `cogito-tools`, `cogito-sandbox`, `cogito-jobs`. That
  means Brain code can `use cogito_sandbox::Subprocess` and call concrete
  Hand implementations without going through a trait. The "Brain/Hands"
  rule then lives only in prose, not in the compiler.
- `cogito-core` also houses the *Agent Runtime* (rehydrator, locks, bus),
  which is the platform that hosts Brain instances. Brain and the
  platform that runs Brain are different concerns; bundling them in one
  crate makes "Brain may only see Hands through traits" un-enforceable.
- The MCP client crate has no documented place in the Brain/Hands map.
  Without a rule, Sprint 5 may give MCP a special path instead of
  treating MCP tools as just another `ToolProvider`.
- The Hook Pipeline (H09) sits in Brain, but hooks can in principle
  invoke external services. If they do so directly, Brain becomes
  impure.

We accept these as design hazards, not yet as code defects (Sprint 0,
all crates are stubs). We resolve them in design first, code second.

## Decision

### 1. Every workspace crate belongs to exactly one layer

| Layer | Role | Crates |
|---|---|---|
| **Protocol** | Pure types + trait contracts. No I/O. No other cogito deps. | `cogito-protocol` |
| **Brain** | The Harness (H01–H10). Decides, never executes directly. | `cogito-core` (`harness/` module — see §4) |
| **Session** | Persistent event log; the single source of truth. | `cogito-store-jsonl` (v0.1 sole backend) |
| **Boundary** | Adapter to an external thinking-aid the Brain speaks to (the LLM). Not Hands — Brain's "mouth/ear". | `cogito-model` |
| **Hands (Brain-facing)** | Implement Protocol-defined contracts (`ToolProvider`, `JobManager`). Brain sees these. | `cogito-tools`, `cogito-jobs`, `cogito-mcp` |
| **Hands (internal primitive)** | Used by tool implementations; not Brain-facing. | `cogito-sandbox` |
| **Runtime** | The platform that hosts Brain instances: DI, panic catch, per-session resource budget. | currently `cogito-core` (`runtime/` module — to split, see §4) |
| **Surface** | User-facing entry points. Wire everything together. | `cogito-cli`, `cogito-tui` |
| **Testing** | Test-only fakes and fixtures. | `cogito-test-fixtures`, `cogito-mock-model` |

### 2. Import rules (enforced by `Cargo.toml`)

A crate in layer L may depend only on crates in the layers listed:

| Layer | May import from |
|---|---|
| Protocol | (nothing internal) |
| Brain | Protocol |
| Session | Protocol |
| Boundary | Protocol |
| Hands | Protocol (and Boundary only if a Hand wraps an external model, which currently it does not) |
| Runtime | Protocol, Brain, Session, Boundary, Hands |
| Surface | Protocol, Runtime (and Brain/Session/Boundary/Hands only via Runtime's re-exports if needed) |
| Testing | Protocol, plus the layer it fakes |

**Brain importing Hands is a build error**, not a code-review nit. This
is the load-bearing rule of the philosophy.

### 3. Contracts live in `cogito-protocol`

The following traits and their input/output types must be defined in
`cogito-protocol`, and every concrete implementation in another crate
must implement the protocol's trait:

- `ConversationStore` — Session contract (implemented by `cogito-store-jsonl` in v0.1; later by Postgres / HTTP backends)
- `ConversationEvent` — wire format of the event log, with `schema_version: u32`
- `ModelGateway` — Boundary contract (implemented by `cogito-model`)
- `ToolProvider` (+ `ToolDescriptor`, `ToolResult`, `InvokeOutcome`, `ExecCtx`) — Hands tool catalog + invoke surface (implemented by `cogito-tools`, `cogito-mcp`, and any consumer-supplied provider)
- `JobManager` (+ `JobId`, `JobStatus`) — Hands async work tracking; exposes `status` / `result` / `cancel` only. Job submission happens *inside* `ToolProvider` implementations, not on this trait
- `HookHandler` — pluggable policy gate (implementations TBD; see H09)

**Hand-internal primitives stay out of Protocol.** `Sandbox`, HTTP clients,
filesystem adapters, and similar building blocks used *by tool
implementations* are not Brain-facing and do not belong in `cogito-protocol`.
The `Sandbox` trait lives inside `cogito-sandbox`; Tool implementations that
need subprocess isolation inject a `dyn Sandbox` from there. **Brain never
holds one.** This is the Hands layer's three-level structure
(see ARCHITECTURE.md §"Hands layer internal structure"):

```
Level 1 (in protocol) :  ToolProvider · JobManager
Level 2 (Hand crates) :  cogito-tools · cogito-mcp · cogito-jobs
Level 3 (internal)    :  cogito-sandbox · HTTP / FS adapters
```

Until a Brain-facing trait exists in `cogito-protocol`, the corresponding
capability is unreachable from Brain. The way to unlock a capability is to
write its contract, not to relax the import rule.

### 4. `cogito-core` currently bundles Brain and Runtime; split when…

For Sprint 0 we keep `cogito-core` as one crate with two modules:

```
cogito-core/
  src/
    harness/   ← Brain. May only `use cogito_protocol::*`.
    runtime/   ← Runtime. May import any non-Surface layer.
```

We split `cogito-core` into `cogito-core` (Brain only) and
`cogito-runtime` when **any** of these becomes true:

- Brain code is tempted to reach into `runtime::` to short-circuit DI
- A second Runtime is needed (e.g. an in-process test runtime vs the
  rehydrating production runtime) and they want to share Brain
- We start packaging Brain for embedding in a host that supplies its
  own Runtime

Until then, the module boundary is enforced by review. The Brain module
may import only `cogito-protocol` — when this is violated, the answer
is to add the missing trait to `cogito-protocol`, not to relax the rule.

### 5. MCP is Hands, not a special path

`cogito-mcp` implements `ToolProvider`. From Brain's view, MCP tools are
indistinguishable from builtin tools. There is no MCP-aware code in
Brain. This is non-negotiable.

### 6. Hooks are Brain-side policy only

H09's `HookHandler` runs inside Brain. Hooks may **read** state and
**reject / modify** an upcoming action, but they may not perform external
I/O. A hook that needs to call out (e.g. a remote audit log, a remote
content classifier) must do it by enqueueing through a `ToolProvider` or
`JobManager` like every other side effect.

Rationale: a side-effecting hook is just an undocumented Hand. Letting
it live inside Brain reintroduces the coupling this ADR exists to forbid.

## Consequences

- **Easier**: layer violations are caught by the compiler (Cargo) instead
  of by humans. New contributors and AI agents cannot accidentally
  couple Brain to a concrete Hand. The Runtime/Brain split has a written
  trigger so we don't drift.
- **Harder**: every new cross-layer call requires a trait in
  `cogito-protocol`. We pay an upfront cost when adding a new Hand.
- **Given up**: convenient `cogito-core` access to concrete crates. We
  trade it for the load-bearing property of the experiment.
- **Follow-on work**: Sprint 1 must add the trait skeletons to
  `cogito-protocol` and adjust `cogito-core/Cargo.toml` to drop direct
  Hand/Session/Boundary dependencies. ADR-0001's "12 member crates"
  count and the `cogito-core` description in ARCHITECTURE.md are updated
  to reflect the layer mapping.

## References

- ADR-0001 (workspace layout) — extends and refines the role column
- ADR-0002 (event sourcing) — implements the Session boundary
- ADR-0003 (state machine Turn Driver) — implements Brain's internal
  resumability
- AGENTS.md §"Inviolable design principles" — prose form of these rules

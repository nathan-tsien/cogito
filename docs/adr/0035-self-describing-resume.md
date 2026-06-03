# ADR-0035: Self-describing resume (rebuild provider surface on any replica)

## Status

Proposed (draft, v0.4).

Amends ADR-0028 §5 ("Resume: the caller is the source of truth for the
active surface"). This ADR does not change the v0.2 behavior; it records
a deferred design and, more importantly, a recommendation on whether to
build it at all.

## Context

ADR-0028 shipped per-session provider injection: `SessionSpec` carries
optional `Arc<dyn ToolProvider>` / `Arc<dyn SkillProvider>` /
`HarnessStrategy` / `Arc<dyn Workspace>` overrides
(`crates/cogito-core/src/runtime/session_spec.rs`), and
`Runtime::open_session_with(id, mode, spec)`
(`crates/cogito-core/src/runtime/builder.rs:100`) installs them for one
session. §5 of that ADR made an explicit narrowing: provider **identity
is not persisted** by the core. A provider is *code*, not conversation
state, so it cannot be replayed from the event log, and the
state-rebuilds-from-the-log rule (AGENTS.md) governs conversation state,
not injected dependencies.

The consequence for resume: when a session is resumed — including on a
**different replica** with a fresh `Runtime` (empty `sessions` registry,
the same shared `ConversationStore`) — the caller must **re-supply** the
current `SessionSpec`. The `session_spec_mutated_then_resume` chaos
scenario (ADR-0028 §"Resume-chaos scenario") proves this works: open with
spec A, mutate to spec B mid-session via `update_session`, crash, then
resume with `open_session_with(id, Resume, B)` and all four oracles hold.
The core stamps `tenant_id` / `user_id` into `SessionMeta`
(`builder.rs:226`; `cogito_protocol::SessionMeta`,
`crates/cogito-protocol/src/session.rs:21`) so a resuming caller can read
back the tenant identity, but it does not record *which* tools, MCP
servers, or skill plugins were active.

The open question ADR-0028 deferred to v0.4: can a replica rebuild a
session's provider surface **without** the caller knowing what that
surface was — i.e. self-describing resume? That would require persisting
provider *identity* (a recipe / descriptor, never serialized `Arc`s) and
a caller-supplied resolver that turns the descriptor back into trait
objects on any replica.

## Decision

### 1. Recommendation: do not build this for v0.4. Re-supply the spec.

The honest assessment is that the motivating consumer (praxis) almost
certainly does **not** need self-describing resume, and neither does any
consumer that follows the same shape:

- The consumer's gateway already owns the `tenant -> surface` mapping. It
  decides which tools / MCP servers / skills a tenant gets; that mapping
  is the gateway's source of truth, not cogito's.
- On any request — first turn or resume, same replica or another — the
  gateway re-derives the surface from `tenant_id` (which it has from auth,
  and which cogito also persists in `SessionMeta`) and passes it in via
  `open_session_with(id, Resume, spec)`. This is exactly the
  `session_spec_mutated_then_resume` pattern, already green in the chaos
  suite.
- The consumer brings its own `ConversationStore` persistence and its own
  gateway routing. Cogito's job at resume is to replay conversation state
  from the shared store and accept the surface the caller hands it. Adding
  a second, cogito-owned descriptor of the surface would duplicate state
  the gateway already holds authoritatively, and create a
  two-sources-of-truth problem (which wins when they disagree?).

So the v0.4 recommendation is: keep ADR-0028 §5 as the contract. Document
the re-supply requirement loudly for consumers (it is the one resume
correctness obligation cogito places on the caller). Build nothing.

### 2. Trigger condition: when this becomes necessary

Build self-describing resume only when a consumer arrives that **cannot**
re-derive the surface from identity it already holds. Concretely, at
least one of:

- A surface is assembled from **ephemeral, mid-session, caller-side state
  that the gateway does not durably own** — e.g. an end user, mid-session,
  uploads a one-off MCP endpoint or activates an ad-hoc skill, and the
  consumer has no place to persist that attachment keyed by session. Then
  on resume the gateway genuinely cannot reconstruct the surface, and the
  only record of "what was attached" would be one cogito persisted.
- The deployment wants **cogito-driven failover** where an arbitrary
  replica resumes a session with **no involvement from the originating
  caller** (e.g. a sweeper process draining abandoned sessions), so there
  is no gateway request in flight to supply the spec.

Absent one of these, re-supply is strictly simpler and avoids the dual
source of truth. The trigger is a real consumer requirement, not a
hypothetical — do not pre-build.

### 3. Design, if the trigger fires: descriptor + resolver

The shape below is the recommended design *when* it is built. It is
recorded now so the deferral is concrete, not vague.

A `SurfaceDescriptor` is a serializable **recipe** for the active provider
surface — never serialized `Arc`s, only stable identity strings the
consumer's resolver understands:

```rust
// cogito-core::runtime (illustrative — not shipping in v0.4)
#[derive(Clone, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
    /// Opaque tool-surface identities (e.g. mcp server names, plugin ids).
    pub tools: Vec<String>,
    /// Opaque skill-surface identities (skill plugin ids).
    pub skills: Vec<String>,
    /// Strategy name (already available via SessionMeta.strategy).
    pub strategy: Option<String>,
    // workspace identity intentionally omitted — see Open questions.
}
```

Persistence and resolution:

- The descriptor is recorded into `SessionMeta.extra`
  (`session.rs:58`, the opaque-pass-through map) at open, and re-stamped on
  each `update_session` swap via a lightweight additive event, so the
  log's latest descriptor reflects the current surface. ADR-0028 §5
  already floated stamping identity into `SessionMeta.extra` "for
  observability"; this promotes that from diagnostics-only to
  load-bearing. No `SCHEMA_VERSION` bump (additive, ADR-0007 precedent).
- The caller supplies a resolver, not the trait objects:

  ```rust
  pub trait SurfaceResolver: Send + Sync {
      fn resolve(&self, d: &SurfaceDescriptor)
          -> Result<SessionSpec, ResolveError>;
  }
  ```

  On `open_session_with(id, Resume, ...)` with no explicit provider
  overrides, the Runtime reads the latest persisted `SurfaceDescriptor`
  from the store and calls the resolver to materialize a `SessionSpec`.
  The resolver is where tenant context and code live — cogito holds only
  the identity strings, never the surface itself. The Brain (H01–H11)
  stays untouched: it still reads providers from the per-turn `TurnDeps`,
  exactly as ADR-0028 established.

This keeps the inviolable rules intact: descriptors are conversation-
adjacent *metadata* (rebuildable-from-log), providers remain injected code
(resolved by the caller), and the layer map (ADR-0004) is unchanged
because the resolver is a Runtime-layer seam the consumer implements.

### 4. Amendment to ADR-0028 §5

ADR-0028 §5 stands as the v0.2 and v0.4 contract: the caller re-supplies
the spec on resume; the core does not reconstruct provider identity. This
ADR supersedes only the parenthetical "deferred to v0.4" framing in
ADR-0028 §"Given up" — the deferral resolves to "deliberately not built;
re-supply is the answer," gated behind the §2 trigger.

## Consequences

What becomes easier:

- The resume contract is now explicit and recommended, not an open TODO:
  re-supply the spec; cogito persists tenant identity to help.
- The descriptor+resolver design is on the shelf, so if the trigger fires
  the implementation path is known (additive `SessionMeta.extra` +
  `SurfaceResolver` seam, Brain untouched).

What becomes harder:

- Nothing changes in v0.2/v0.4 behavior. The cost is documentation
  discipline: consumers must understand the re-supply obligation.

What we give up (deliberately):

- Caller-agnostic, replica-driven resume in v0.4. We accept that a
  resuming caller must know the tenant's surface (it does, via its own
  gateway) and pass it in.

## Alternatives considered

- **Persist serialized providers.** Rejected outright — providers are code
  (MCP transports, composite tool trees); they cannot be serialized, and
  this would violate the protocol/layer boundary. Only identity strings
  are ever persistable.
- **Make cogito the source of truth for the tenant surface.** Rejected —
  the consumer's gateway already owns `tenant -> surface`; duplicating it
  in cogito creates a reconciliation problem with no owner.
- **Build the descriptor now, behind a feature flag.** Rejected for v0.4 —
  no consumer needs it (§1), and a persisted-but-unconsumed descriptor
  risks drifting out of sync with the real surface, becoming a misleading
  half-truth in the log.

## Open questions

- Workspace identity in the descriptor: an ephemeral per-session workspace
  (ADR-0031) has no stable cross-replica identity; whether a resolver could
  ever rebuild it, or whether workspace must always be re-supplied even
  under self-describing resume, is unresolved. Listed as omitted in §3.
- Descriptor freshness vs. `update_session`: the design re-stamps on every
  swap, but the precise event shape (new `EventPayload` variant vs.
  overwriting `SessionMeta.extra`) is undecided and should be settled only
  when the trigger fires.
- Whether the trigger condition (§2) should itself be encoded as a
  consumer-facing capability flag, or just remain a design-time gate.

## References

- ADR-0028 (per-session provider injection) — §5 is the contract this
  amends; `session_spec_mutated_then_resume` is the proof re-supply works
- ADR-0007 (event log as cross-language contract) — additive-event /
  no-`SCHEMA_VERSION`-bump precedent for stamping descriptors
- ADR-0004 (Brain/Hands/Session boundaries) — the layer map the resolver
  seam must respect (caller-implemented, Runtime-layer)
- ADR-0014 (TenantContext propagation, v0.4) — the tenant identity already
  flowing into `SessionMeta` that a resolver would key on
- ADR-0034 (runtime session-registry lifecycle) — same v0.4 SaaS-ready
  theme; in-process reopen, complementary to cross-replica resume
- AGENTS.md §"Inviolable design principles" — state-from-log rule and its
  boundary (conversation state vs injected dependencies)

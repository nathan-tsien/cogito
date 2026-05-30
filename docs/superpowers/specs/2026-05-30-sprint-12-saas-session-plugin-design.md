# Sprint 12 — SaaS per-session injection + Plugin (Skills+MCP) — design

> Status: design approved 2026-05-30 (brainstorming). Decisions ratified
> in [ADR-0028](../../adr/0028-per-session-provider-injection.md)
> (per-session provider injection) and
> [ADR-0021](../../adr/0021-plugin-manifest-and-loader.md) (plugin
> manifest + loader). This spec is the implementation mechanism + scope
> + test plan; the ADRs hold the rationale.

## 1. Goal and reframe

The roadmap's Sprint 12 was "Plugin P1 (skills+agents+hooks+mcp+commands,
local-only)". During brainstorming the consumer's real need reshaped it:

- **Defer hooks** — a cogito hook is a pure Rust policy gate (no shell,
  no I/O); the right way to express one as plugin data depends on the
  consumer's product form. Not rushed. (ADR-0021 §2.)
- **Defer agents / commands** — `agents/` will reuse strategies (no new
  format); no slash-command registry exists yet. Reserve the dir names.
- **Lead with SaaS** — the consumer runs a **multi-tenant API server in
  one process** and needs each session to get a different, **mutable**
  tool/skill surface. That is a Runtime change, not a plugin feature.

So Sprint 12 becomes two ordered pieces:

1. **Core: per-session provider injection** (ADR-0028) — the SaaS
   foundation.
2. **Hands: plugin loader for Skills + MCP** (ADR-0021) — the producer
   of per-session providers.

This pulls one slice of v0.4 SaaS-ready forward; see §7 (roadmap
deviation).

## 2. Why the core change is small

Provider trait objects already flow per-turn:

```
Runtime (global Arcs)
  └─ open_inner: clone into per-session SessionDeps / SessionState
       └─ spawn_turn_driver: rebuild TurnDeps PER TURN from session state
            └─ TurnDriver / H05 tool surface / H08 dispatch read deps.tools, state.skills
```

The Brain (H01–H11) never reaches for a global provider; it reads the
per-turn `TurnDeps`. `ExecCtx` carries context, not providers. The only
things pinning providers to "global, fixed at build" are:

- `open_session(id, mode)` taking no per-session params, and
- `open_inner` cloning `self.tools` / `self.skills`.

`open_inner` *already* takes a per-session `strategy` and a
`meta_override` (subagents use them). We generalize that seam.

## 3. Piece 1 — per-session injection (ADR-0028)

### 3.1 Types and API

```rust
// cogito-core::runtime
#[derive(Default)]
pub struct SessionSpec {
    pub tools:     Option<Arc<dyn ToolProvider>>,
    pub skills:    Option<Arc<dyn SkillProvider>>,
    pub strategy:  Option<HarnessStrategy>,
    pub tenant_id: Option<String>,
    pub user_id:   Option<String>,
}

impl Runtime {
    pub async fn open_session_with(self: &Arc<Self>, id: SessionId,
        mode: OpenMode, spec: SessionSpec) -> Result<SessionHandle, RuntimeError>;
}
impl SessionHandle {
    pub async fn update_session(&self, spec: SessionSpec) -> Result<(), RuntimeError>;
}
```

`open_session(id, mode)` becomes `open_session_with(id, mode,
SessionSpec::default())`. Spec field `Some` → use it; `None` → clone the
Runtime default. Composition is the **caller's** job; core swaps whole
Arcs (no incremental merge).

### 3.2 Mutability + timing

Providers move into the mutable `SessionState`. A new
`SessionCommand::UpdateSession(SessionSpec)` swaps the Arcs in the
session loop. Because `TurnDeps` is rebuilt per turn, the swap takes
effect at the **next turn boundary** (never mid-turn — the in-flight
turn's tool surface and model call are already committed). If a turn is
in flight, the update applies before the next turn drains.

### 3.3 Resume

The core does not persist provider identity (a provider is code, not
state). On resume the caller passes the **current** spec via
`open_session_with(id, Resume, spec)` — equal to or different from the
open-time spec; the core assumes neither. `SessionMeta.tenant_id` /
`user_id` are persisted to help the caller rebuild. Optional
diagnostics-only identity capture in `SessionMeta.extra`. Fully
self-describing multi-replica resume → v0.4.

### 3.4 Change sites (4)

| # | File (from trace) | Change |
|---|---|---|
| 1 | `runtime/builder.rs` | `open_session_with`; `open_session` delegates |
| 2 | `runtime/builder.rs::open_inner` | build `SessionDeps`/`SessionState` from spec; stamp `tenant_id`/`user_id` into `SessionMeta` |
| 3 | `runtime/session_loop.rs` (`SessionState`/`SessionDeps`) | move `tools` (and `strategy`) into mutable `SessionState` next to `skills` |
| 4 | `runtime/session_loop.rs` mailbox | `SessionCommand::UpdateSession` branch swapping Arcs |

Brain crates: **no change**.

## 4. Piece 2 — plugin loader, Skills + MCP (ADR-0021)

### 4.1 New crate `cogito-plugin` (Hands)

```
cogito-plugin/src/{lib.rs, manifest/{toml.rs, claude_json.rs}, discovery.rs, overrides.rs}
```

`PluginSet::load(entries, config_dir) -> PluginContributions { skill_roots, mcp_servers }`.
It resolves each enabled plugin's manifest, applies overrides,
namespaces artifacts `<plugin_id>:<name>`, and returns contributions —
it does **not** build providers (the registries own cross-scope merge).

### 4.2 Config

`[[plugins]]` promotes Reserved → Locked. `PluginEntry { path, enabled,
artifact_overrides }` is defined in `cogito-plugin`; `cogito-config`
depends on `cogito-plugin` to aggregate it (verified MCP precedent:
`cogito-config → cogito-mcp` for `McpServerConfig`). Array replaces
wholesale on merge; additive serde change.

### 4.3 Folding contributions into existing registries

- **Skills**: `ScanConfig` gains `plugin_roots: Vec<PluginSkillRoot>`;
  `SkillRegistry::scan` registers them at Plugin scope (precedence
  already defined in ADR-0020; `SkillSource::Plugin { plugin_id }`
  already exists).
- **MCP**: `cfg.mcp_servers ++ contributions.mcp_servers` → one
  `build_mcp_provider` call. Namespaced server names keep ADR-0018
  dedup valid.

### 4.4 Wiring (CLI surface and consumer server)

```rust
let c = cogito_plugin::PluginSet::load(&cfg.plugins, config_dir)?;
let mcp = cogito_mcp::build_mcp_provider(&[cfg.mcp_servers, c.mcp_servers].concat()).await;
let tools = CompositeToolProvider::new(vec![builtin, mcp_provider], Strict)?;
let skills = SkillRegistry::scan(ScanConfig { plugin_roots: c.skill_roots, ..base })?;
runtime.open_session_with(id, mode, SessionSpec {
    tools: Some(Arc::new(tools)), skills: Some(Arc::new(skills)), ..Default::default()
}).await?;
```

The SaaS server runs this per request with the tenant's plugin set; a
mid-session attach recomposes and calls `update_session`.

## 5. Scope ledger

**In v0.2 (this sprint):**
- `SessionSpec` + `open_session_with` + `update_session` (tools, skills,
  strategy; tenant/user stamping).
- `cogito-plugin` crate: manifest (TOML + claude-json metadata
  fallback), local-path discovery, namespacing, enable/disable +
  per-artifact override, plugin-id uniqueness (fatal).
- Skills + MCP folding; CLI wiring.
- Tests (§6); docs reconcile (§7).

**Deferred (reserved, not loaded):** hooks (`hooks/hooks.toml`),
subagent roles (`agents/`), slash commands (`commands/`); git/HTTP
distribution; self-describing multi-replica resume.

## 6. Test plan

- **Unit**: manifest parse (toml + json-metadata fallback, both-present
  precedence); override application; duplicate-id fatal; namespacing
  (`<id>:<name>`, MCP `:`→`_` sanitize).
- **Integration (acceptance)**: one local plugin with `skills/` (1) +
  `mcp.toml` (1) → `open_session_with` with a spec built from
  `PluginSet::load` → assert plugin skill activates via
  `$<id>:<name>` and plugin MCP tool appears as
  `mcp__<id>_<server>__<tool>`. Then `update_session` adds a second
  plugin MCP server → assert its tool is visible on the **next** turn.
- **Resume-chaos** (new scenario `session_spec_mutated_then_resume`):
  open spec A → turn → `update_session` to spec B (adds MCP) → turn
  using new tool → crash at each boundary → resume with
  `open_session_with(id, Resume, B)` → all four oracles hold.
- **Contract**: existing `open_session(id, mode)` callers unchanged
  (delegation to all-`None` spec) — no regression in current suites.

## 7. Roadmap deviation + doc reconcile

This intentionally pulls a v0.4 SaaS-ready capability (per-session
provider injection) forward, on explicit consumer direction (CLAUDE.md
"only current sprint unless directed otherwise" — directed). Doc work:

- **ROADMAP.md**: Sprint 12 line → "Plugin P1 (Skills + MCP, local) +
  per-session provider injection (ADR-0028)"; acceptance test → "1
  skill + 1 MCP"; note hooks/agents/commands deferred; add a deviation
  note that ADR-0028 advances a v0.4 item.
- **docs/configuration/overview.md**: `[[plugins]]` Reserved → Locked
  (§3 taxonomy, §8 version table) with owner `cogito-plugin`.
- **ADR-0028** (new) and **ADR-0021** (finalized) — done.

## 8. Open implementation questions (for the plan, not blockers)

- Exact `SessionCommand::UpdateSession` ack semantics (fire-and-forget
  vs await applied-at-next-turn).
- Whether `strategy` mutation rides `update_session` in v0.2 or is
  open-time-only (lean: allow it; it's the same rail).
- `ScanConfig` shape: add `plugin_roots` field vs a new `scan_with`
  entry (lean: add field, default empty — backward compatible).

# ADR-0021: Plugin manifest + loader (`cogito-plugin`)

## Status

Accepted (v0.2 Sprint 12, 2026-05-30). Supersedes the earlier
placeholder.

Finalizes the v0.2 tier of the
[2026-05-22 roadmap rebalance](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
(§2.4 P1 + §2.7 + §3.2). Full design + implementation mechanism:
[Sprint 12 spec](../superpowers/specs/2026-05-30-sprint-12-saas-session-plugin-design.md).

**v0.2 scope was narrowed during Sprint 12 brainstorming to Skills +
MCP servers.** Hooks, subagent roles (`agents/`), and slash commands
(`commands/`) are deferred (see §2 and §8); the placeholder's
`agents/*.yaml` shape was stale and is corrected here.

## Context

The rebalance makes v0.2 the **Extensibility** theme: pack a team's
domain capabilities into a single shippable directory rather than
scattered `cogito.toml` edits.

Three precedents surveyed:

- **Claude Code** — `.claude-plugin/plugin.json`; bundles slash
  commands, subagents, skills, hooks, MCP servers; marketplace
  distribution.
- **Codex** — reuses Claude Code's `plugin.json` schema directly;
  signals industry convergence on the manifest.
- **Manus** — no bundle model beyond MCP connectors + skill upload.

cogito should **not invent a new schema**; reading Claude Code-format
plugins lets the same plugin work across runtimes.

Two scoping forces shaped the final v0.2 decision:

1. **Hooks don't fit as data yet.** A cogito `HookHandler` is a pure,
   synchronous, no-I/O policy gate (ADR-0004 §6, H09). Claude Code's
   hooks are shell commands — incompatible with that contract.
   Expressing a hook as data (a declarative match-DSL) or as a
   host-registered Rust factory is a real design fork whose right
   answer depends on the consumer's product form (notably SaaS). It is
   therefore deferred rather than rushed.
2. **SaaS needs per-session, mutable surfaces.** The motivating
   consumer runs a multi-tenant API server and needs each session's
   tools/skills chosen per request and changeable mid-session. That is
   a Runtime/session change, captured separately in **ADR-0028**; this
   ADR's loader is the producer of the providers ADR-0028 injects.

Distribution is tiered (rebalance §2.4 P4): **v0.2 = local path only**
(this ADR); v0.3 = git fetch + lock (ADR-0022); v0.6+ = marketplace.

## Decision

### 1. Manifest: TOML primary + Claude-plugin JSON metadata fallback

Primary `.cogito-plugin/plugin.toml`:

```toml
[plugin]
id          = "code-review"          # required; the namespace; [a-z0-9-]+
version     = "0.1.0"                # optional
description = "Rust + SQL review skills"  # optional

# Artifact paths (optional; defaults shown):
# skills_dir   = "skills"
# mcp_file     = "mcp.toml"
# agents_dir   = "agents"     # reserved, not loaded in v0.2
# hooks_file   = "hooks/hooks.toml"  # reserved, not loaded in v0.2
# commands_dir = "commands"   # reserved, not loaded in v0.2
```

If `.cogito-plugin/plugin.toml` is absent, the loader falls back to
`.claude-plugin/plugin.json` and reads **metadata only** (id / version
/ description). Artifact discovery still uses cogito's default
directory layout. When both files exist, TOML wins. (Full directory-
layout mapping of the Claude-plugin format is out of scope for v0.2.)

### 2. Bundled artifacts — v0.2 scope: Skills + MCP only

| Artifact | Default path | v0.2 | Bundled into |
|---|---|---|---|
| Skills | `skills/<name>/SKILL.md` | ✅ loaded | `SkillRegistry` (Plugin scope) |
| MCP servers | `mcp.toml` | ✅ loaded | `build_mcp_provider` input |
| Subagent roles | `agents/<role>.md` | ⏸ deferred | (strategy registry, later sprint) |
| Hooks | `hooks/hooks.toml` | ⏸ deferred | (see ADR-0004 §6 / H09 constraint) |
| Slash commands | `commands/<name>.md` | ⏸ deferred | (no command registry exists yet) |

Deferred artifact directories are **reserved**: their presence in a
plugin is not an error; the loader ignores them and may warn. When a
later sprint lifts them, plugins already written keep working.

`agents/` is corrected to **markdown + frontmatter** (the shipped
strategy format, ADR-0026), not the placeholder's `.yaml`; a plugin
subagent role will be a Plugin-scoped strategy resolved by the
`delegate` tool's `role` argument.

### 3. Namespace: `<plugin_id>:<artifact_name>` for everything

- **Skills** register as `<plugin_id>:<name>` via the existing
  `SkillSource::Plugin { plugin_id }` variant. The sigil regex already
  admits `:` (ADR-0020 closure note: `\$([A-Za-z][A-Za-z0-9_:-]{0,63})`),
  so `$code-review:review-rust` activates with no change. Bare-name
  repo/user skills never collide with namespaced plugin skills.
- **MCP servers** are renamed `<plugin_id>:<server>`. Downstream
  qualified tool naming `mcp__<server>__<tool>` sanitizes `:`→`_`
  (ADR-0018), e.g. `mcp__code-review_github__list_prs`. Because
  `plugin_id` is globally unique (§5), the namespaced server name is
  unique too, so ADR-0018's existing dedup-by-name suffices.

### 4. Per-plugin enable/disable + per-artifact override

```toml
# cogito.toml
[[plugins]]
path = "./plugins/code-review"      # abs, or relative to cogito.toml

[[plugins]]
path = "./plugins/sql-tools"
enabled = false                     # whole-plugin off, kept declared

[[plugins.artifact_overrides]]      # fine-grained
plugin = "code-review"
kind   = "skill"                    # skill | mcp  (v0.2)
name   = "sql-explain"
enabled = false
```

Overrides are applied during load; an overridden-off artifact never
enters `PluginContributions` (§7).

### 5. Plugin id uniqueness — fatal

`plugin.id` must be globally unique across all `[[plugins]]`. A
duplicate is a **fatal** startup error naming both paths. (Stricter
than MCP's warn-and-skip because plugins are explicitly declared by the
operator, not auto-discovered.)

### 6. v0.2 distribution: local path only

`path` accepts absolute paths and paths relative to the `cogito.toml`
that declared them. No git, no HTTP, no marketplace. A missing path or
malformed manifest is **fatal** (explicit operator config → fail
loud). Git fetch lands in v0.3 (ADR-0022).

### 7. Loader shape: produce contributions, don't own cross-scope merge

`cogito-plugin` does **not** build standalone providers. Skills and
strategies have cross-scope precedence (Repo > User > Plugin > System,
ADR-0020) owned by the existing registries; a separate plugin-built
provider would fork that logic. Instead the loader resolves manifests
into contributions that the caller folds into the existing registries:

```rust
// cogito-plugin
pub fn PluginSet::load(
    entries: &[PluginEntry], config_dir: &Path,
) -> Result<PluginContributions, PluginError>;

pub struct PluginContributions {
    pub skill_roots:  Vec<PluginSkillRoot>, // (plugin_id, abs_dir) → SkillRegistry Plugin scope
    pub mcp_servers:  Vec<McpServerConfig>, // namespaced → concatenated before build_mcp_provider
}
```

Type ownership follows the MCP precedent (verified:
`cogito-config` depends on `cogito-mcp` for `McpServerConfig`):
**`PluginEntry` is defined in `cogito-plugin`; `cogito-config`
depends on `cogito-plugin` to aggregate the `[[plugins]]` section.**
The section promotes from "Reserved" to "Locked" in the config
taxonomy — an additive serde change (no `SCHEMA_VERSION` impact;
the top level already omits `deny_unknown_fields`).

### 8. Composition + wiring (consumes ADR-0028)

The surface (CLI/TUI) or the consumer's API server, per session:

1. `let c = PluginSet::load(&cfg.plugins, config_dir)?;`
2. Tools: `CompositeToolProvider::new([builtin, build_mcp_provider(cfg.mcp_servers ++ c.mcp_servers)])`
3. Skills: `SkillRegistry::scan(ScanConfig { plugin_roots: c.skill_roots, ..base })`
4. Inject via **ADR-0028** `open_session_with(id, mode, SessionSpec { tools, skills, .. })`.

In a SaaS server this runs per request with the tenant's plugin set; a
mid-session `/add-plugin`-style command recomposes and calls
`update_session` (ADR-0028 §3).

### 9. Crate layout

New crate `cogito-plugin` (Hands; pre-approved in ROADMAP Sprint 12):

```
cogito-plugin/
  src/
    lib.rs              # PluginSet::load, PluginEntry, PluginContributions
    manifest/
      toml.rs           # .cogito-plugin/plugin.toml
      claude_json.rs    # .claude-plugin/plugin.json metadata fallback
    discovery.rs        # path resolution, artifact scan, namespacing
    overrides.rs        # enable/disable + per-artifact override
```

Depends on `cogito-protocol`, `cogito-mcp` (`McpServerConfig`), and the
skill-root type from `cogito-skills`. Does **not** depend on
`cogito-core` (Brain/Runtime).

## Consequences

**Easier**:
- A capability ships as one directory; same plugin can target cogito
  and Claude Code (read both manifests).
- Cross-plugin collisions impossible by construction (`<id>:` prefix).
- Cleanly feeds ADR-0028's per-session, mutable surface for SaaS.

**Harder**:
- `SkillRegistry::scan` and `ScanConfig` gain an explicit plugin-roots
  input (plugin dirs are declared, not convention-discovered).
- Two crate edges added: `cogito-config → cogito-plugin`,
  `cogito-plugin → cogito-skills`/`cogito-mcp`.

**Given up (this version)**:
- Hooks / subagent-role / slash-command bundling (deferred; dirs
  reserved).
- Git / HTTP / marketplace distribution (ADR-0022 / v0.6+).
- Signing / verification (no current threat model).

## Integration test (acceptance)

One local plugin directory with `skills/` (1 skill) + `mcp.toml` (1
server) → opened via `open_session_with` with a `SessionSpec` built
from `PluginSet::load` → assert: the plugin skill activates via
`$<plugin_id>:<name>`, and the plugin's MCP tool appears in the turn's
tool surface under `mcp__<plugin_id>_<server>__<tool>`. Plus a
mid-session `update_session` adding a second plugin MCP server,
asserting the new tool is visible on the next turn.

## References

- Sprint 12 spec:
  [`docs/superpowers/specs/2026-05-30-sprint-12-saas-session-plugin-design.md`](../superpowers/specs/2026-05-30-sprint-12-saas-session-plugin-design.md)
- ADR-0028 (per-session provider injection) — how plugins reach a session
- ADR-0017 (config model) — `[[plugins]]` section, layered merge
- ADR-0018 (MCP integration) — naming, dedup, `McpServerConfig` ownership precedent
- ADR-0020 (skill loader) — Plugin scope, `SkillSource::Plugin`, sigil regex
- ADR-0022 (plugin distribution) — v0.3 git fetch this builds toward
- Claude Code plugins: https://docs.claude.com/en/docs/claude-code/plugins

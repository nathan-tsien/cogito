# ADR-0021: Plugin manifest + loader (`cogito-plugin`)

## Status

Proposed — placeholder (finalized in v0.2 Sprint 12).

Captures decisions ratified in the
[2026-05-22 roadmap rebalance spec](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
(§2.4 P1 + §2.7 + §3.2 Sprint 12). The full ADR — manifest schema,
loader algorithm, conflict resolution rules — is finalized during
Sprint 12.

## Context

The rebalance turns v0.2 into the **Extensibility** theme: pack
Skills + Subagents + Hooks + MCP servers into a single shippable
**Plugin** so team members can ship domain capabilities as one unit,
not as scattered config edits.

Three precedents surveyed:

- **Claude Code** — fully formed plugin model:
  `.claude-plugin/plugin.json` manifest; bundles slash commands,
  subagents, skills, hooks, MCP servers; distributed via marketplaces
  (Git / local / HTTP); installed with `/plugin install`.
- **Codex** — reuses `.claude-plugin/plugin.json` schema directly (see
  `core-skills/src/plugin_namespace.rs`); plugins discovered from
  `~/.codex/plugins/` etc.; no separate marketplace concept.
- **Manus** — no public plugin bundle model beyond MCP connector
  attachment + Skill Library upload.

Claude Code is the strongest precedent. Codex's reuse of Claude Code's
manifest signals industry convergence on the schema. cogito should
**not invent a new schema** — being readable from Claude Code-format
plugins lets the same plugin work across runtimes.

Distribution scope is tiered (rebalance spec §2.4 P4):
- **v0.2 (this ADR)** — local path only (`[[plugins]] path = "..."`)
- **v0.3** — git fetch + lock file (see ADR-0022)
- **v0.6+** — marketplace, signing (see future ADR-0023+)

## Decision

### 1. Manifest format: TOML primary + Claude-plugin JSON compatible

Primary: `.cogito-plugin/plugin.toml` (TOML, matching cogito's
config-file convention from ADR-0017).

```toml
[plugin]
id = "code-review"
version = "0.1.0"
description = "Rust + SQL code review skills"
authors = ["nathan@example.com"]

# Default artifact paths (all optional, default values shown):
# skills_dir   = "skills"
# agents_dir   = "agents"
# hooks_file   = "hooks/hooks.toml"
# mcp_file     = "mcp.toml"
# commands_dir = "commands"
```

Compatible read of `.claude-plugin/plugin.json` (JSON, Claude Code
format). When both files are present in the same plugin directory,
TOML wins. When only JSON is present, loader translates to internal
manifest model.

### 2. Bundled artifacts (v0.2 scope)

| Artifact | Path (default) | Bundled into |
|---|---|---|
| Skills | `skills/<name>/SKILL.md` | `SkillProvider` |
| Subagents | `agents/<role>.yaml` | strategy registry (v0.2 minimal Subagent reads role YAML) |
| Hooks | `hooks/hooks.toml` | `HookProvider` |
| MCP servers | `mcp.toml` (subset of cogito.toml `[[mcp_servers]]`) | `McpToolProvider` |
| Slash commands | `commands/<name>.md` | CLI / TUI command registry |

Each plugin contributes one `SkillProvider` / one `HookProvider` / one
`McpToolProvider` (etc.), composed by the runtime via existing
provider-aggregation patterns (`CompositeToolProvider` etc.).

### 3. Namespace: `<plugin_id>:<artifact_name>` for all bundled artifacts

Skill `review-rust` from plugin `code-review` is registered as
`code-review:review-rust`. Same prefix rule applies to:

- subagent roles (`code-review:critic`)
- hook ids (`code-review:bash-audit`)
- MCP servers (`code-review:github`)
- slash commands (`code-review:review`)

This eliminates cross-plugin name collisions structurally. Bare-name
skills from Repo/User scope never collide with plugin-namespaced ones
(see ADR-0020 §2).

### 4. Per-project enable/disable + per-artifact override

```toml
# cogito.toml
[[plugins]]
path = "./plugins/code-review"

# disable an entire plugin without removing it from cogito.toml:
[[plugins]]
path = "./plugins/sql-tools"
enabled = false

# fine-grained per-artifact override:
[[plugins.artifact_overrides]]
plugin = "code-review"
artifact_kind = "skill"
artifact_name = "sql-explain"
enabled = false
```

Plugin loader applies overrides after loading the plugin manifest; an
artifact disabled via override does not register with its provider.

### 5. Plugin id uniqueness

`plugin.id` must be globally unique across all `[[plugins]]` entries.
Loader fails at runtime startup if two plugins declare the same id,
with an error pointing to both plugin paths. (Same shape as MCP server
name uniqueness in ADR-0018.)

### 6. v0.2 distribution scope: local path only

`[[plugins]] path = "..."` accepts absolute paths and paths relative
to the `cogito.toml` file. **No git, no HTTP, no marketplace in v0.2.**

Rationale: Plugin manifest schema needs real usage feedback before any
distribution layer is built. P2 (git fetch) lands in v0.3 under
ADR-0022; P3 (marketplace) is v0.6+ spike.

### 7. Crate layout

New crate `cogito-plugin` in the Hands layer:

```
cogito-plugin/
  src/
    lib.rs            # PluginLoader, PluginConfig
    manifest/
      toml.rs         # .cogito-plugin/plugin.toml parser
      claude_json.rs  # .claude-plugin/plugin.json compat reader
    discovery.rs      # path resolution, scan
    composition.rs    # provider aggregation
```

## Open questions (for Sprint 12)

- Plugin load ordering: alphabetical by id, declaration order in
  `cogito.toml`, or explicit `priority` field?
- Hot-reload: do plugin changes during a session trigger reload, or
  only at session start? (Recommendation: session-start only for v0.2;
  Claude Code's mid-session reload pattern is a v0.3+ candidate.)
- MCP server config inheritance: does a plugin's `mcp.toml` get the
  same `bearer_token_env_var` interpolation as the top-level
  `[[mcp_servers]]`? (Likely yes; verify with ADR-0017 author.)

## Consequences

**Easier**:
- Team members ship a self-contained capability bundle as one directory
- Same plugin can target both cogito and Claude Code (read both
  manifests)
- Per-project enable/disable + per-artifact override give product teams
  fine-grained control without per-deployment forks

**Harder**:
- Plugin lifecycle interacts with Hook / Skill / MCP / Subagent — four
  separate provider patterns must compose cleanly; Sprint 12 validates
  this through integration tests
- Conflicting MCP server ports / Subagent roles within one project's
  plugin set need explicit reporting

**Given up**:
- Git / HTTP fetch (deferred to ADR-0022 / v0.3)
- Signing / verification (deferred indefinitely; no current threat model)
- Marketplace UX (`/plugin install`, `/plugin search`) — v0.6+

## References

- Rebalance spec: [`docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md`](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md) §2.4 + §2.7 + §3.2 + §5.2
- ADR-0017 (config model) — `cogito.toml` layered merge baseline
- ADR-0018 (MCP integration) — naming + uniqueness pattern reused
- Claude Code plugins: https://docs.claude.com/en/docs/claude-code/plugins
- Codex plugin source: `codex-rs/utils/plugins/`

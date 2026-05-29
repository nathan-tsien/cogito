# Architecture Decision Records

We use Michael Nygard's ADR format. Each ADR is a short markdown file
recording an architectural decision and its context.

## Index

- [0001](./0001-rust-workspace-layout.md) — Rust workspace layout
- [0002](./0002-event-sourcing-conversation.md) — Event-sourced conversation log
- [0003](./0003-state-machine-turn-driver.md) — Turn Driver as explicit state machine
- [0004](./0004-brain-hands-session-boundaries.md) — Brain / Hands / Session crate boundaries
- [0005](./0005-production-scope-and-quality-gates.md) — Production scope, quality gates, SLO posture, compatibility commitments
- [0006](./0006-runtime-h01-execution-model.md) — Runtime + H01 Turn Driver execution model (amended 2026-05-19 for ContextManaged state)
- [0007](./0007-event-log-as-cross-language-contract.md) — Event log as cross-language storage contract
- 0008 — Context Management (`H11 Context Manage` mechanism: Compactor / HistoryProjector / SystemPromptInjector trait freeze + first Compactor impl; **promoted from spike to v0.1 Sprint 6** by 2026-05-22 roadmap rebalance — pending implementation)
- 0009 — StorageSystem trait + URI scheme + `ContentBlock::Image` — **moved from v0.2 to v0.5** by 2026-05-22 rebalance (URL-as-text via tools covers v0.1+; first-class multimodal lands with full multimedia tool catalog)
- 0010 — Multimedia tool conventions — **moved from v0.2 to v0.5** by 2026-05-22 rebalance
- [0011](./0011-subagent-execution-model.md) — Subagent execution model — **split** by 2026-05-22 rebalance: **v0.2 Sprint 11 minimal Accepted (2026-05-30)** — sync `delegate(role, input) → output` tool in `cogito-core::runtime::subagent`, `BrainSpawner` run-to-completion trait, child = independent top-level session, parent↔child link as typed `SessionMeta` fields (child-side only), additive `ExecCtx` / `StreamEvent` touches, no `SCHEMA_VERSION` bump; v0.3 amendment adds the full 4-tool lifecycle + parent-child event tree + crash semantics
- 0012–0014 — Reserved for v0.4 ADRs (Sandbox lifecycle, Credential isolation, TenantContext) — unchanged
- 0015 — Reserved for v0.6 Storage HTTP wire protocol — renumbered from 0013
- [0016](./0016-turn-trigger-abstraction.md) — Turn-trigger abstraction (`TurnTrigger` enum, `SessionHandle::submit`, additive event-log evolution path)
- [0017](./0017-cogito-runtime-configuration-model.md) — Cogito Runtime configuration model (`cogito-config` crate, `RuntimeConfig` + `ConfigLoader` trait, `cogito.toml` + `strategies/*.yaml` hybrid layout, layered partial merge, named provider instances)
- [0018](./0018-mcp-integration.md) — MCP integration (`cogito-mcp` crate, `rmcp` 1.5 client wrapper, stdio + streamable-HTTP transports, `mcp__server__tool` namespacing, **MCP failures non-fatal to Runtime** principle + `McpStartupFailure` channel + mandatory startup banner)
- [0019](./0019-reasoning-content-modeling.md) — Reasoning content modeling and event scope (`ContentBlock::Thinking { text, provider_opaque }` inline variant + additive `EventPayload::ThinkingBlockRecorded` event under ADR-0007 precedent — no `SCHEMA_VERSION` bump; covers Anthropic signature / OpenAI Responses encrypted_content / OpenAI-compat `<think>` tag + `reasoning_content` regimes; persisted JSONL is append-only, no backfill of old `<think>`-in-text sessions)
- [0020](./0020-skill-loader.md) — Skill loader (`cogito-skills`, K5 sigil activation `$SkillName` + slash `/skill X` dual channel, agentskills.io-compatible SKILL.md frontmatter, scope precedence Repo > User > Plugin > System, bundled scripts deferred to ADR-0023) — **placeholder, finalized in v0.1 Sprint 7**
- [0021](./0021-plugin-manifest-and-loader.md) — Plugin manifest + loader (`cogito-plugin`, `.cogito-plugin/plugin.toml` primary + `.claude-plugin/plugin.json` compat read, bundles Skills + Subagents + Hooks + MCP + slash commands, `<plugin_id>:<artifact_name>` namespace for all artifacts, v0.2 scope = local path only) — **placeholder, finalized in v0.2 Sprint 12**
- [0022](./0022-plugin-distribution.md) — Plugin distribution (git URL + commit/tag pin, `cogito.lock` file, `cogito plugin sync` command, network-failure-non-fatal-if-cached semantics, marketplace/signing out-of-scope) — **placeholder, finalized in v0.3**
- [0023](./0023-bundled-script-execution.md) — Bundled-script execution in Skills (**deliberately deferred**; records the deferral and design space — Codex-style implicit invocation, Claude-Code-style `` !`cmd` `` substitution, or auto-tool-registration; revisit when concrete use case surfaces or Subagent v0.3 lands)
- [0024](./0024-crate-naming-consolidation.md) — Crate naming consolidation (crate names label layers / roles, not backend implementations; `cogito-store-jsonl` → `cogito-store --features jsonl|postgres|...`; same pattern adopted by `cogito-context`; Accepted ADRs containing old names not modified — historical-name map maintained here) — **Accepted (2026-05-29)**

## Template

```markdown
# ADR-XXXX: Title

## Status
Proposed | Accepted | Deprecated | Superseded by ADR-YYYY

## Context
What is the issue? What forces are at play?

## Decision
What did we decide?

## Consequences
What becomes easier? What becomes harder? What did we give up?
```

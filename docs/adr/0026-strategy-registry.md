# ADR-0026: Strategy registry (`cogito-strategy`) — declarative agent modes

**Status**: Accepted (ratified at Sprint 9a close, 2026-05-28)
**Date**: 2026-05-27
**Spec**: `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
**Supersedes (partially)**: ADR-0017 §13 (single `strategies_dir` model is now
overridable but no longer the only path)

## Context

A strategy is a named, declarative "agent mode" — *which model,
which persona, which tools, which context policy* bundled into one
artifact selectable by `--strategy <name>`. Strategies are not
`cogito.toml` configuration; they are a separate Hands sub-layer
artifact owned by the agent designer.

`HarnessStrategy` (the per-turn behavior bundle: system prompt, tool
filter, model params, context policy) has shipped as a Rust value type
since Sprint 2. Until now the only way to construct one was the
`HarnessStrategy::default_with_model(model_id)` factory plus
ad-hoc field mutation in CLI code. Agent designers cannot ship a
behavior change without a code change.

The original Sprint 4.5 model (ADR-0017 §13) folded a
`HashMap<String, HarnessStrategy>` directly into `RuntimeConfig`,
populated by walking a single `runtime.strategies_dir` directory.
That design has three problems:

1. **It hard-codes one filesystem source.** v0.4 SaaS deployment will
   read strategies from a database or object store, not from disk.
2. **It mixes audiences.** `RuntimeConfig` is the deployment
   operator's territory (endpoints, credentials). Strategies are the
   agent designer's territory (prompts, tool filters). Bundling them
   muddles ownership and rate of change.
3. **It misses the Skills (Sprint 7) precedent.** Skills already
   established Repo > User scope precedence for declarative artifacts;
   strategies should follow the same convention.

## Decision

Introduce a protocol-layer trait `StrategyRegistry` and a Hands sub-layer
crate `cogito-strategy` that hosts the v0.1 FS-backed implementation.

### Trait

```rust
pub trait StrategyRegistry: Send + Sync + 'static {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError>;
    fn list(&self) -> Vec<String>;
    fn as_any(&self) -> &dyn std::any::Any { &() }
}
```

Read-only, object-safe, Arc-shareable. `as_any` lets concrete impls
expose extra metadata (e.g., the FS impl's `provider_ref(name)`)
without bloating the trait API.

### File format

Markdown with YAML frontmatter. Body of the markdown is the system
prompt. Skills (Sprint 7 / ADR-0020) uses the same shape. Filename
basename must match the `name` frontmatter field.

### Scope precedence

Repo `.cogito/strategies/` > User `~/.config/cogito/strategies/`.
`cogito.toml` `runtime.strategies_dir` overrides the Repo root only.

### What `cogito-strategy` is NOT

It is **not** an extension of `cogito-config`. Two distinct artifacts,
two audiences, two lifecycles:

|  | `cogito.toml` (cogito-config) | Strategy `.md` (cogito-strategy) |
|---|---|---|
| Audience | Deployment operator | Agent designer |
| Contents | Provider endpoints, API keys, defaults | Prompts, tool filters, knobs |
| Secrets | Yes (env-interpolated) | No — commit to repo |
| Cardinality | One file, global | Many files, one per mode |
| Rate of change | Rare (deployment change) | Frequent (iterating behavior) |
| SaaS path (v0.4) | Stays TOML + env | Swaps to DB/S3 behind same trait |
| Layer | Startup/runtime config | Hands sub-layer artifact |

Folding strategies into `cogito-config` would force the
deployment-operator config loader to also own multi-file YAML/markdown
discovery, scope precedence, frontmatter parsing, and an eventual
S3/DB swap. That is boundary stretch.

### Supersession of ADR-0017 §13

ADR-0017's `RuntimeConfig.strategies: HashMap<String, HarnessStrategy>`
field is **removed**. `runtime.strategies_dir` remains as an
optional Repo-root override. Strategy storage moves entirely behind
the `StrategyRegistry` trait.

## Consequences

### Positive

- Agent designers ship behavior changes as `.md` files, no Rust.
- v0.4 SaaS can swap to DB/S3 without touching Brain.
- Skills, plugins (v0.2), and subagents (v0.3) share the same
  artifact-loading mental model.
- The CLI's `resolve_strategy` becomes the single seam where strategy
  + CLI flags + cogito.toml merge — TUI (Sprint 9b) and consumer
  server code call the same helper.

### Negative

- One more crate in the workspace (`cogito-strategy`). Mitigated by:
  the alternative was putting it in `cogito-config` and stretching that
  crate's responsibility.
- `as_any` downcast in `resolve_strategy` is a code smell that protocol
  purists will dislike. Mitigated by: it's a single call site in
  `cogito-cli`; v0.4 DB-backed registries can return `None` for
  `provider_ref` cleanly because operators using a SaaS deployment
  will declare providers exclusively in cogito.toml.

### Neutral

- Existing draft `strategies/*.yaml` files are deleted. They never
  worked (stale schema); no users depended on them.

## Alternatives considered

1. **Inline provider config in strategies.** Rejected: would embed
   secrets in committed files.
2. **Templating with `{{var}}` substitution.** Rejected: adds
   prompt-injection surface; Skills (Sprint 7) already handles dynamic
   per-task content.
3. **Multi-file prompt composition (`files: [base.md, role.md]`).**
   Rejected: same reasoning — Skills is the modular layer.
4. **Folding into `cogito-config`.** Rejected: see "Negative" above.
5. **Hot reload.** Rejected by design — same as Skills, same as
   `cogito.toml`. Registry built once at startup.

## References

- Spec: `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
- Brainstorm: same date
- Precedent: ADR-0020 (Skill loader), ADR-0017 (Runtime configuration)
- Forward-looking: ADR-0021 (Plugin loader) will treat strategies as
  one of the plugin-bundleable artifact types (v0.2 Sprint 12).

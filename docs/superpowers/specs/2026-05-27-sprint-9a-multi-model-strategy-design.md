# Sprint 9a · Multi-model Strategy — design spec

**Status**: Proposed
**Sprint**: v0.1 / Sprint 9a (split off from ROADMAP Sprint 9 by 2026-05-27 brainstorm)
**Budget**: 2 days (revised from "Sprint 9 total 2 days" by 9a/9b split — 9a takes the heavier half: new crate + new adapter + CLI rewire)
**ADR**: [`ADR-0026`](../../adr/0026-strategy-registry.md) (Proposed; ratified at end of this sprint)
**Predecessors**: Sprint 6 (`cogito-context`, `ContextConfig`), Sprint 7 (Skill loader — markdown+frontmatter precedent, scope discovery shape), ADR-0017 (Provider config schema), ADR-0019 (Reasoning content)
**Successors**: Sprint 9b (TUI), Sprint 11 (Subagent — strategies become role bundles), v0.4 SaaS (DB-backed `StrategyRegistry` impl behind the same trait)

---

## 1. Goals

Ship a **strategy registry** so agent designers can declare "how this agent
should think for this kind of task" as a markdown file with frontmatter, no
Rust required. Concretely, after Sprint 9a lands:

1. A team member drops `.cogito/strategies/coder.md` (or
   `~/.config/cogito/strategies/coder.md` for personal-default scope); the next
   `cogito chat --strategy coder` picks it up.
2. The strategy bundles model, persona, allowed tools, sampling knobs, and
   context policy as one named artifact. Switching agents is `--strategy <name>`,
   not a code change.
3. CLI flags layer over the strategy (`--model X --strategy coder` overrides
   `model`; `--system "..."` overrides `system_prompt`) so iteration stays fast.
4. A new OpenAI Responses adapter lands in `cogito-model` so strategies can
   bind to OpenAI's Responses API with native reasoning items (mapped to
   `ContentBlock::Thinking` per ADR-0019).
5. `cogito chat` with zero strategy files still works — a synthesized
   `default` strategy is built from `cogito.toml` + `--model` flag, identical
   to today's behavior.
6. The whole thing sits behind a `StrategyRegistry` trait in
   `cogito-protocol`, so v0.4 SaaS deployment can swap the FS-backed impl for
   a DB/S3-backed one **without touching Brain**.

## 2. Non-goals

- **Templating in prompts** (`{{var}}`-style substitution). Strategies are
  the *base* behavior contract; dynamic per-task augmentation is Skills'
  job (Sprint 7).
- **Multi-file prompt composition** (`system_prompt: { files: [base.md, role.md] }`).
  Same reasoning — Skills owns modular prompt content.
- **Environment-variable interpolation in prompt bodies.** Prompts are not
  secrets. If a value really needs runtime injection, plumb it through
  `ExecCtx` or a skill activation.
- **Hot reload.** Registry is built once at `RuntimeBuilder::build()`. YAML
  edits require a restart, same as Skills and `cogito.toml`.
- **`task` selector axis.** H10's reserved `task: &TaskContext` parameter
  stays reserved. v0.1 keeps the one-strategy-per-name mapping.
- **TUI work.** That's Sprint 9b in a separate spec.
- **Built-in tools on the Responses adapter** (`file_search`, `web_search`,
  `code_interpreter`). Those are Hands-layer concerns and deserve their
  own ADR if/when needed.
- **DB-backed registry impl.** The trait is shaped for v0.4 SaaS, but only
  the FS-backed impl ships in 9a.

## 3. What is a strategy (the consumer-facing framing)

> **A strategy is a named, declarative "agent mode."** It bundles *which model,
> which persona, which tools, which context policy* for one kind of work.
> The consumer ships their cogito-embedded service with N strategies —
> `coder`, `planner`, `reviewer`, `critic` — and `cogito chat --strategy coder`
> (or, programmatically, `runtime.open_session_with_strategy("coder", ...)`)
> selects the mode. Same Brain, same Boundary, different *behavior contract*.
> **Without strategies, every behavior change is a code change and a redeploy.**

> **Strategies are not configuration of cogito.** `cogito.toml` is "where is
> the model and how do I reach it" — endpoints, credentials, provider
> defaults. Strategies are "what do I tell the model to do." The two layer
> cleanly: strategies *reference* providers from `cogito.toml` by name;
> they never embed credentials.

This framing is the headline of the design and the load-bearing reason
`cogito-strategy` is a separate crate from `cogito-config`. The same
two paragraphs (or tightened versions) land in five more places (see §13).

## 4. Strategy vs Config — why two crates

|  | `cogito.toml` (cogito-config) | Strategy `.md` (cogito-strategy) |
|---|---|---|
| **Audience** | Deployment operator | Agent designer |
| **Contents** | Provider endpoints, API keys, `base_url`, defaults | System prompt, allowed_tools, model knobs, context policy |
| **Secrets** | Yes (env-interpolated) | No — commit to repo, share across team |
| **Cardinality** | One file, global | Many files, one per agent mode |
| **Rate of change** | Rare (deployment change) | Frequent (iterating on behavior) |
| **SaaS path (v0.4)** | Stays TOML + env | Swaps to DB/S3 behind same trait |
| **Layer** | Startup/runtime config | Hands sub-layer artifact (peer of Skills, MCP catalog) |
| **Brain access** | Via `RuntimeConfig` at boot only | Via `Arc<dyn StrategyRegistry>` on every session-open |

The crate boundary matches the audience boundary. Folding strategies into
`cogito-config` would force the deployment-operator config loader to also
own multi-file YAML/markdown discovery, scope precedence, frontmatter
parsing, and an eventual S3/DB swap — boundary stretch.

## 4.1 Supersession of ADR-0017 §13 strategy storage

ADR-0017 §13 (Sprint 4.5 scope) reserved `runtime.strategies_dir` as a
single configurable path with default `./strategies/`, and held
strategies inside `RuntimeConfig` as a `HashMap<String, HarnessStrategy>`
populated by walking that one directory. Sprint 9a **supersedes** this
shape:

- `RuntimeConfig` no longer carries an in-line `strategies` HashMap.
  Strategies live behind `Arc<dyn StrategyRegistry>`, built by the
  Runtime layer at startup.
- Scope precedence replaces the single `strategies_dir` key. The
  conventional roots are Repo `.cogito/strategies/` (highest) and
  User `~/.config/cogito/strategies/` (lowest). `runtime.strategies_dir`
  in cogito.toml, if present, **overrides the Repo root only**
  (User root is unaffected); absent = default `.cogito/strategies/`.
  This preserves the ADR-0017 escape hatch for unusual layouts while
  matching the Sprint 7 Skills convention by default.
- ADR-0026 records the supersession explicitly so the conflict is
  obvious to anyone reading ADR-0017 first.

## 5. Locked decisions

The brainstorming session locked the following. ADR-0026 ratifies them.

| Topic | Decision |
|---|---|
| Strategy binding model | **C — bundles behavior + optional provider/model defaults; CLI flags override** (vs. A pure-bundle / B model-agnostic) |
| Discovery layer | New trait `StrategyRegistry` in `cogito-protocol`, read-only (`get` + `list`) |
| FS impl crate | New crate `cogito-strategy` (Hands sub-layer, mirrors `cogito-skills` / `cogito-jobs`) |
| Scope precedence | Repo `.cogito/strategies/` > User `~/.config/cogito/strategies/` (Skills convention) |
| Zero-config behavior | Synthesize `default` strategy from `cogito.toml` + CLI `--model` (preserves today's `cogito chat --model X` flow) |
| File format | **Markdown with YAML frontmatter** (Skills-style); body is `system_prompt` unless frontmatter overrides |
| Frontmatter required | `name` (must match filename basename) |
| Frontmatter optional | `description`, `provider`, `model`, `allowed_tools`, `tool_order`, `max_turns`, `model_params`, `context`, `system_prompt` (override-body form: `system_prompt: { file: ... }`) |
| ContextConfig schema | Reuse `cogito_context::ContextConfig` directly via serde — single source of truth |
| OpenAI Responses adapter scope | Core (messages + tools + streaming) + reasoning effort toggle. No built-in tools (`file_search`/`web_search`/`code_interpreter`). |
| `--model` CLI semantics | Stays as model-id override (today's meaning) |
| New CLI flag | `--strategy <name>` — selects strategy from registry |
| Migration of existing draft YAMLs | Delete `strategies/{claude-opus,gpt-4}.yaml`; ship fresh `.cogito/strategies/{coder,planner,reviewer}.md` examples |

## 6. Architectural overview

```
┌──────────────────────────────────────────────────────────────────────┐
│  RuntimeBuilder                                                      │
│    ├─ scans .cogito/strategies/*.md      ──┐                         │
│    └─ scans ~/.config/cogito/strategies/*.md ┴─► StrategyRegistry    │
│                                                  (FsStrategyRegistry)│
│        │                                                             │
│        ▼ Arc<dyn StrategyRegistry>                                   │
│  ExecCtx { ..., strategies, ... }                                    │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │  cogito chat --strategy coder                                │    │
│  │    1. resolve_strategy("coder", &cli_flags, &cfg, registry): │    │
│  │       a. registry.get("coder")  →  HarnessStrategy           │    │
│  │       b. apply CLI overrides (--model, --system)             │    │
│  │       c. resolve provider ref → ProviderConfig from cogito.toml   │
│  │       d. merge model_params (strategy wins on key conflict)  │    │
│  │    2. build_gateway(provider_cfg)  →  Arc<dyn ModelGateway>  │    │
│  │    3. runtime.open_session(strategy, gateway, ...)           │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                      │
│  H10 Strategy Selector (cogito-core::harness):                       │
│    select(model_id, task, registry) → HarnessStrategy                │
│      (per-turn read; deterministic; cached for duration of turn)     │
└──────────────────────────────────────────────────────────────────────┘
```

H10's role does not expand — it still produces a `HarnessStrategy` value
per turn, deterministic, no I/O. The new machinery (registry, file scan,
frontmatter parser) lives outside Brain in `cogito-strategy`.

## 7. Strategy file format

### 7.1 Layout

```markdown
---
name: coder                          # required, must match filename basename
description: >                       # optional, human-only; surfaced by `--list`
  Coding tasks. Read first, write second. Low temperature for precision.

# Optional provider/model binding. If absent, CLI --model and
# cogito.toml [default_provider] resolve them. CLI --model wins.
provider: anthropic-default          # references cogito.toml [providers.anthropic-default]
model: claude-opus-4-7

# Tool filter. null/omit = ToolFilter::All (every tool the provider lists).
allowed_tools:
  - read_file
  - run_tests

# Optional explicit ordering for prompt-cache stability.
tool_order:
  - read_file
  - run_tests

# Safety budget; default 16 if omitted.
max_turns: 50

# Sampling knobs. Overlay on top of provider-level model_params.
# Strategy keys win on conflict.
model_params:
  temperature: 0.3
  max_tokens: 4096

# Context-management pipeline. Deserializes directly into
# cogito_context::ContextConfig. Default = all-no-op.
context:
  compactor: { kind: truncate, max_tokens: 100000 }
---

You are a precise software engineer.
Always read before writing. Run tests after every change.
...
```

The body of the markdown **is** the `system_prompt`. Frontmatter and body
are both optional in isolation but at least one must produce a non-empty
`system_prompt` (either body has content or frontmatter sets
`system_prompt` explicitly).

### 7.2 Frontmatter override of body

If frontmatter explicitly carries a `system_prompt` field, it wins and the
body is ignored (warn-log if body is non-empty — operator probably made an
edit and forgot to clear one):

```yaml
system_prompt:
  file: ./prompts/coder-long.md     # path relative to the strategy .md
```

Wire-level the field is:

```rust
#[serde(untagged)]
enum SystemPromptSource {
    /// "system_prompt: just a string"
    Inline(String),
    /// "system_prompt: { file: ./path.md }"
    FileRef { file: PathBuf },
}
```

Path resolution: relative to the strategy `.md` file's directory. Absolute
paths are allowed (operator's call) but documented as smell. Registry
loads the referenced file **eagerly** at registry-build time — `Brain`
always receives a fully-materialized `String`. v0.4 SaaS DB impl will not
have a `file:` notion at all; the trait abstracts the materialization.

### 7.3 Schema reuse

`context:` deserializes directly into `cogito_context::ContextConfig`.
`model_params:` deserializes into `cogito_protocol::gateway::ModelParams`
(minus the `model:` field, which is hoisted to the top level). `cogito-strategy`
declares deps on both — both are Hands/Protocol sub-layer, allowed.

### 7.4 Filename ≡ name

`coder.md` MUST declare `name: coder` in frontmatter. Mismatch is fatal at
registry-load. Rationale: makes `grep -r coder .cogito/` find both the
declaration and references; eliminates the "two names, which is canonical"
question.

## 8. `StrategyRegistry` trait

In `cogito-protocol`:

```rust
/// Read-only registry of named `HarnessStrategy` bundles. v0.1 ships an
/// FS-backed impl in `cogito-strategy`; v0.4 SaaS adds a DB-backed impl
/// behind the same trait.
pub trait StrategyRegistry: Send + Sync + 'static {
    /// Returns the named strategy. Both unknown name and load-time
    /// parse error surface as `StrategyError`. The returned value has
    /// `system_prompt` fully materialized (file refs resolved).
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError>;

    /// Returns the names of all strategies currently registered, sorted.
    /// Used by `cogito chat --strategy <TAB>` completion and
    /// `cogito chat --list-strategies` (deferred to 9b if needed).
    fn list(&self) -> Vec<String>;
}

#[derive(thiserror::Error, Debug)]
pub enum StrategyError {
    #[error("strategy `{0}` not found; available: {1:?}")]
    Unknown(String, Vec<String>),
    #[error("strategy `{name}` references missing provider `{provider}`")]
    UnknownProvider { name: String, provider: String },
    #[error("strategy `{name}` validation failed: {reason}")]
    Validation { name: String, reason: String },
}
```

Notes:

- **No `register` / `reload` / mutation.** Read-only by design (per
  brainstorm).
- **`Send + Sync + 'static`** so the registry is shareable as
  `Arc<dyn StrategyRegistry>` across the session loop.
- **Error variants are non-exhaustive at the rustdoc level** (matches our
  Protocol-layer convention) — concrete impls can add internal context
  via `Validation { reason }`.

## 9. `cogito-strategy` crate

### 9.1 Public surface

```rust
//! Filesystem-backed StrategyRegistry implementation.
//!
//! See ADR-0026 for the trait contract and the "strategy ≠ config"
//! framing.

pub use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};
pub use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};

/// FS-backed registry. Built once at startup by scanning the configured
/// scope directories.
pub struct FsStrategyRegistry { /* private */ }

impl FsStrategyRegistry {
    /// Build a registry by scanning the given scope roots in precedence
    /// order (highest first). Each root is a directory of `*.md` files.
    /// Missing roots are silently skipped.
    ///
    /// # Errors
    /// Returns the first parse / validation error encountered. Fatal at
    /// startup — agent designers should learn about broken strategies
    /// immediately, not at session-open three hours later.
    pub fn from_roots(roots: &[ScopeRoot]) -> Result<Self, LoadError>;

    /// Convenience: scan the conventional Cogito scope roots
    /// (Repo `.cogito/strategies/`, User `~/.config/cogito/strategies/`).
    /// Missing roots are skipped.
    pub fn from_conventional_scopes() -> Result<Self, LoadError>;
}

#[derive(Debug, Clone)]
pub enum Scope { Repo, User }

#[derive(Debug, Clone)]
pub struct ScopeRoot { pub scope: Scope, pub path: PathBuf }

impl StrategyRegistry for FsStrategyRegistry { /* ... */ }
```

### 9.2 Module layout

```
crates/cogito-strategy/
  Cargo.toml
  src/
    lib.rs          // public surface above
    scope.rs        // Scope, ScopeRoot, scope_root_paths()
    parser.rs       // frontmatter parser (reuses logic from cogito-skills if cheap)
    schema.rs       // serde structs mirroring frontmatter shape
    registry.rs     // FsStrategyRegistry impl
    error.rs        // LoadError + From<StrategyError>
  tests/
    fixtures/       // sample strategy files for snapshot tests
      coder.md
      planner.md
      malformed_no_name.md
      mismatched_name.md
    parse.rs        // YAML/markdown parsing snapshot tests
    registry.rs     // scope precedence + duplicate-name tests
    contract.rs     // shared contract suite from cogito-test-fixtures
```

### 9.3 Frontmatter parser

Reuses the pattern from `cogito-skills` (Sprint 7) — `---`-delimited YAML
front, markdown body. We extract the parser into a small shared helper
**only if** lifting it from `cogito-skills` requires fewer than ~50 lines;
otherwise duplicate. (The "no premature abstraction" rule from
CLAUDE.md beats the dedupe instinct here.) Decision left to the
implementation PR.

### 9.4 Scope precedence and duplicate handling

- Within a single scope: duplicate `name` is **fatal at startup**
  (`LoadError::DuplicateName { name, files }`).
- Across scopes: Repo wins over User. The User entry is silently shadowed
  (no warn — operators expect their per-user defaults to be overridable
  by repo-local ones, exactly like skills).
- Cross-scope same-name collision is recorded in
  `ScopeRoot::collisions: Vec<(name, winning_path, shadowed_path)>` for
  test inspection but not surfaced as a runtime warning.

### 9.5 Layer compliance

- Imports: `cogito-protocol`, `cogito-context`, `serde`, `serde_yaml`,
  `thiserror`, `walkdir`. No Brain crate. No Surface crate.
- Sub-layer (per ADR-0025): Hands-side artifact loader. Same shape as
  `cogito-skills`. Brain consumes via `Arc<dyn StrategyRegistry>` only.
- Per CLAUDE.md "tagged-config factories belong in the crate that owns
  the implementations" rule, the YAML → `HarnessStrategy` translation
  lives entirely in `cogito-strategy`; surfaces never pattern-match on
  any tagged subfields.

## 10. Default-strategy synthesis (D1)

When no `--strategy` flag is given AND no `default` strategy exists in
the registry, the runtime synthesizes one in-memory:

```rust
let default = HarnessStrategy::default_with_model(model_id_from_cli_or_cfg);
// CLI --system, if present, overrides default.system_prompt.
// CLI --temperature/--max-tokens are NOT added in 9a; out of scope.
```

This preserves today's `cogito chat --model X` zero-config UX exactly.
The synthesis lives in **the wiring layer** (`cogito-cli`'s chat command,
specifically in a new helper `resolve_strategy`), not inside
`cogito-strategy` — keeps the registry crate pure (no implicit defaults).

If the user explicitly defines `.cogito/strategies/default.md`, it wins
over synthesis.

## 11. OpenAI Responses adapter

### 11.1 Scope (locked option b)

- `responses.create` endpoint, streaming SSE.
- Input messages: ContentBlock (`Text`, `ToolUse`, `ToolResult`, `Thinking`)
  serialized to Responses' flat top-level items.
- Output stream: parse Responses SSE event types into cogito `ModelEvent`s
  (`TextDelta`, `ToolCallDelta`/`Completed`, `ThinkingDelta`/`Completed`,
  `StopReason`, `Usage`).
- Tools: function tools only (no `file_search`, `web_search`,
  `code_interpreter` — those are Hands concerns).
- Reasoning: native Responses `reasoning` summary items → `ContentBlock::Thinking`
  per ADR-0019. Reasoning effort toggle exposed in provider config:
  `reasoning_effort: low | medium | high | null` (null = let the provider
  pick).

### 11.2 ProviderConfig addition

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderConfig {
    Anthropic { /* unchanged */ },
    #[serde(rename = "openai-compat")]
    OpenAiCompat { /* unchanged */ },
    #[serde(rename = "openai-responses")]
    OpenAiResponses {
        name: String,
        api_key: String,
        #[serde(default = "defaults::openai_responses_base_url")]
        base_url: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
        /// `low` | `medium` | `high` | omit (= provider default).
        #[serde(default)]
        reasoning_effort: Option<ReasoningEffort>,
    },
}

/// New enum colocated with `ProviderConfig`. Pure pass-through to the
/// Responses API's `reasoning.effort` field; no interpretation cogito-side.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort { Low, Medium, High }
```

### 11.3 Module layout

```
crates/cogito-model/src/
  openai_responses/
    mod.rs           // OpenAiResponsesGateway + OpenAiResponsesConfig
    encode.rs        // ContentBlock → Responses input items
    decode.rs        // Responses SSE → ModelEvent
    wire.rs          // raw SSE types (serde)
    tests.rs         // unit-level encode/decode round trips
  provider_config.rs // ProviderConfig::OpenAiResponses arm + build_gateway dispatch
```

Mirrors the `anthropic/` and `openai_compat/` shape. Streaming tests use
recorded SSE fixtures (no live API hits in CI), same pattern as
`openai_compat` adapter.

### 11.4 Reasoning content (ADR-0019 follow-through)

The Responses adapter is the **third** ContentBlock::Thinking-aware
gateway. Anthropic (Sprint 4.7) decodes `thinking_delta` +
`signature_delta` + `redacted_thinking`. OpenAI-compat decodes
`<think>` SSE markers + `reasoning_content` field. OpenAI Responses
decodes native `reasoning.summary` items. All three encode
`ContentBlock::Thinking` back to wire on follow-up turns.

The Responses path is the **cleanest** of the three — reasoning items
are first-class wire-protocol objects, not tag-parsed or signature-paired.
No new protocol-level changes (additive to `cogito-model` only).

## 12. CLI integration

### 12.1 New flag and resolution table

`cogito chat` gains `--strategy <name>`. The resolution order:

| Field | Lookup order |
|---|---|
| `strategy` | `--strategy` → cogito.toml `default_strategy` (new key) → synthesized `default` |
| `provider` | `--provider` → strategy.provider → cogito.toml `default_provider` → error |
| `model` | `--model` → strategy.model → cogito.toml `runtime.default_model` (per ADR-0017) → error |
| `system_prompt` | `--system` → strategy.system_prompt (resolved from body or file ref) → empty |
| `allowed_tools` | strategy.allowed_tools → `ToolFilter::All` |
| `tool_order` | strategy.tool_order → `None` |
| `max_turns` | strategy.max_turns → 16 (current default) |
| `model_params` | per-key overlay: strategy.model_params over provider.model_params; CLI scalar flags (none added in 9a) would win on top |
| `context` | strategy.context → `ContextConfig::default()` |

### 12.2 cogito.toml addition

```toml
default_strategy = "coder"   # optional; default = synthesized "default"
```

Lives next to existing `default_provider` and `default_model` keys.

### 12.3 Resolution helper

A new helper in `cogito-cli::chat`:

```rust
fn resolve_strategy(
    args: &ChatArgs,
    cfg: &RuntimeConfig,
    registry: &dyn StrategyRegistry,
) -> Result<(HarnessStrategy, ProviderConfig)>;
```

This is the **only** code path that knows how to combine strategy + CLI
flags + cogito.toml + synthesized default. Other surfaces (Sprint 9b TUI,
consumer Server code) call the same helper. Per CLAUDE.md
"tagged-config factories belong in the crate that owns the
implementations", the resolution lives in `cogito-cli` only because that
is the consumer-surface crate — once a second surface (TUI) lands in 9b
we either extract `resolve_strategy` to a small shared crate or duplicate
(decision deferred to 9b based on call-site count).

### 12.4 Listing helper

`cogito chat --list-strategies` prints `registry.list()` + `description`
field. Useful sanity check during onboarding. Implemented in 9a; the
Sprint 9b TUI surfaces the same data interactively.

## 13. Documentation propagation

The strategy-vs-config framing (§3 + §4) is load-bearing for consumers
and lands in four places. **Per the auto-memory note about propagating
locked decisions to durable docs.**

| Location | What it carries |
|---|---|
| `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md` (this spec) | Full framing + table; leads the doc |
| `docs/adr/0026-strategy-registry.md` (new) | Durable rationale; the §4 table; non-goals; "two crates, two audiences" decision |
| `docs/components/H10-strategy-selector.md` | Top-of-file "What is a strategy" section before existing impl notes; supersedes the 2026-05-21 "drops `name:` field" note (we keep `name:` and validate against filename) |
| `crates/cogito-strategy/src/lib.rs` (crate-level `//!` docstring) | First thing a consumer sees in rustdoc; same two-paragraph framing |
| `AGENTS.md` §"Authoritative docs" | One-line cross-reference to ADR-0026 |
| `docs/configuration/overview.md` | One paragraph: "Strategies live in `.cogito/strategies/`, not in `cogito.toml`. See ADR-0026." |

## 14. Error handling

All errors flow through `thiserror`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("I/O error reading {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("parse error in {path}: {source}")]
    Parse { path: PathBuf, #[source] source: serde_yaml::Error },
    #[error("frontmatter missing or malformed in {path}")]
    Frontmatter { path: PathBuf },
    #[error("duplicate strategy name `{name}` in scope: {files:?}")]
    DuplicateName { name: String, files: Vec<PathBuf> },
    #[error("filename / name mismatch: {path} declares `name: {declared}`")]
    NameMismatch { path: PathBuf, declared: String },
    #[error("strategy {name} has empty system_prompt (body and frontmatter both empty)")]
    EmptyPrompt { name: String },
}
```

Behavior:

- **Registry-load is fatal**: any `LoadError` propagates out of
  `RuntimeBuilder::build()`. Agent designers learn about broken
  strategies at startup, not three hours into a session.
- **Cross-reference validation deferred**: dangling provider names
  (strategy `provider: foo` where `cogito.toml` has no `[providers.foo]`)
  surface at session-open as `StrategyError::UnknownProvider`. Detected
  in `resolve_strategy`, which is where both inputs first meet.
- **`cogito chat --strategy foo` where `foo` missing**: print `available
  strategies: [...]` and exit 2 (CLI usage error).
- **`allowed_tools` references nonexistent tools**: warn-log at H05 when
  the tool surface is built (we don't know the surface at registry-load
  time, so can't validate there). Strategy keeps running with the empty
  filtered set; this matches today's `ToolFilter::Allow` semantics
  (silently drop unknowns).
- **`model_params` keys not understood by the provider**: pass through;
  provider wire-level error becomes ground truth.

## 15. Testing

### 15.1 Unit tests (cogito-strategy)

- Frontmatter parsing: snapshot tests with fixtures for minimal,
  maximal, and all degenerate-but-valid inputs.
- Negative parse tests: missing `name`, mismatched name, malformed
  YAML, empty body and empty frontmatter, file-reference target
  missing.
- ContextConfig round-trip: parse a strategy with each compactor
  variant and verify the resulting `cogito_context::ContextConfig`
  matches.

### 15.2 Contract tests

Shared contract suite consumed by every `StrategyRegistry` impl. Lives
in `cogito-test-fixtures` (per existing pattern from
`ConversationStore`). v0.1 has two impls: `FsStrategyRegistry` and a
`MapStrategyRegistry` in test-fixtures for tests that don't want to
touch disk.

Contract assertions:

- `get(name)` returns the same `HarnessStrategy` across repeated calls
  (deterministic).
- `list()` is sorted, deduplicated, total.
- `get` of any name in `list()` succeeds.
- `get` of any name NOT in `list()` returns `StrategyError::Unknown`.

### 15.3 OpenAI Responses adapter unit tests

- Encode round-trip: `ContentBlock::{Text, ToolUse, ToolResult, Thinking}`
  → Responses items → back via the wire types. Property: lossless on
  every variant.
- Decode SSE: recorded fixture files in
  `crates/cogito-model/tests/fixtures/openai_responses/` (one per scenario:
  plain text completion, tool-call, reasoning summary, stop reason
  variants). Parser runs against each fixture and emits the expected
  `ModelEvent` sequence.
- Reasoning effort: `OpenAiResponsesConfig { reasoning_effort: Some(Medium) }`
  serializes the request body with the expected field.

### 15.4 Integration test

End-to-end through `cogito chat`:

- Set up a tmp dir with `.cogito/strategies/coder.md` + a stub `cogito.toml`.
- Run `cogito chat --strategy coder --message "hello"` against
  `MockModelGateway`.
- Assert: strategy was resolved, system_prompt reached the gateway,
  `allowed_tools` was honored, model_params were merged.

### 15.5 Resume-chaos addition

New scenario in `crates/cogito-core/tests/resume_chaos.rs`:
`strategy_with_tool_filter`. Setup:

- A non-trivial strategy with `allowed_tools: Allow([read_file])` and
  a non-empty `model_params`.
- Drive the existing happy-path turn through it.
- Inject crash at each existing boundary point.

Oracle: post-resume, the re-run turn sees the **same** strategy (same
allowed_tools, same model_params, same system_prompt). Verifies that
strategy is not state that gets lost mid-turn — it's re-derived
deterministically from the registry on each turn.

## 16. Migration of existing draft YAMLs

- **Delete** `strategies/claude-opus.yaml` and `strategies/gpt-4.yaml` at
  repo root (stale schema; `prompt_framing` / `tool_call_parser` /
  `context_compression` fields don't match anything we have).
- **Create** `.cogito/strategies/coder.md`,
  `.cogito/strategies/planner.md`,
  `.cogito/strategies/reviewer.md` as the example set.
- Names match the v0.3 Sprint 11 subagent role plan (planner / coder /
  critic) — when subagents land, the same files serve double duty as
  subagent role definitions. (`reviewer` is the v0.1 placeholder for
  `critic` — rename in v0.3 if needed.)
- The example strategies bind to a known provider via `provider:`
  reference; CLI users provide their own `cogito.toml` to fill in
  endpoint/credentials.
- Each example carries a `description` field that explains when to use
  it; surfaces as `cogito chat --list-strategies` output.

## 17. Workspace topology after Sprint 9a

```
crates/
  cogito-protocol/       # +StrategyRegistry trait, +StrategyError
  cogito-core/           # H10 reads Arc<dyn StrategyRegistry> via ExecCtx
  cogito-context/        # unchanged (strategy.context reuses ContextConfig)
  cogito-config/         # +default_strategy key in RuntimeConfig
  cogito-strategy/       # NEW — FsStrategyRegistry impl
  cogito-model/          # +openai_responses adapter, +ProviderConfig arm
  cogito-cli/            # +--strategy flag, +resolve_strategy helper
  cogito-tui/            # unchanged (Sprint 9b)
  crates/testing/cogito-test-fixtures/  # +MapStrategyRegistry, +contract suite
.cogito/strategies/      # NEW dir at repo root with example strategies
strategies/              # DELETED (was the stale draft YAMLs)
```

CLAUDE.md "new crate requires explicit approval" rule: `cogito-strategy`
addition is approved by this spec's acceptance.

## 18. Risks and open questions

- **Schema drift between `cogito_context::ContextConfig` and strategy
  frontmatter.** Mitigation: reuse via serde — single source of truth.
  But this couples cogito-strategy to cogito-context's serde shape;
  if Sprint 6's `ContextConfig` schema changes, strategy files break.
  Acceptable risk because cogito-context is itself a versioned protocol
  surface, not an internal type.
- **Markdown body indentation gotchas.** YAML frontmatter inside
  triple-dash delimiters is standard but operators occasionally embed
  `---` lines in markdown body (horizontal rules). Parser must consume
  *exactly* the first frontmatter block and treat the rest as body
  verbatim. Test for this.
- **File-reference path security.** A strategy YAML reading
  `system_prompt: { file: /etc/passwd }` will succeed silently. Not a
  Cogito-side security issue per se (operator chose to write the YAML;
  the contents only reach the model, not third parties), but worth
  documenting. We don't impose a sandbox boundary in 9a — operators
  who care can drop strategies into `.cogito/strategies/` only, which
  is git-tracked and thus reviewable.
- **OpenAI Responses SSE volatility.** OpenAI has historically changed
  the Responses streaming wire format. Mitigation: comprehensive
  fixture-based decode tests + clear adapter-level error messages
  pointing at the SSE event type that failed. The same risk applied to
  the Sprint 2 `openai_compat` adapter and we managed it; same playbook.
- **What if `cogito-skills` and `cogito-strategy` end up with near-identical
  frontmatter parsers?** We accept duplication for now. If both crates
  end up with non-trivial parser code, extract to
  `cogito-common-frontmatter` in a separate PR (post-9a). Per CLAUDE.md
  "three similar lines is better than a premature abstraction" — let
  the duplication ride until the case is obvious.

## 19. Acceptance criteria

Sprint 9a is done when:

1. `cogito chat --strategy coder` works end-to-end against a strategy
   defined in `.cogito/strategies/coder.md`.
2. `cogito chat --model claude-opus-4-7` (no strategy flag, no strategy
   files) still works — synthesized default preserves today's UX.
3. OpenAI Responses adapter passes all unit tests and a smoke test
   against a live endpoint (manual; documented in spec but not in CI).
4. `make ci` is green (fmt + clippy + layer-check + test).
5. New resume-chaos scenario `strategy_with_tool_filter` passes all
   4 oracles.
6. ADR-0026 ratified and merged.
7. H10 doc and `cogito-strategy` rustdoc both carry the
   strategy-vs-config framing (verifiable by grep).
8. `strategies/` directory at repo root is gone; `.cogito/strategies/`
   carries `coder.md`, `planner.md`, `reviewer.md`.
9. ROADMAP Sprint 9 entry split into 9a (this; checked off) and 9b
   (TUI; unchanged).

## 20. Out of scope (explicitly deferred)

- **TUI** — Sprint 9b.
- **DB-backed `StrategyRegistry`** — v0.4 SaaS.
- **Per-tenant strategy overrides** — v0.4 (needs `TenantContext`).
- **Hot reload** — never, by design.
- **Strategy composition / inheritance** — never, by design (Skills
  handles modular content).
- **Strategy-level hook config** — Sprint 5's `HookProvider` shape
  already allows per-strategy hook lists, but plumbing strategy →
  hook-provider selection lands when there's a concrete need.
- **OpenAI Responses built-in tools** (`file_search`, `web_search`,
  `code_interpreter`) — Hands concern, separate ADR if/when needed.

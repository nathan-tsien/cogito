# ADR-0017: Cogito Runtime configuration model

## Status

Accepted (2026-05-21).

## Context

Through Sprint 0–3, `cogito chat` reads everything it needs from CLI
arguments (`--model`, `--provider`, `--base-url`, `--session-root`,
`--session-id`, `--system`) plus a hand-coded set of environment
variables (`ANTHROPIC_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_API_KEY`).
There is no configuration file. The `strategies/*.yaml` directory
exists with two draft entries whose schema does not match the v0.1
`HarnessStrategy` field set; the loader for those files is Sprint 5
work and has not landed.

Three near-term needs make the "CLI + ENV only" surface insufficient:

1. **Issue gitlab.sz.sensetime.com/compass/cogito#1** asks for
   configuration-file-driven startup, with support for Anthropic-protocol
   third-party LLMs (different `base_url` from `api.anthropic.com`) and
   for the OpenAI `responses` API (Sprint 5).
2. **TUI surface (Sprint 6 of v0.1, with a v0.2 fallback)** ships a
   second surface that needs the same provider / model / strategy story.
   Forcing TUI to re-implement environment-variable parsing duplicates
   code and drifts.
3. **External API-server consumers (v0.4 SaaS-ready)** embed `cogito`
   inside their own service. They will not surface CLI flags to
   end-users; their configuration source is typically a database or a
   config-service rather than a TOML file on disk. They still need the
   same logical configuration shape (provider connections, strategies,
   etc.) — the source differs, the schema does not.

Additionally, future versions add more configurable concerns:

- **Strategies** (Sprint 5, H10): the prompt / tool-filter / model-params
  recipe per strategy. H10 doc already designs YAML schema.
- **Plugins** (mechanism TBD, post-v0.3).
- **Subagents** (v0.3): subagent strategy + depth limits.

Without an architectural anchor, every new configurable concern risks
inventing its own source / format / merge / secret-handling rules. We
have already seen this anti-pattern (`strategies/*.yaml` exists with an
incompatible schema; CLI parses ENV inline in `chat.rs`; no two
consumers will load configuration the same way unless a contract
forces them to).

This ADR locks **the configuration model** — sections, sources,
composition, secrets, search path, profile policy, crate placement, and
the boundary value type — so that Sprint 4.5 implementation and all
later sprints (strategies, plugins, subagents, database source) plug
into the same scaffold instead of forking it.

The ADR does **not** specify the field set of every future section.
Sections that have not yet been designed (plugins, subagents) get
reserved slots; sections that have been designed elsewhere (strategies
via H10 doc) are referenced, not re-litigated.

## Decision

### 1. Section taxonomy

The configuration schema is partitioned into top-level sections, each
owned by a clear concern:

| Section       | When locked | Owner                          | Notes |
|---------------|-------------|--------------------------------|-------|
| `runtime`     | Now (ADR-0017) | `cogito-config::RuntimeSection`   | Process-wide startup: session root, default selection, strategies dir |
| `providers`   | Now (ADR-0017) | `cogito-model::ProviderConfig`    | Named provider instances (kind + credentials + connection) |
| `strategies`  | Now (ADR-0017) | H10 doc + `cogito-protocol::HarnessStrategy` | YAML files under `strategies_dir`; loader is Sprint 5 |
| `plugins`     | Reserved    | TBD post-v0.3                  | Slot named, schema deferred until plugin mechanism ADR |
| `subagents`   | Reserved    | TBD v0.3 (ADR-0011)            | Slot named, schema deferred until subagent ADR |

Reserved sections deserialize without error: the top-level
`RuntimeConfigPartial` does **not** apply `#[serde(deny_unknown_fields)]`,
so a future `[plugins]` or `[[subagents]]` entry in `cogito.toml` is
parsed and ignored by Sprint 4.5. Inner structs (`RuntimeSectionPartial`,
`ProviderConfig` variants) **do** apply `deny_unknown_fields` to catch
typos. Adding a real schema later is an additive change: add the
section's `Option<Section>` field; old configs (without the section)
continue to parse as `None`.

### 2. File layout

The file source uses a **hybrid two-format layout**:

- **`cogito.toml`** (TOML) — single file that holds `[runtime]` plus
  the `[[providers]]` array (and, when sections land, `[[plugins]]` /
  `[[subagents]]`).
- **`strategies/*.yaml`** (one strategy per file) — directory pointed
  to by `runtime.strategies_dir`. Each YAML file's basename (without
  the `.yaml` extension) is the strategy's name. Schema per H10 doc.

Why two formats: strategies are a registry that grows over time and
carries multi-line `system_prompt` content. YAML's `|` block scalar +
per-file granularity match this shape. Runtime + providers are a small
fixed-size configuration; TOML matches the Rust ecosystem convention
(Cargo, rustfmt, clippy). Each format is used where it is the right
tool; the cost ("two formats to learn") is bounded because the two
domains rarely interleave.

The database source (v0.4+, consumer-implemented) produces the same
logical schema. Files vs. database is a `ConfigLoader` impl detail; the
final `RuntimeConfig` value type is identical.

### 3. Source composition

Configuration comes from multiple sources, combined by a fixed
precedence:

```
CLI args     >  ENV vars  >  File (or Database)  >  Defaults
(highest)                                          (lowest)
```

Each source implements `ConfigLoader::load() -> RuntimeConfigPartial`.
A reducer merges layers in precedence order — later layers' `Some(_)`
fields override earlier layers'.

- **CLI layer** is surface-specific. `cogito-cli` and (future)
  `cogito-tui` each produce their own `RuntimeConfigPartial` from
  their argument parsers and apply it as the top layer.
- **ENV layer** (`EnvConfigLoader` in `cogito-config`, std-only) reads
  a fixed set of variables.
- **File layer** (`FileConfigLoader`, feature-gated `file` in
  `cogito-config`) reads `cogito.toml` + walks `strategies/*.yaml`.
- **Database layer** (v0.4+) is implemented by the consumer's Server
  code, not by this repository, unless a `cogito-server-bootstrap`
  crate is later added (not in ROADMAP).
- **Defaults** are baked into `RuntimeConfig::finalize`.

Arrays (`providers`, future `plugins`, future `subagents`) are
**replaced wholesale** by a later layer, not merged element-wise.
Per-field overrides inside a single provider are not achieved via
array merge; they are achieved via secret interpolation (§6).
Rationale: element-wise array merge requires a join key and an
override policy per field — operationally complex and error-prone. The
"file declares the array; ENV provides only secret values referenced
from inside" pattern covers the common case.

### 4. Provider/model schema

Providers are declared as a **named instance array**, tagged-union over
provider kind:

```toml
[[providers]]
name = "anthropic-prod"
kind = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
base_url = "https://api.anthropic.com"

[[providers]]
name = "anthropic-internal"
kind = "anthropic"
api_key = "${INTERNAL_KEY}"
base_url = "https://internal.api/anthropic/v1"

[[providers]]
name = "vllm-cluster"
kind = "openai-compat"
api_key = "${VLLM_API_KEY}"
base_url = "http://vllm.svc:8000/v1"
```

The corresponding Rust type lives in `cogito-model` (the crate that
owns the gateway implementations), serde-tagged on `kind`:

```rust
// crates/cogito-model/src/provider_config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderConfig {
    Anthropic {
        name: String,
        api_key: String,
        #[serde(default = "defaults::anthropic_base_url")]
        base_url: String,
        #[serde(default = "defaults::anthropic_version")]
        anthropic_version: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    OpenAiCompat {
        name: String,
        api_key: Option<String>,
        base_url: String,
        #[serde(default = "defaults::auth_header")]
        auth_header: String,
        #[serde(default = "defaults::auth_scheme")]
        auth_scheme: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    // OpenAiResponses { ... } lands in Sprint 5
}
```

A single factory in `cogito-model` performs the kind-dispatch and
returns the trait object:

```rust
// crates/cogito-model/src/lib.rs
pub fn build_gateway(cfg: ProviderConfig)
    -> Result<Arc<dyn ModelGateway>, ModelError>;
```

This factory is **the only place** that knows the mapping from
`kind` to `dyn ModelGateway`. Surface code (`cogito-cli`,
`cogito-tui`, consumer Server) never pattern-matches on the kind tag;
it calls `build_gateway(cfg)` and receives the trait object. This is
the "tagged-config factory" rule in `CLAUDE.md` §Coding standards;
adding a new provider variant (Sprint 5 OpenAI Responses) edits only
`cogito-model`.

### 5. Crate placement

A new crate `cogito-config` carries the configuration concern:

| Surface (consumer)    | Default features         | `file` feature |
|-----------------------|--------------------------|----------------|
| `cogito-cli`          | yes                      | yes            |
| `cogito-tui` (v0.2)   | yes                      | yes            |
| Consumer's Server     | yes (impl own loader)    | no             |

- **Default features** (no file-format parsers) — `RuntimeConfig` /
  `RuntimeConfigPartial` value types, `ConfigLoader` trait,
  `EnvConfigLoader`, merge logic. Depends only on light workspace
  basics (`serde`, `thiserror`, `async-trait`, `tracing`); no
  `toml`, no `serde_yaml`, no `sqlx`.
- **Feature `file`** — pulls `toml` + `serde_yaml`, adds
  `FileConfigLoader`. Surfaces that read a config file from disk
  enable this feature; surfaces that load from elsewhere (databases,
  in-memory) do not.

`cogito-config` depends on `cogito-protocol` (for `HarnessStrategy`)
and `cogito-model` (for `ProviderConfig` schema). It does **not**
depend on `cogito-core`. `cogito-core::runtime` consumes a finalized
`RuntimeConfig` value and a pre-built set of trait objects; it has no
knowledge of where the configuration came from. The Brain layer
(`cogito-core::harness`) is unchanged.

**Database-source loaders (v0.4+) are not part of `cogito-config`.**
They belong to the consumer's Server code (or a future
`cogito-server-bootstrap` crate, not in ROADMAP). This keeps
`cogito-config` from accumulating `sqlx` / `tokio-postgres` /
`redis` / etc. dependencies as Server backends multiply. Consumers
that need a custom source implement the `ConfigLoader` trait directly.

Crate increment: **+1** (`cogito-config`). No further crates required
by this ADR.

### 6. Secret interpolation

Configuration values may reference environment variables using shell
syntax. Two forms are supported, applied by the file loader after
TOML/YAML parsing and before merge:

```
${VAR_NAME}            -> value of $VAR_NAME; ConfigError if unset
${VAR_NAME:-default}   -> value of $VAR_NAME; default if unset/empty
```

Interpolation runs over every string field at the file-source level.
ENV loader and CLI loader contribute already-resolved values and do
not interpolate.

The `${VAR}` form errors out if the variable is missing, by design:
secret-bearing fields (`api_key`) should fail loudly rather than be
silently empty. The `${VAR:-default}` form supports optional values
with a fallback.

### 7. File search path

`FileConfigLoader` resolves the `cogito.toml` path in this strict
order — the first hit wins, the rest are not consulted (no inner
merge across paths):

1. `--config <path>` CLI argument (if surface supplied)
2. `COGITO_CONFIG` environment variable
3. `./cogito.toml` (project-local working directory)
4. `$XDG_CONFIG_HOME/cogito/config.toml` (XDG default; Linux/Mac;
   Windows path is reserved for future addition)
5. None of the above — `FileConfigLoader` returns
   `RuntimeConfigPartial::default()` (entirely `None`); ENV + CLI
   + defaults cover the remaining shape

Rationale for "first hit wins, no inner merge": layered merge already
provides four sources of override (CLI > ENV > file > defaults).
Adding a fifth layer ("user-wide overlay merged with project-local
overlay") makes "where did this field come from" exponentially harder
to debug. Project-local config wins over user-wide config; if a user
wants to share configuration across projects, they put it in
`$XDG_CONFIG_HOME` and omit `./cogito.toml`.

### 8. Profile / multi-environment policy

**No profile concept in v0.1.** `cogito.toml` does not support
`[profile.dev]` overlays or any equivalent. Users with multiple
environments (dev / staging / prod) use one of:

- Multiple files (`cogito.dev.toml`, `cogito.prod.toml`) selected via
  `--config <path>` or `COGITO_CONFIG`.
- Multiple shell aliases, `make` targets, or CI-matrix entries that
  set the right path.

Rationale: cogito is an embeddable library; environment selection is
a deployment concern, not a runtime concern. Adding a profile system
adds another merge layer (per ADR §3 we kept the layers count
deliberate) and another debug-failure mode without proven user
demand. If demand surfaces in v0.4+, a future ADR can add profiles
without breaking the file format (a profile-aware loader is a
superset of the current loader).

### 9. Strategy YAML schema

This ADR does **not** redefine the strategy field set; it defers to
`docs/components/H10-strategy-selector.md` §"v0.x Sprint 5 scope".
Two ADR-level refinements:

1. **Filename is the strategy name.** `strategies/code-review.yaml`
   defines a strategy whose name is `code-review`. No `name:` field
   inside the YAML body. Rationale: a single source of truth avoids
   "filename says X, body says Y" drift.
2. **No `applicable_models:` glob.** The two existing draft files
   (`claude-opus.yaml`, `gpt-4.yaml`) use a different schema with
   `applicable_models` and `prompt_framing`; that schema is
   superseded. The H10 doc's example schema (with `model_id`,
   `system_prompt`, `allowed_tools`, `tool_order`, `model_params`,
   `max_turns`) is the canonical shape.

The existing two draft files are obsolete and will be rewritten when
Sprint 5 implements the loader.

### 10. Strategy ↔ provider binding

Strategies are **provider-agnostic**. A strategy YAML file declares
`model_id` and prompt/tool configuration but does **not** name a
provider. The selection of which provider serves a strategy happens
at runtime:

1. Surface picks a strategy (e.g., `cogito chat --strategy
   code-review` in Sprint 5).
2. Surface picks a provider (`--provider`, or
   `runtime.default_provider`, or auto-select when only one exists).
3. Runtime sends a model call: the strategy's `model_params.model`
   string is the wire-level model id; the provider determines the
   endpoint and auth.

If a model id is not valid for the chosen provider (e.g., sending
`qwen-72b` to an Anthropic endpoint), the provider's HTTP layer
returns 4xx and `ModelError::Provider` propagates. No static
validation at startup; this would require maintaining a registry of
"which models each provider serves" with no clean source.

Rationale: decoupling lets the same strategy (recipe) run against
multiple providers (endpoints) — including the "real Anthropic vs.
internal Anthropic-compat endpoint" case that motivates Issue #1.
The cost is that a strategy is a slightly leaky abstraction (its
`model_id` implies a provider family); the cost is bounded by the
clear failure mode at HTTP-time.

### 11. CLI backward compatibility

The existing CLI surface (`cogito chat --model X --provider Y
--base-url Z --session-root D --system "..."`) keeps working
unchanged in Sprint 4.5 even when `cogito.toml` is absent.

Mechanism: each CLI flag maps to a `RuntimeConfigPartial` patch
applied in the CLI layer. Specifically:

- `--session-root D` → `runtime.session_root = D`
- `--model X` → `runtime.default_model = X`
- `--provider Y` → `runtime.default_provider = Y`
- `--system "..."` → strategy `system_prompt` override applied
  post-finalization to the selected strategy value
- `--base-url Z` → applied post-finalization as a patch to the
  selected provider entry's `base_url` field. If the selected
  provider does not have a `base_url` (none of v0.1's variants lack
  it), the flag is silently a no-op; this is consistent with current
  CLI behavior.

When `cogito.toml` is absent and no providers are declared, the CLI
layer additionally synthesizes a single provider entry from the
legacy ENV variables (`ANTHROPIC_API_KEY` → an `Anthropic` provider
named `default`; `OPENAI_BASE_URL` + `OPENAI_API_KEY` → an
`OpenAiCompat` provider named `default`). This is the legacy-bridge
mode that preserves `just chat` behavior with zero migration.

### 12. RuntimeConfig Rust types

```rust
// crates/cogito-config/src/types.rs

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub runtime: RuntimeSection,
    pub providers: Vec<cogito_model::ProviderConfig>,
    pub strategies: HashMap<String, cogito_protocol::strategy::HarnessStrategy>,
    // plugins:   Reserved
    // subagents: Reserved
}

#[derive(Debug, Clone)]
pub struct RuntimeSection {
    pub session_root: PathBuf,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub strategies_dir: PathBuf,
}

// Top-level intentionally does NOT use deny_unknown_fields, so a
// future `[plugins]` or `[[subagents]]` section in cogito.toml is
// parsed and silently dropped by Sprint 4.5. Inner structs do apply
// deny_unknown_fields to catch typos within a known section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfigPartial {
    pub runtime: Option<RuntimeSectionPartial>,
    pub providers: Option<Vec<cogito_model::ProviderConfig>>,
    // plugins / subagents: tolerated by absence of deny_unknown_fields
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSectionPartial {
    pub session_root: Option<PathBuf>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub strategies_dir: Option<PathBuf>,
}

#[async_trait::async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError>;
}
```

Defaults applied by `RuntimeConfigPartial::finalize`:

- `runtime.session_root` → `PathBuf::from("./sessions")`
- `runtime.strategies_dir` → `PathBuf::from("./strategies")`
- `runtime.default_provider` → `None`; if `providers.len() == 1`,
  finalize substitutes the sole provider's name
- `runtime.default_model` → `None`; if unset, `RuntimeBuilder`
  requires the surface (e.g., CLI `--model`) to supply one
- `providers` → empty `Vec` (caller must supply at least one, either
  via file or via CLI legacy-bridge)
- `strategies` → empty `HashMap` (Sprint 4.5; Sprint 5 populates it
  by walking `strategies_dir`)

### 13. What this ADR does NOT decide

- **The exact field set of `[runtime]` beyond the four locked here.**
  Telemetry config, tracing config, retry policy globals, etc. are
  intentionally not in `[runtime]` for v0.1. Future ADRs may add them
  additively.
- **Plugin section schema.** Plugins are a post-v0.3 concern;
  the slot in the file format is reserved but the schema is undefined.
- **Subagent section schema.** Same — reserved for ADR-0011 (v0.3).
- **Database-source schema and migration.** Consumer-implemented in
  v0.4+. The `ConfigLoader` trait is the only contract surface this
  ADR locks; the database table layout is a consumer concern.
- **Hot reload.** Config is loaded once at process startup. Hot
  reload is a future capability if/when operational need surfaces.
- **Strategy selection by `(model_id, task)` tuple.** H10 doc leaves
  this open; this ADR does not close it.

## Consequences

**Easier:**

- New surfaces (`cogito-tui` in v0.2, consumer Server in v0.4) share
  one configuration scaffold. CLI argument parsing forks per surface,
  but the value-type → trait-object pipeline is identical.
- Adding a new provider kind (Sprint 5 `OpenAiResponses`) requires
  editing only `cogito-model`: one enum variant + one factory arm.
  Surface code and `cogito-config` are unchanged.
- Adding a new configurable concern (plugins / subagents) adds a
  section to the schema and a value-type module. Sources (file / ENV
  / database) pick it up via `RuntimeConfigPartial` extension without
  changes to source-loading code.
- Consumers with custom configuration sources implement one trait
  (`ConfigLoader`) and plug into the existing merge pipeline. No
  cogito-side coordination required.
- Secret values stay out of files: `${ENV_VAR}` interpolation lets
  `cogito.toml` be safely checked into version control.

**Harder / cost:**

- One new crate (`cogito-config`). Crate count is a scarce resource
  per CLAUDE.md §Workspace layout; this is the +1 of a v0.1 cycle.
- The `Partial<T>` pattern adds one shadow type per locked section.
  Manageable for the four sections this ADR locks; if section count
  grows past ~10, the boilerplate cost will warrant a derive macro.
- Layered-merge debugging ("where did this field come from") requires
  diagnostic helpers (`tracing` at DEBUG level on every layer
  contribution). Cost ≈ a half-day of telemetry wiring; cheaper than
  the alternative of users chasing it field by field.

**Given up:**

- The simplicity of "the CLI is the only configuration surface". v0.1
  worked fine with this; v0.2 (TUI) would not. Paying for the
  abstraction now avoids a forced retrofit when TUI lands.
- A profile / overlay system that some users will ask for. We say no
  for v0.1 (§8); the door is open if a future ADR proves the demand.
- Element-wise array merge inside `[[providers]]`. Users cannot say
  "the file declares 3 providers; ENV overrides the second one's
  base_url". They can use `${ENV_VAR}` inside the file's `base_url`
  field instead, which is more explicit and easier to audit.

## References

- `CLAUDE.md` §Coding standards — "Tagged-config factories belong in
  the crate that owns the implementations"
- `AGENTS.md` §Inviolable design principles #6 — Brain may only see
  Hands / Session / Boundary through Protocol
- ADR-0004 — Brain / Hands / Session crate boundaries
- ADR-0007 — Event log as cross-language storage contract (additive
  variants, `#[non_exhaustive]` discipline)
- `docs/components/H10-strategy-selector.md` — strategy YAML schema
  and Sprint 5 loader plan
- `crates/cogito-model/src/anthropic/mod.rs` — `AnthropicConfig`
  (already carries `base_url`; this ADR surfaces it through the
  configuration model)
- `crates/cogito-model/src/openai_compat/mod.rs` — `OpenAiCompatConfig`
- `crates/cogito-cli/src/chat.rs` — Sprint 2 CLI surface to be
  refactored in Sprint 4.5
- ROADMAP §"Sprint 4" (Async Jobs — unchanged) and §"Sprint 5"
  (OpenAI Responses adapter + H10 YAML loader — picks up
  `strategies_dir` from this ADR)
- Issue gitlab.sz.sensetime.com/compass/cogito#1
- 2026-05-21 brainstorming transcript leading to this ADR

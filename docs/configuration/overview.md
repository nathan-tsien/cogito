# Cogito Runtime Configuration · Overview

> **Architectural anchor**: [ADR-0017](../adr/0017-cogito-runtime-configuration-model.md)
> **First implementation**: [Sprint 4.5 spec](../superpowers/specs/2026-05-21-sprint-4-5-config-file-design.md)
> **Owners**: `cogito-config` crate, `cogito-model::ProviderConfig`

This document is the **orientation map** for cogito's configuration
story. It is meant to be the first thing a newcomer reads to
understand "how does configuration work end-to-end" — without forcing
them through ADRs, sprint specs, and component docs first.

| If you want to… | Read |
|---|---|
| Understand the whole picture | This doc (start to finish) |
| Use configuration in a project | §2 (Quick start) + §6 (Section reference) |
| Understand a specific decision's rationale | [ADR-0017](../adr/0017-cogito-runtime-configuration-model.md) |
| Implement Sprint 4.5 | [Sprint 4.5 spec](../superpowers/specs/2026-05-21-sprint-4-5-config-file-design.md) |
| Change the architecture | Write a new ADR (do not edit this doc alone) |

This doc is **derivative**: it summarizes durable artifacts (ADRs,
component docs, code). When ADR-0017 and this doc disagree, ADR-0017
wins; please open an MR to bring this doc back in sync.

---

## 1. TL;DR

Cogito reads configuration from multiple sources (CLI args, environment
variables, `cogito.toml`, optionally a database in v0.4+), merges them
in a fixed precedence (`CLI > ENV > file/db > defaults`), and produces
a single typed value `RuntimeConfig`. Surface code (`cogito-cli`,
future `cogito-tui`, consumer's Server) uses `RuntimeConfig` to build
concrete `ModelGateway` / `ToolProvider` / `StrategyRegistry` instances
which are injected into the Runtime. The Brain layer never sees raw
configuration — it sees already-built trait objects.

Configuration is partitioned into **sections** by concern: `runtime`,
`providers`, `strategies` (locked now); `plugins`, `subagents`
(reserved for later). Two file formats: `cogito.toml` for the small
fixed-size sections, `strategies/*.yaml` for the strategy registry.
Secrets stay out of files via `${ENV_VAR}` interpolation.

---

## 2. Quick start

The minimum useful `cogito.toml` for `cogito chat` against Anthropic:

```toml
[runtime]
session_root    = "./sessions"
default_model   = "claude-opus-4-7"

[[providers]]
name    = "anthropic"
kind    = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
```

Run:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cogito chat                    # picks up ./cogito.toml automatically
cogito chat --config /etc/cogito.toml   # explicit path
```

No `cogito.toml`? Sprint 4.5 falls back to a legacy ENV bridge that
synthesizes a `default` provider from `ANTHROPIC_API_KEY` /
`OPENAI_API_KEY` / `OPENAI_BASE_URL` — preserving Sprint 2 behavior
with zero migration. See §10.

---

## 3. The shape: section taxonomy

Configuration is partitioned into top-level sections, each owned by a
clear concern. Sections are loaded independently and composed into one
`RuntimeConfig`.

| Section       | Status      | Owner crate                       | Notes |
|---------------|-------------|-----------------------------------|-------|
| `runtime`     | Locked      | `cogito-config::RuntimeSection`   | Process-wide startup |
| `providers`   | Locked      | `cogito-model::ProviderConfig`    | Named provider instances |
| `strategies`  | Locked      | `cogito-protocol::HarnessStrategy` (loader: `cogito-config`) | YAML registry, dir-based |
| `plugins`     | Reserved    | TBD post-v0.3                     | Slot named; schema deferred |
| `subagents`   | Reserved    | TBD v0.3                          | Slot named; schema deferred |

"Locked" sections have a defined schema and a designated owner crate
that maintains it. "Reserved" sections deserialize silently (no
`deny_unknown_fields` at the top level) so a forward-looking
`cogito.toml` can include them without breaking older parsers.

---

## 4. Sources and composition

Configuration arrives from multiple sources. Each source produces a
`RuntimeConfigPartial` (every field is `Option<T>`). A reducer merges
the partials by precedence:

```text
                          ┌─────────────────────┐
                          │   CLI args          │  ◄── highest priority
                          └──────────┬──────────┘
                                     │
                          ┌──────────▼──────────┐
                          │   ENV variables     │
                          └──────────┬──────────┘
                                     │
                  ┌──────────────────▼──────────────────┐
                  │   File (cogito.toml + YAML dir)     │
                  │     OR Database (v0.4+ consumer)    │
                  └──────────────────┬──────────────────┘
                                     │
                          ┌──────────▼──────────┐
                          │   Defaults          │  ◄── lowest priority
                          └──────────┬──────────┘
                                     │
                                     ▼
                          ┌─────────────────────┐
                          │  RuntimeConfig      │  (finalize)
                          └─────────────────────┘
```

Later sources override earlier ones field-by-field. Arrays
(`providers`, future `plugins`, future `subagents`) replace
wholesale — no element-wise merge. To override one field of one
provider, use secret interpolation inside the file (see §5).

Each source is a `ConfigLoader`:

```rust
#[async_trait::async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError>;
}
```

The default Sprint 4.5 lineup:

| Source     | Loader                       | Lives in              | Activation |
|------------|------------------------------|-----------------------|------------|
| File       | `FileConfigLoader`           | `cogito-config`       | feature `file` |
| ENV        | `EnvConfigLoader`            | `cogito-config`       | always |
| CLI        | (surface-specific)           | `cogito-cli`          | always |
| Database   | (consumer-supplied)          | consumer's Server     | v0.4+ |

Consumers with custom sources (internal config service, etc.)
implement `ConfigLoader` and plug into the same merge pipeline.

---

## 5. Secret handling

Secrets do not live in files. The file loader interpolates two forms
of placeholder over every string field after parsing, before merge:

```text
${VAR_NAME}           → value of $VAR_NAME; ConfigError if unset
${VAR_NAME:-default}  → value of $VAR_NAME; default if unset/empty
```

```toml
[[providers]]
name    = "anthropic-prod"
kind    = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"          # required; errors if missing
base_url = "${ANTHROPIC_BASE_URL:-https://api.anthropic.com}"
```

The file may be safely checked into version control: secret values
stay in the environment.

---

## 6. Section reference

### `[runtime]`

Process-wide startup options.

| Field             | Type            | Default            | Notes |
|-------------------|-----------------|--------------------|-------|
| `session_root`    | `PathBuf`       | `"./sessions"`     | JSONL store directory |
| `default_provider`| `Option<String>`| auto if 1 provider | Provider name to use when `--provider` is absent |
| `default_model`   | `Option<String>`| —                  | Wire-level model id; required if CLI doesn't pass `--model` |
| `strategies_dir`  | `PathBuf`       | `"./strategies"`   | YAML directory; consumed by Sprint 6 loader |

### `[[providers]]`

Named connection-and-credential entries. Owner: `cogito-model::ProviderConfig`.
Serde-tagged on `kind`; each variant has its own field set.

```toml
# Variant: kind = "anthropic"
[[providers]]
name              = "anthropic-prod"        # required
kind              = "anthropic"             # required
api_key           = "${ANTHROPIC_API_KEY}"  # required
base_url          = "https://api.anthropic.com"   # default shown
anthropic_version = "2023-06-01"            # default shown
timeout_secs      = 300                     # optional

# Variant: kind = "openai-compat"
[[providers]]
name         = "vllm-cluster"               # required
kind         = "openai-compat"              # required
base_url     = "http://vllm.svc:8000/v1"    # required
api_key      = "${VLLM_API_KEY}"            # optional
auth_header  = "Authorization"              # default shown
auth_scheme  = "Bearer"                     # default shown
timeout_secs = 300                          # optional
```

Multiple instances of the same `kind` are supported: a user may
declare both "real Anthropic" and "internal Anthropic-compatible
endpoint" as two separate `kind = "anthropic"` entries.

Future variants land in `cogito-model` without changing this doc's
shape — see §8.

### `strategies/*.yaml`

One YAML file per strategy, under `runtime.strategies_dir`. Filename
(without `.yaml`) is the canonical strategy name; the YAML body
contains no `name:` field.

Schema authoritative source: [`docs/components/H10-strategy-selector.md`](../components/H10-strategy-selector.md).
Loader lands in **Sprint 6** — Sprint 4.5 reserves the field but
does not walk the directory.

Strategies are **provider-agnostic**: the YAML does not name a
provider. The runtime binds strategy and provider at request time
via `--provider <name>` or `runtime.default_provider`.

### `[[mcp_servers]]`

The `[[mcp_servers]]` array configures Model Context Protocol servers
whose tools surface alongside cogito's built-ins. See
[ADR-0018](../adr/0018-mcp-integration.md) for the full contract.

**Transports:**

stdio:

```toml
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "uvx"
args = ["mcp-server-filesystem", "/tmp"]
env = { LOG_LEVEL = "info" }     # optional
startup_timeout_sec = 10         # optional, default 10
tool_timeout_sec = 60            # optional, default 60
enabled_tools = ["read_file"]    # optional allowlist
disabled_tools = []              # optional denylist
```

streamable-HTTP:

```toml
[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"
http_headers = { "X-Tenant" = "acme" }
```

**Verbose tool descriptions.** If your MCP servers produce verbose
tool descriptions, use `enabled_tools` to narrow the catalog. Future
strategy-level token budgets (Sprint 6 H10 + the Context Management
spike) will enforce per-turn limits automatically; v0.1 leaves the
choice to you.

**stdio `args` path resolution.** `args` entries are passed verbatim
to the child process; cogito performs no path expansion, no `~` /
`$VAR` substitution, and no absolutization. Relative paths resolve
against the child's working directory, which inherits from the cogito
CLI process. If you need a specific working directory, wrap with
`command = "bash"`, `args = ["-c", "cd /path && exec the-server"]`.

**Failure behavior.** MCP server failures never block `cogito chat`
startup. Each configured server is announced on stderr with its
status; the agent continues with whatever tools came up. To make a
missing server fatal, you currently need a wrapper script — a
built-in `strict_mcp_startup` mode is on the v0.4 SaaS-ready roadmap.

---

## 7. File search path

`FileConfigLoader` resolves the `cogito.toml` path in this order. The
**first hit wins**; later paths are not consulted (no inner merge).

1. `--config <path>` CLI argument (if the surface supplies it)
2. `COGITO_CONFIG` environment variable
3. `./cogito.toml` (project-local working directory)
4. `$XDG_CONFIG_HOME/cogito/config.toml` (XDG default on Linux/Mac)
5. No file found → loader returns empty partial; ENV + CLI + defaults
   cover the remaining shape (this is the **legacy ENV bridge** mode;
   see §10)

The "first hit wins" rule keeps "where did this field come from"
answerable by a single layer-by-layer trace; adding an inner merge
would make debugging exponentially harder. To share configuration
across projects, put it in `$XDG_CONFIG_HOME` and omit
`./cogito.toml`.

---

## 8. Version evolution

| Capability                                | Lands in      | Owner artifact |
|-------------------------------------------|---------------|----------------|
| `[runtime]` + `[[providers]]` schema      | Sprint 4.5    | ADR-0017       |
| `FileConfigLoader` + `EnvConfigLoader`    | Sprint 4.5    | ADR-0017       |
| Layered partial merge                     | Sprint 4.5    | ADR-0017       |
| Legacy ENV bridge (no `cogito.toml`)      | Sprint 4.5    | Sprint 4.5 spec |
| `${ENV_VAR}` + `${ENV_VAR:-default}`      | Sprint 4.5    | ADR-0017 §6    |
| `cogito-model::ProviderConfig::Anthropic` | Sprint 4.5    | Sprint 4.5 spec |
| `cogito-model::ProviderConfig::OpenAiCompat` | Sprint 4.5 | Sprint 4.5 spec |
| `cogito-model::ProviderConfig::OpenAiResponses` | Sprint 6 | ROADMAP §"Sprint 6" |
| `strategies/*.yaml` loader (H10 registry) | Sprint 6      | H10 doc        |
| `--strategy <name>` CLI                   | Sprint 6      | H10 doc        |
| `[plugins]` schema                        | post-v0.3     | future ADR     |
| `[[subagents]]` schema                    | v0.3          | ADR-0011 (reserved) |
| Database `ConfigLoader` (consumer-impl)   | v0.4+         | consumer code  |
| Hot reload                                | not planned   | —              |
| Profile / multi-environment overlay       | not planned   | (use multiple `--config` files) |

Sections marked "Reserved" deserialize silently in Sprint 4.5;
adding them later is an additive serde change, not a breaking one.

---

## 9. Crate map

```text
                  ┌──────────────────────┐
                  │   cogito-protocol    │  HarnessStrategy, traits
                  └──────────┬───────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
       ┌─────────────┐ ┌──────────────┐ ┌──────────────┐
       │ cogito-     │ │ cogito-model │ │ cogito-tools │
       │   core      │ │              │ │              │
       │ (Brain +    │ │ ProviderConfig│ │ ...          │
       │  Runtime)   │ │ build_gateway│ │              │
       └─────────────┘ └───────┬──────┘ └──────────────┘
                               │
                  ┌────────────▼────────────┐
                  │   cogito-config (NEW)   │
                  │   RuntimeConfig,        │
                  │   ConfigLoader trait,   │
                  │   FileConfigLoader,     │
                  │   EnvConfigLoader,      │
                  │   merge + finalize      │
                  └────────────┬────────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                ▼
       ┌──────────────┐ ┌──────────────┐ ┌──────────────────┐
       │  cogito-cli  │ │ cogito-tui   │ │ consumer's Server│
       │  (v0.1)      │ │ (v0.2)       │ │ (v0.4+, external)│
       └──────────────┘ └──────────────┘ └──────────────────┘
```

Key constraints (per ADR-0004 / ADR-0017 §5):

- **`cogito-core` does not depend on `cogito-config`.** The Runtime
  layer consumes a finalized `RuntimeConfig` and a pre-built set of
  trait objects; it has no idea where the config came from.
- **`cogito-config` does not depend on `cogito-core`.** It depends on
  `cogito-protocol` (for `HarnessStrategy`) and `cogito-model` (for
  `ProviderConfig`).
- **`cogito-model` owns the provider-kind dispatch.** Per the
  "tagged-config factories" rule in [`CLAUDE.md`](../../CLAUDE.md)
  §Coding standards, the `match`-on-`kind` lives in
  `cogito_model::build_gateway`, not in Surface crates.
- **Database loaders for v0.4+ are not in `cogito-config`.** They are
  consumer-side code, or a future `cogito-server-bootstrap` crate
  (not in ROADMAP). This keeps `cogito-config` from accumulating
  `sqlx` / `redis` / etc.

---

## 10. Backward compatibility: the legacy ENV bridge

Sprint 4.5 must not break Sprint 2's workflow. If no `cogito.toml` is
found (search path §7 exhausted) AND the CLI does not declare a
provider explicitly, the CLI layer synthesizes a single provider
entry named `default` from the legacy environment variables:

| Legacy ENV var       | Synthesizes |
|----------------------|-------------|
| `ANTHROPIC_API_KEY`  | `[[providers]] name="default" kind="anthropic" api_key="${ANTHROPIC_API_KEY}" base_url="https://api.anthropic.com"` |
| `OPENAI_API_KEY` + `OPENAI_BASE_URL` | `[[providers]] name="default" kind="openai-compat" api_key="${OPENAI_API_KEY}" base_url="${OPENAI_BASE_URL}"` |

Selection in legacy mode follows Sprint 2 rules:

- `--provider` defaults to `anthropic` when the model id starts with
  `claude-`, otherwise `openai-compat` (Sprint 2's `build_gateway`
  inference logic).
- The synthesized provider's name is always `default`, but the user
  does not need to pass `--provider default` — the auto-select rule
  (one provider → use it) handles this.

This guarantees `just chat --model claude-opus-4-7` works with only
`ANTHROPIC_API_KEY` set, exactly as it did in Sprint 2.

The legacy bridge is **not** a permanent feature; it is a migration
runway. When the project decides to require explicit
`cogito.toml` / `--config`, a future ADR will deprecate this
synthesis.

---

## 11. Data flow: from `cogito chat` to a model call

```text
                            cogito chat --model X
                                    │
                                    ▼
            ┌────────────────────────────────────────┐
            │  cogito-cli parses args (clap)         │
            └───────────────────┬────────────────────┘
                                │
                                ▼
            ┌────────────────────────────────────────┐
            │  cogito-config::load_runtime_config()  │
            │                                        │
            │    ┌───────────────────┐               │
            │    │ FileConfigLoader  │ ─► Partial    │
            │    └───────────────────┘               │
            │    ┌───────────────────┐               │
            │    │ EnvConfigLoader   │ ─► Partial    │
            │    └───────────────────┘               │
            │    ┌───────────────────┐               │
            │    │ CLI args patch    │ ─► Partial    │
            │    └───────────────────┘               │
            │             │                          │
            │             ▼                          │
            │     merge_layers + finalize            │
            │             │                          │
            │             ▼                          │
            │       RuntimeConfig                    │
            └───────────────────┬────────────────────┘
                                │
                                ▼
            ┌────────────────────────────────────────┐
            │  select_provider(&cfg, &args)          │
            │       ─► ProviderConfig (one variant)  │
            └───────────────────┬────────────────────┘
                                │
                                ▼
            ┌────────────────────────────────────────┐
            │  cogito_model::build_gateway(cfg)      │
            │       ─► Arc<dyn ModelGateway>         │
            └───────────────────┬────────────────────┘
                                │
                                ▼
            ┌────────────────────────────────────────┐
            │  Runtime::builder()                    │
            │       .model(gateway)                  │
            │       .store(jsonl_store)              │
            │       .tools(...)                      │
            │       .strategy(...)                   │
            │       .build()                         │
            └───────────────────┬────────────────────┘
                                │
                                ▼
            ┌────────────────────────────────────────┐
            │  Per-turn agent loop (H01–H11)         │
            │  Brain sees: trait objects only.       │
            │  Brain does NOT see: ProviderConfig,   │
            │  RuntimeConfig, file paths, ENV vars.  │
            └────────────────────────────────────────┘
```

The vertical line between "build" and "agent loop" is the **layer
boundary** specified by ADR-0004 and AGENTS.md §6: above the line,
configuration is concrete and source-aware; below the line, the Brain
sees only protocol traits.

---

## 12. What this document does NOT cover

- **Per-provider wire-level behavior** — that is `cogito-model`'s
  internal concern (request/response encoding, SSE handling). See
  `crates/cogito-model/src/anthropic/` and `openai_compat/`.
- **Strategy YAML field set in detail** — see
  [`docs/components/H10-strategy-selector.md`](../components/H10-strategy-selector.md).
- **`HarnessStrategy` runtime semantics** — see H10 doc + ADR-0006.
- **Event log (`ConversationStore`)** — a separate concern; see ADR-0007
  and `docs/data-model/jsonl-v1.md`.

---

## 13. References

- [ADR-0017 — Cogito Runtime configuration model](../adr/0017-cogito-runtime-configuration-model.md) (authoritative on every architectural decision)
- [Sprint 4.5 spec](../superpowers/specs/2026-05-21-sprint-4-5-config-file-design.md) (first implementation)
- [H10 Strategy Selector](../components/H10-strategy-selector.md) (strategy schema, Sprint 6 loader)
- [`CLAUDE.md` §Coding standards](../../CLAUDE.md) (tagged-config factories rule)
- [`AGENTS.md` §Inviolable design principles](../../AGENTS.md) (Brain ↔ Hands ↔ Session boundaries)
- [ADR-0004](../adr/0004-brain-hands-session-boundaries.md) (crate layering)
- [ADR-0007](../adr/0007-event-log-as-cross-language-contract.md) (additive-variant discipline; informs ADR-0017's reserved-section policy)
- GitLab Issue gitlab.sz.sensetime.com/compass/cogito#1 (motivating ask)

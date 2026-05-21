# Sprint 4.5 · Config File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land Sprint 4.5 — file-driven runtime configuration with the `cogito-config` crate (+1 crate), `cogito-model::ProviderConfig` factory, and `cogito-cli` rewiring. Preserves Sprint 2 ENV-only behaviour via a legacy bridge. Closes GitLab Issue #1 sub-needs 1 + 2; sub-need 3 (OpenAI Responses) remains scheduled for Sprint 5.

**Architecture:** New `cogito-config` crate hosts `RuntimeConfig` value types + `ConfigLoader` trait + `EnvConfigLoader` (default features, no third-party parsers) and feature-gated `FileConfigLoader` (`feature = "file"` pulls `toml` + `serde_yaml`). `cogito-model` owns `ProviderConfig` (serde-tagged on `kind`) and the single `build_gateway` factory function — surfaces never match on provider kind. `cogito-cli` runs the layered merge (`CLI > ENV > file > defaults`) and applies post-merge patches for the legacy CLI flags (`--base-url`, `--system`, `--session-root`). Brain layer (`cogito-core::harness`) is **untouched** — it consumes pre-built trait objects exactly as before.

**Tech Stack:** Rust 2024 / MSRV 1.85, `serde` + `serde_yaml` (file feature) + `toml` (file feature), `thiserror`, `async-trait`, `tokio`, `tracing`. TDD via `cargo nextest`. Workspace `just` recipes (`just fmt`, `just fix`, `just test -p <crate>`, `just ci`).

**Spec:** [`docs/superpowers/specs/2026-05-21-sprint-4-5-config-file-design.md`](../specs/2026-05-21-sprint-4-5-config-file-design.md)
**ADR:** [`docs/adr/0017-cogito-runtime-configuration-model.md`](../../adr/0017-cogito-runtime-configuration-model.md)
**Overview:** [`docs/configuration/overview.md`](../../configuration/overview.md)

**Sprint sequencing:** ROADMAP places Sprint 4 (Async Jobs) before this plan. Execute Sprint 4 first; resume here when Sprint 4 closes. Tasks below do not touch `cogito-jobs` or H08 async-path code, so no merge conflicts are expected.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/cogito-config/Cargo.toml` | Create | New crate manifest; `default = []` + `file = ["dep:toml", "dep:serde_yaml"]`. |
| `crates/cogito-config/src/lib.rs` | Create | Module declarations + public re-exports (`RuntimeConfig`, `ConfigLoader`, `EnvConfigLoader`, `merge_layers`; `FileConfigLoader` + `load_runtime_config` behind `feature = "file"`). |
| `crates/cogito-config/src/types.rs` | Create | `RuntimeConfig`, `RuntimeConfigPartial`, `RuntimeSection`, `RuntimeSectionPartial`. |
| `crates/cogito-config/src/loader.rs` | Create | `ConfigLoader` trait + `ConfigError` enum. |
| `crates/cogito-config/src/env.rs` | Create | `EnvConfigLoader` — reads `COGITO_SESSION_ROOT` / `COGITO_DEFAULT_PROVIDER` / `COGITO_DEFAULT_MODEL` / `COGITO_STRATEGIES_DIR`. |
| `crates/cogito-config/src/merge.rs` | Create | `merge_layers(Vec<RuntimeConfigPartial>) -> RuntimeConfigPartial` + `RuntimeConfigPartial::finalize() -> Result<RuntimeConfig, ConfigError>`. |
| `crates/cogito-config/src/interpolate.rs` | Create (feature = "file") | `${VAR}` + `${VAR:-default}` substitution over all string fields in a `toml::Value` tree. |
| `crates/cogito-config/src/file.rs` | Create (feature = "file") | `FileConfigLoader::resolve(--config arg) -> Option<PathBuf>` + `load() -> RuntimeConfigPartial`; runs `interpolate` then `toml::from_str`. |
| `crates/cogito-config/tests/merge.rs` | Create | Precedence, array-replaces-not-merges, finalize defaults, auto-select sole provider, ambiguous-provider error. |
| `crates/cogito-config/tests/env_loader.rs` | Create | Each `COGITO_*` var → correct partial field; empty env → empty partial. |
| `crates/cogito-config/tests/interpolate.rs` | Create (feature = "file") | `${VAR}` expand, `${VAR:-default}` fallback, missing-var error, no-interpolation-for-non-strings. |
| `crates/cogito-config/tests/file_loader.rs` | Create (feature = "file") | Four-step search path order; file-not-found → default partial; `deny_unknown_fields` rejects typos within known sections; reserved sections (`[plugins]`) parse silently. |
| `crates/cogito-model/src/provider_config.rs` | Create | `ProviderConfig` enum + `build_gateway` factory + default constants. |
| `crates/cogito-model/src/lib.rs` | Modify | `pub mod provider_config;` + re-export `ProviderConfig` and `build_gateway`. |
| `crates/cogito-model/tests/provider_config.rs` | Create | Deserialize per kind, `deny_unknown_fields` rejects bad keys, `build_gateway` constructs correct gateway type. |
| `crates/cogito-cli/Cargo.toml` | Modify | Depend on `cogito-config = { workspace = true, features = ["file"] }`. |
| `crates/cogito-cli/src/chat.rs` | Modify | Replace `build_gateway` body with config-driven flow; add `--config` arg; legacy ENV bridge; post-merge CLI patches (`--base-url`, `--system`, `--session-root`). |
| `crates/cogito-cli/tests/config_legacy_env_bridge.rs` | Create | No `cogito.toml`, only `ANTHROPIC_API_KEY` set → CLI builds gateway equivalent to Sprint 2 path. |
| `crates/cogito-cli/tests/config_file_only.rs` | Create | `cogito.toml` declares one Anthropic provider with `${ANTHROPIC_API_KEY}` → loads, gateway constructable. |
| `crates/cogito-cli/tests/config_cli_overrides.rs` | Create | File has `base_url = A`; `--base-url B` → chosen provider's `base_url == B`. |
| `crates/cogito-cli/tests/config_anthropic_compat_third_party.rs` | Create | `cogito.toml` declares `kind = "anthropic"` with non-default `base_url` → gateway uses internal endpoint (verified by `AnthropicConfig::base_url` inspection in a helper). |
| `Cargo.toml` (workspace root) | Modify | Add `crates/cogito-config` to `[workspace] members`; add `serde_yaml = "0.9"` and `humantime` (already present?) to `[workspace.dependencies]`. |
| `docs/components/H10-strategy-selector.md` | Modify | Add 2026-05-21 note: filename = strategy name; `applicable_models` glob dropped (ADR-0017 §9). |
| `ROADMAP.md` | Modify | Insert "Sprint 4.5 · 配置文件 + base_url override" subsection between Sprint 4 and Sprint 5 with `[ ]` checklist matching spec §1.1. |
| `CHANGELOG.md` (if present) | Modify | `### Added — Sprint 4.5 (config-file loading)` block. |

**Commits**: one per task (11 total). Each task leaves `just ci` green.

**Test file lint header (REQUIRED):** Workspace lints set `unwrap_used` / `expect_used` / `panic` to `deny`. Every test file (both inline `#[cfg(test)] mod tests {}` and `tests/*.rs` integration files) must begin with:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

Add this header to every test file in this plan before committing. The plan's code blocks omit it for brevity, but `just ci` will fail without it. Precedent: `crates/cogito-core/tests/resume_chaos.rs`.

**Workspace `unsafe_code = "forbid"` — env mutation pattern (REQUIRED):** The workspace `[workspace.lints.rust]` forbids `unsafe_code`. From Rust edition 2024 onward, `std::env::set_var` and `std::env::remove_var` are `unsafe` — they cannot be called directly from any workspace-member test. The plan's test code below shows `unsafe { std::env::set_var(...) }` for readability of intent; **every such block must be transformed** to use the `temp-env` crate before the corresponding task is committed.

Canonical transformation:

```rust
// SHOWN IN PLAN (compile error under workspace lints):
unsafe { std::env::set_var("KEY", "value"); }
// ... test body ...
unsafe { std::env::remove_var("KEY"); }

// REPLACE WITH (async test body):
temp_env::async_with_vars(
    [("KEY", Some("value"))],
    async {
        // ... test body ...
    },
).await;

// REPLACE WITH (sync test body):
temp_env::with_vars([("KEY", Some("value"))], || {
    // ... test body ...
});
```

To use `None::<&str>` for "unset this var while the closure runs". The `ENV_LOCK: Mutex<()>` pattern in the plan's tests still applies — `temp-env` mutates process env globally and does not serialize across threads.

Cargo additions (one-time, before Task 5 commits):

```toml
# Workspace Cargo.toml [workspace.dependencies]:
temp-env = "0.3"

# crates/cogito-config/Cargo.toml [dev-dependencies]:
temp-env = { workspace = true, features = ["async_closure"] }

# crates/cogito-cli/Cargo.toml [dev-dependencies] (Task 10):
temp-env = { workspace = true, features = ["async_closure"] }
```

Apply this transformation in **every test file** below that mutates env vars (Tasks 5, 7, 8, and the four cogito-cli integration tests in Task 10). The plan's code samples remain readable in the original form so the *intent* of each test is obvious; the `temp-env` rewrite is mechanical.

---

## Task 1: `cogito-model::ProviderConfig` + `build_gateway` factory

**Files:**
- Create: `crates/cogito-model/src/provider_config.rs`
- Modify: `crates/cogito-model/src/lib.rs`
- Create: `crates/cogito-model/tests/provider_config.rs`

This task lands first because `cogito-config::types` will reference `ProviderConfig`. It is self-contained — no other crate consumes it yet.

- [ ] **Step 1.1: Write the failing tests in a new integration test file**

Create `crates/cogito-model/tests/provider_config.rs`:

```rust
//! Tests for `cogito_model::ProviderConfig` deserialization + factory.

use std::sync::Arc;

use cogito_model::{ProviderConfig, build_gateway};
use cogito_protocol::gateway::ModelGateway;

#[test]
fn anthropic_deserializes_with_defaults() {
    let toml_str = r#"
        name = "anthropic-prod"
        kind = "anthropic"
        api_key = "sk-test"
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::Anthropic {
            name,
            api_key,
            base_url,
            anthropic_version,
            timeout_secs,
        } => {
            assert_eq!(name, "anthropic-prod");
            assert_eq!(api_key, "sk-test");
            assert_eq!(base_url, "https://api.anthropic.com");
            assert_eq!(anthropic_version, "2023-06-01");
            assert!(timeout_secs.is_none());
        }
        _ => panic!("expected Anthropic variant"),
    }
}

#[test]
fn anthropic_deserializes_with_overrides() {
    let toml_str = r#"
        name = "anthropic-internal"
        kind = "anthropic"
        api_key = "key"
        base_url = "https://internal.api/anthropic/v1"
        anthropic_version = "2024-01-01"
        timeout_secs = 120
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::Anthropic { base_url, anthropic_version, timeout_secs, .. } => {
            assert_eq!(base_url, "https://internal.api/anthropic/v1");
            assert_eq!(anthropic_version, "2024-01-01");
            assert_eq!(timeout_secs, Some(120));
        }
        _ => panic!("expected Anthropic variant"),
    }
}

#[test]
fn openai_compat_deserializes() {
    let toml_str = r#"
        name = "vllm"
        kind = "openai-compat"
        base_url = "http://vllm:8000/v1"
    "#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("parse");
    match cfg {
        ProviderConfig::OpenAiCompat { name, base_url, api_key, auth_header, auth_scheme, .. } => {
            assert_eq!(name, "vllm");
            assert_eq!(base_url, "http://vllm:8000/v1");
            assert!(api_key.is_none());
            assert_eq!(auth_header, "Authorization");
            assert_eq!(auth_scheme, "Bearer");
        }
        _ => panic!("expected OpenAiCompat variant"),
    }
}

#[test]
fn unknown_kind_errors() {
    let toml_str = r#"
        name = "x"
        kind = "no-such-kind"
    "#;
    let err = toml::from_str::<ProviderConfig>(toml_str).unwrap_err();
    assert!(err.to_string().contains("no-such-kind") || err.to_string().contains("unknown variant"));
}

#[test]
fn unknown_field_errors() {
    let toml_str = r#"
        name = "x"
        kind = "anthropic"
        api_key = "k"
        bogus_field = "boom"
    "#;
    let err = toml::from_str::<ProviderConfig>(toml_str).unwrap_err();
    assert!(err.to_string().contains("bogus_field") || err.to_string().contains("unknown field"));
}

#[test]
fn build_anthropic_gateway() {
    let cfg = ProviderConfig::Anthropic {
        name: "x".into(),
        api_key: "sk".into(),
        base_url: "https://internal.api/anthropic/v1".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    };
    let gw: Arc<dyn ModelGateway> = build_gateway(cfg).expect("build");
    assert_eq!(gw.provider_id(), "anthropic");
}

#[test]
fn build_openai_compat_gateway() {
    let cfg = ProviderConfig::OpenAiCompat {
        name: "x".into(),
        api_key: Some("k".into()),
        base_url: "http://localhost:8000/v1".into(),
        auth_header: "Authorization".into(),
        auth_scheme: "Bearer".into(),
        timeout_secs: None,
    };
    let gw: Arc<dyn ModelGateway> = build_gateway(cfg).expect("build");
    assert_eq!(gw.provider_id(), "openai-compat");
}

#[test]
fn provider_config_name_accessor() {
    let cfg = ProviderConfig::Anthropic {
        name: "anthropic-prod".into(),
        api_key: "k".into(),
        base_url: "https://api.anthropic.com".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    };
    assert_eq!(cfg.name(), "anthropic-prod");
}
```

- [ ] **Step 1.2: Run tests to verify they fail**

Run: `cargo test -p cogito-model --test provider_config`
Expected: compile error — `ProviderConfig` / `build_gateway` not defined.

- [ ] **Step 1.3: Implement `provider_config.rs`**

Create `crates/cogito-model/src/provider_config.rs`:

```rust
//! `ProviderConfig` — declarative description of a `ModelGateway`
//! instance. The single source of truth for the
//! `(connection-endpoint, auth, model-family)` triple that surfaces
//! (`cogito-cli`, future `cogito-tui`, consumer Server) read from
//! configuration files / environment / databases.
//!
//! See ADR-0017 §4 for the schema decision and CLAUDE.md
//! §"Coding standards" for the "tagged-config factories belong in the
//! crate that owns the implementations" rule.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::gateway::{ModelError, ModelGateway};
use serde::{Deserialize, Serialize};

use crate::{AnthropicConfig, AnthropicGateway, OpenAiCompatConfig, OpenAiCompatGateway};

/// Provider configuration: a tagged-union over the gateway kinds
/// `cogito-model` knows how to construct. `kind` is the serde tag.
///
/// Serializes as flat TOML/JSON with `kind` as a discriminator field;
/// kebab-case to match `cogito.toml` conventions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderConfig {
    /// Anthropic Messages API endpoint. `base_url` defaults to the
    /// public endpoint; override for Anthropic-compatible third-party
    /// services.
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
    /// OpenAI-compatible Chat Completions endpoint (vLLM, SGLang, Azure,
    /// internal LLM gateways). Required `base_url`; optional `api_key`
    /// (`None` skips the auth header for unauthenticated deployments).
    OpenAiCompat {
        name: String,
        #[serde(default)]
        api_key: Option<String>,
        base_url: String,
        #[serde(default = "defaults::auth_header")]
        auth_header: String,
        #[serde(default = "defaults::auth_scheme")]
        auth_scheme: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    // OpenAiResponses { ... } lands in Sprint 5 — single-arm addition.
}

impl ProviderConfig {
    /// The configured `name` for this provider entry (used by surfaces
    /// for `--provider <name>` lookup).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic { name, .. } | Self::OpenAiCompat { name, .. } => name,
        }
    }
}

/// Build a concrete `ModelGateway` from a `ProviderConfig`. This is the
/// only place in the workspace that pattern-matches on `kind`; surfaces
/// must call this function rather than reproducing the dispatch table.
///
/// # Errors
///
/// Forwards `ModelError` from the underlying gateway constructors
/// (TLS / client builder failures; rare).
pub fn build_gateway(cfg: ProviderConfig) -> Result<Arc<dyn ModelGateway>, ModelError> {
    match cfg {
        ProviderConfig::Anthropic {
            api_key,
            base_url,
            anthropic_version,
            timeout_secs,
            ..
        } => {
            let mut c = AnthropicConfig::with_api_key(api_key);
            c.base_url = base_url;
            c.anthropic_version = anthropic_version;
            if let Some(s) = timeout_secs {
                c.timeout = Duration::from_secs(s);
            }
            Ok(Arc::new(AnthropicGateway::new(c)?))
        }
        ProviderConfig::OpenAiCompat {
            api_key,
            base_url,
            auth_header,
            auth_scheme,
            timeout_secs,
            ..
        } => {
            let mut c = OpenAiCompatConfig::with_base_url(base_url);
            c.api_key = api_key;
            c.auth_header = auth_header;
            c.auth_scheme = auth_scheme;
            if let Some(s) = timeout_secs {
                c.timeout = Duration::from_secs(s);
            }
            Ok(Arc::new(OpenAiCompatGateway::new(c)?))
        }
    }
}

mod defaults {
    pub(super) fn anthropic_base_url() -> String {
        "https://api.anthropic.com".into()
    }
    pub(super) fn anthropic_version() -> String {
        "2023-06-01".into()
    }
    pub(super) fn auth_header() -> String {
        "Authorization".into()
    }
    pub(super) fn auth_scheme() -> String {
        "Bearer".into()
    }
}
```

- [ ] **Step 1.4: Register module and re-exports**

Modify `crates/cogito-model/src/lib.rs`. Find:

```rust
pub use anthropic::{AnthropicConfig, AnthropicGateway};
pub use openai_compat::{OpenAiCompatConfig, OpenAiCompatGateway};
```

Replace with:

```rust
mod provider_config;

pub use anthropic::{AnthropicConfig, AnthropicGateway};
pub use openai_compat::{OpenAiCompatConfig, OpenAiCompatGateway};
pub use provider_config::{ProviderConfig, build_gateway};
```

Add `mod provider_config;` to the existing module declarations near
the top of the file.

- [ ] **Step 1.5: Add `toml` as a dev-dependency**

Modify `crates/cogito-model/Cargo.toml`. Find the `[dev-dependencies]` section (or add it if missing) and add:

```toml
[dev-dependencies]
toml = { workspace = true }
```

If `toml` is not in `[workspace.dependencies]`, add it at the workspace root `Cargo.toml`:

```toml
[workspace.dependencies]
toml = "0.8"
```

- [ ] **Step 1.6: Run tests to verify they pass**

Run: `cargo test -p cogito-model --test provider_config`
Expected: 9 tests pass.

- [ ] **Step 1.7: Run full crate test + lint**

Run: `just fmt && just fix cogito-model && just test cogito-model`
Expected: green.

- [ ] **Step 1.8: Commit**

```bash
git add crates/cogito-model/ Cargo.toml
git commit -m "$(cat <<'EOF'
feat(model): add ProviderConfig + build_gateway factory (ADR-0017 §4)

Lands the tagged-union ProviderConfig (Anthropic + OpenAiCompat
variants, serde tagged on `kind`) plus the single dispatch function
build_gateway(cfg) -> Arc<dyn ModelGateway>. Per CLAUDE.md
§"Coding standards", this dispatch is owned by the crate that defines
the variants; surfaces never match on kind.

Sprint 5 adds an OpenAiResponses variant by extending the enum + one
match arm — surfaces untouched.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `cogito-config` crate skeleton + workspace registration

**Files:**
- Create: `crates/cogito-config/Cargo.toml`
- Create: `crates/cogito-config/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

Empty crate that builds with both default features and `--features file`. Foundation for the next tasks.

- [ ] **Step 2.1: Create the crate manifest**

Create `crates/cogito-config/Cargo.toml`:

```toml
[package]
name = "cogito-config"
description = "Configuration loading for the cogito Agent Runtime: value types, ConfigLoader trait, ENV / file source loaders, layered partial merge."
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[features]
default = []
file = ["dep:toml", "dep:serde_yaml"]

[dependencies]
cogito-protocol = { workspace = true }
cogito-model    = { workspace = true }

serde       = { workspace = true, features = ["derive"] }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
async-trait = { workspace = true }
tracing     = { workspace = true }

toml       = { workspace = true, optional = true }
serde_yaml = { workspace = true, optional = true }

[dev-dependencies]
tokio    = { workspace = true, features = ["macros", "rt"] }
tempfile = { workspace = true }
temp-env = { workspace = true, features = ["async_closure"] }
toml     = { workspace = true }
```

- [ ] **Step 2.2: Create stub `lib.rs`**

Create `crates/cogito-config/src/lib.rs`:

```rust
//! cogito-config — configuration loading for the cogito Agent Runtime.
//!
//! See [`docs/configuration/overview.md`](../../../docs/configuration/overview.md)
//! for the orientation map and ADR-0017 for the architectural anchor.
//!
//! Default features: value types + `ConfigLoader` trait +
//! `EnvConfigLoader` + layered merge. No file-format parsers.
//!
//! Feature `file`: adds `FileConfigLoader` (TOML + YAML), the
//! `${ENV_VAR}` interpolation pass, and the
//! [`load_runtime_config`] convenience.

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]
```

- [ ] **Step 2.3: Register as workspace member + add `serde_yaml` / `tempfile` / `temp-env`**

Modify root `Cargo.toml`. Find `[workspace] members = [...]` and add `"crates/cogito-config"`. Find `[workspace.dependencies]` and add (if missing):

```toml
serde_yaml = "0.9"
tempfile   = "3"
temp-env   = "0.3"
```

- [ ] **Step 2.4: Verify both feature configurations build**

Run:
```bash
cargo check -p cogito-config
cargo check -p cogito-config --features file
```
Expected: both succeed with no output (empty crate, both feature sets clean).

- [ ] **Step 2.5: Run workspace CI gate**

Run: `just ci`
Expected: green (empty crate is a no-op for tests/clippy).

- [ ] **Step 2.6: Commit**

```bash
git add crates/cogito-config/ Cargo.toml
git commit -m "$(cat <<'EOF'
feat(config): scaffold cogito-config crate (ADR-0017 §5)

Empty crate + workspace member + feature gates (`default = []`,
`file = ["dep:toml", "dep:serde_yaml"]`). No third-party file-format
parsers in default features. Subsequent commits populate
types / loader trait / EnvConfigLoader / merge / FileConfigLoader.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Value types — `RuntimeConfig`, `Partial`, `RuntimeSection`

**Files:**
- Create: `crates/cogito-config/src/types.rs`
- Modify: `crates/cogito-config/src/lib.rs`

Pure data types. Tested via serde roundtrip.

- [ ] **Step 3.1: Write the failing roundtrip test inline in `types.rs`**

Create `crates/cogito-config/src/types.rs`:

```rust
//! Value types for cogito runtime configuration. See ADR-0017 §12 for
//! the locked shape and `docs/configuration/overview.md` §6 for the
//! human-facing reference.

use std::collections::HashMap;
use std::path::PathBuf;

use cogito_model::ProviderConfig;
use cogito_protocol::strategy::HarnessStrategy;
use serde::{Deserialize, Serialize};

/// Finalized configuration value consumed by `RuntimeBuilder`. Always
/// the output of `RuntimeConfigPartial::finalize` after merge.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub runtime: RuntimeSection,
    pub providers: Vec<ProviderConfig>,
    /// Sprint 4.5: always empty. Sprint 5 populates by walking
    /// `runtime.strategies_dir`.
    pub strategies: HashMap<String, HarnessStrategy>,
}

/// Finalized `[runtime]` section. All fields are resolved (no `Option`
/// where a default exists).
#[derive(Debug, Clone)]
pub struct RuntimeSection {
    pub session_root: PathBuf,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub strategies_dir: PathBuf,
}

/// Partial configuration produced by a single `ConfigLoader`. Every
/// field is `Option<T>` so `None` means "do not contribute" during the
/// layered merge.
///
/// The top level intentionally does NOT use
/// `#[serde(deny_unknown_fields)]`: reserved sections (`[plugins]`,
/// `[[subagents]]`) deserialize silently. Inner structs do apply
/// `deny_unknown_fields` to catch typos within a known section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfigPartial {
    pub runtime: Option<RuntimeSectionPartial>,
    pub providers: Option<Vec<ProviderConfig>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSectionPartial {
    pub session_root: Option<PathBuf>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub strategies_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_roundtrips_through_json() {
        let p = RuntimeConfigPartial {
            runtime: Some(RuntimeSectionPartial {
                session_root: Some(PathBuf::from("/tmp/sessions")),
                default_provider: Some("anthropic-prod".into()),
                default_model: Some("claude-opus-4-7".into()),
                strategies_dir: Some(PathBuf::from("./strategies")),
            }),
            providers: Some(vec![]),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: RuntimeConfigPartial = serde_json::from_str(&s).unwrap();
        assert_eq!(back.runtime.as_ref().unwrap().default_provider,
                   p.runtime.as_ref().unwrap().default_provider);
    }

    #[test]
    fn empty_partial_default_is_all_none() {
        let p = RuntimeConfigPartial::default();
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
    }

    #[test]
    fn unknown_top_level_section_does_not_error() {
        // [plugins] is a reserved section; Sprint 4.5 must accept and
        // ignore it.
        let toml_str = r#"
            [[plugins]]
            name = "future-plugin"
            other = "x"
        "#;
        let p: RuntimeConfigPartial = toml::from_str(toml_str).expect("parse");
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
    }

    #[test]
    fn unknown_inner_field_errors() {
        // Inner struct has deny_unknown_fields.
        let toml_str = r#"
            [runtime]
            bogus = "x"
        "#;
        let err = toml::from_str::<RuntimeConfigPartial>(toml_str).unwrap_err();
        assert!(err.to_string().contains("bogus") || err.to_string().contains("unknown"));
    }
}
```

Note: tests reference `toml`; tests live under `#[cfg(test)]` so we
add `toml` as a `dev-dependency` of `cogito-config`. Modify
`crates/cogito-config/Cargo.toml` `[dev-dependencies]` (add to the
existing block):

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }
tempfile = { workspace = true }
toml = { workspace = true }
```

- [ ] **Step 3.2: Wire the module + run tests to verify they fail**

Modify `crates/cogito-config/src/lib.rs` — add after the `#![allow]` line:

```rust
pub mod types;

pub use types::{
    RuntimeConfig, RuntimeConfigPartial, RuntimeSection, RuntimeSectionPartial,
};
```

Run: `cargo test -p cogito-config --lib`
Expected: 4 tests pass (types.rs is already complete in step 3.1).

If tests reference items not yet in scope (unlikely given step 3.1 is
the full implementation), fix the imports and re-run.

- [ ] **Step 3.3: Run lint + fmt**

Run: `just fmt && just fix cogito-config`
Expected: clean.

- [ ] **Step 3.4: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): add RuntimeConfig + Partial value types (ADR-0017 §12)

Finalized RuntimeConfig + per-layer RuntimeConfigPartial + serde
roundtrip and reserved-section tests. Top-level lacks
`deny_unknown_fields` so future `[plugins]` / `[[subagents]]`
sections deserialize silently; inner structs catch typos.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `ConfigLoader` trait + `ConfigError` enum

**Files:**
- Create: `crates/cogito-config/src/loader.rs`
- Modify: `crates/cogito-config/src/lib.rs`

- [ ] **Step 4.1: Write `loader.rs` (tests inline)**

Create `crates/cogito-config/src/loader.rs`:

```rust
//! `ConfigLoader` trait and `ConfigError`. Every source (file, env, db,
//! custom) implements `ConfigLoader::load -> RuntimeConfigPartial`.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::types::RuntimeConfigPartial;

/// One source of configuration. Sources do not see each other; merge
/// happens externally via `merge_layers`.
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError>;
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingEnv(String),

    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("invalid YAML in {path}: {source}")]
    YamlParse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("interpolation error: {0}")]
    Interpolation(String),

    #[error("validation failed: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyLoader;

    #[async_trait]
    impl ConfigLoader for EmptyLoader {
        async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
            Ok(RuntimeConfigPartial::default())
        }
    }

    #[tokio::test]
    async fn empty_loader_returns_default() {
        let l = EmptyLoader;
        let p = l.load().await.expect("ok");
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
    }

    #[test]
    fn config_error_messages_are_informative() {
        let err = ConfigError::MissingEnv("ANTHROPIC_API_KEY".into());
        assert!(err.to_string().contains("ANTHROPIC_API_KEY"));

        let err = ConfigError::Validation("no provider declared".into());
        assert!(err.to_string().contains("no provider declared"));
    }
}
```

Note `ConfigError` mentions `toml::de::Error` and `serde_yaml::Error`
unconditionally. The `cogito-config` crate **does not** carry `toml`
+ `serde_yaml` in default features (only behind `feature = "file"`).
Resolve this by adding the `toml` + `serde_yaml` types behind a
feature gate inside the error enum:

Replace the relevant arms with feature-gated cfg:

```rust
    #[cfg(feature = "file")]
    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[cfg(feature = "file")]
    #[error("invalid YAML in {path}: {source}")]
    YamlParse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
```

Default-features build keeps the enum smaller; the `file` feature
adds the parse-error variants.

- [ ] **Step 4.2: Register module in `lib.rs` and re-export**

Modify `crates/cogito-config/src/lib.rs`. Add after `pub mod types;`:

```rust
pub mod loader;

pub use loader::{ConfigError, ConfigLoader};
```

- [ ] **Step 4.3: Run tests**

Run:
```bash
cargo test -p cogito-config --lib
cargo test -p cogito-config --lib --features file
```
Expected: tests pass under both feature configurations.

- [ ] **Step 4.4: Lint**

Run: `just fmt && just fix cogito-config`

- [ ] **Step 4.5: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): add ConfigLoader trait + ConfigError enum

Async trait that every source (env, file, db, custom) implements,
returning a RuntimeConfigPartial. ConfigError variants are
non-exhaustive; TOML/YAML parse variants are feature-gated behind
`file` so default build stays parser-free.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `EnvConfigLoader` (default feature, no third-party deps)

**Files:**
- Create: `crates/cogito-config/src/env.rs`
- Create: `crates/cogito-config/tests/env_loader.rs`
- Modify: `crates/cogito-config/src/lib.rs`

Reads `COGITO_*` env vars for structured `RuntimeConfig` fields. Does
NOT read legacy `ANTHROPIC_API_KEY` — that is the cogito-cli legacy
bridge's job (Task 9).

- [ ] **Step 5.1: Write the failing integration test**

Create `crates/cogito-config/tests/env_loader.rs`:

```rust
//! Integration tests for `EnvConfigLoader`. Uses serial-test pattern
//! manually via a global mutex because tests share the process env.

use std::sync::Mutex;

use cogito_config::{ConfigLoader, EnvConfigLoader};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn unset_all() {
    for k in [
        "COGITO_SESSION_ROOT",
        "COGITO_DEFAULT_PROVIDER",
        "COGITO_DEFAULT_MODEL",
        "COGITO_STRATEGIES_DIR",
    ] {
        // SAFETY: ENV_LOCK guards concurrent access; std env mutation
        // is `unsafe` from edition 2024 onward.
        unsafe { std::env::remove_var(k); }
    }
}

#[tokio::test]
async fn empty_env_yields_empty_partial() {
    let _g = ENV_LOCK.lock().unwrap();
    unset_all();

    let loader = EnvConfigLoader::default();
    let p = loader.load().await.expect("ok");
    assert!(p.runtime.is_none());
    assert!(p.providers.is_none());
}

#[tokio::test]
async fn cogito_session_root_sets_field() {
    let _g = ENV_LOCK.lock().unwrap();
    unset_all();
    unsafe { std::env::set_var("COGITO_SESSION_ROOT", "/tmp/cogito-sess"); }

    let p = EnvConfigLoader::default().load().await.expect("ok");
    let rt = p.runtime.expect("runtime present");
    assert_eq!(rt.session_root.as_deref(), Some(std::path::Path::new("/tmp/cogito-sess")));
    assert!(rt.default_provider.is_none());
    assert!(rt.default_model.is_none());
    assert!(rt.strategies_dir.is_none());
}

#[tokio::test]
async fn all_cogito_vars_set() {
    let _g = ENV_LOCK.lock().unwrap();
    unset_all();
    unsafe {
        std::env::set_var("COGITO_SESSION_ROOT", "./s");
        std::env::set_var("COGITO_DEFAULT_PROVIDER", "anthropic-prod");
        std::env::set_var("COGITO_DEFAULT_MODEL", "claude-opus-4-7");
        std::env::set_var("COGITO_STRATEGIES_DIR", "./strats");
    }

    let p = EnvConfigLoader::default().load().await.expect("ok");
    let rt = p.runtime.expect("runtime present");
    assert_eq!(rt.session_root.as_deref(), Some(std::path::Path::new("./s")));
    assert_eq!(rt.default_provider.as_deref(), Some("anthropic-prod"));
    assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(rt.strategies_dir.as_deref(), Some(std::path::Path::new("./strats")));
}
```

- [ ] **Step 5.2: Run test to verify failure**

Run: `cargo test -p cogito-config --test env_loader`
Expected: compile error — `EnvConfigLoader` not in scope.

- [ ] **Step 5.3: Implement `env.rs`**

Create `crates/cogito-config/src/env.rs`:

```rust
//! `EnvConfigLoader` — reads `COGITO_*` environment variables and
//! produces a `RuntimeConfigPartial`. Default-features available
//! (std-only; no third-party deps).
//!
//! Legacy variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
//! `OPENAI_BASE_URL`) are NOT handled here — they belong to the
//! `cogito-cli` legacy bridge that synthesizes a `default` provider
//! when `cogito.toml` is absent. See `docs/configuration/overview.md`
//! §10.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::{RuntimeConfigPartial, RuntimeSectionPartial};

/// Reads `COGITO_*` env vars synchronously inside an async `load`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvConfigLoader;

#[async_trait]
impl ConfigLoader for EnvConfigLoader {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
        let session_root = read_path("COGITO_SESSION_ROOT");
        let default_provider = read_string("COGITO_DEFAULT_PROVIDER");
        let default_model = read_string("COGITO_DEFAULT_MODEL");
        let strategies_dir = read_path("COGITO_STRATEGIES_DIR");

        let any_runtime_field = session_root.is_some()
            || default_provider.is_some()
            || default_model.is_some()
            || strategies_dir.is_some();

        let partial = RuntimeConfigPartial {
            runtime: any_runtime_field.then(|| RuntimeSectionPartial {
                session_root,
                default_provider,
                default_model,
                strategies_dir,
            }),
            providers: None,
        };
        Ok(partial)
    }
}

fn read_string(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn read_path(key: &str) -> Option<PathBuf> {
    read_string(key).map(PathBuf::from)
}
```

- [ ] **Step 5.4: Register module + re-export**

Modify `crates/cogito-config/src/lib.rs`. Add after `pub mod loader;`:

```rust
pub mod env;

pub use env::EnvConfigLoader;
```

- [ ] **Step 5.5: Run tests**

Run: `cargo test -p cogito-config --test env_loader -- --test-threads=1`
Expected: 3 tests pass (single-threaded because tests mutate the process env).

Run: `cargo test -p cogito-config`
Expected: all tests pass.

- [ ] **Step 5.6: Lint**

Run: `just fmt && just fix cogito-config`

- [ ] **Step 5.7: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): add EnvConfigLoader for COGITO_* vars

Reads COGITO_SESSION_ROOT / COGITO_DEFAULT_PROVIDER /
COGITO_DEFAULT_MODEL / COGITO_STRATEGIES_DIR into a
RuntimeConfigPartial. Default features, std-only (no third-party
deps). Legacy variables (ANTHROPIC_API_KEY etc.) are not handled
here; they belong to the cogito-cli legacy bridge added later.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Layered merge + `finalize`

**Files:**
- Create: `crates/cogito-config/src/merge.rs`
- Create: `crates/cogito-config/tests/merge.rs`
- Modify: `crates/cogito-config/src/lib.rs`

Implements `merge_layers(Vec<RuntimeConfigPartial>) -> RuntimeConfigPartial` (later layers' `Some(_)` override earlier; arrays replace wholesale) and `RuntimeConfigPartial::finalize() -> Result<RuntimeConfig, ConfigError>` (apply defaults, auto-select sole provider).

- [ ] **Step 6.1: Write the failing integration tests**

Create `crates/cogito-config/tests/merge.rs`:

```rust
//! Tests for layered partial merge and `finalize` defaults / validation.

use std::path::PathBuf;

use cogito_config::{
    merge_layers, RuntimeConfigPartial, RuntimeSectionPartial,
};
use cogito_model::ProviderConfig;

fn partial_with_model(model: &str) -> RuntimeConfigPartial {
    RuntimeConfigPartial {
        runtime: Some(RuntimeSectionPartial {
            default_model: Some(model.into()),
            ..Default::default()
        }),
        providers: None,
    }
}

fn anthropic_provider(name: &str) -> ProviderConfig {
    ProviderConfig::Anthropic {
        name: name.into(),
        api_key: "k".into(),
        base_url: "https://api.anthropic.com".into(),
        anthropic_version: "2023-06-01".into(),
        timeout_secs: None,
    }
}

#[test]
fn later_layer_overrides_earlier() {
    let merged = merge_layers(vec![
        partial_with_model("claude-sonnet-4-6"),
        partial_with_model("claude-opus-4-7"),
    ]);
    let rt = merged.runtime.unwrap();
    assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn later_some_does_not_overwrite_with_none() {
    let merged = merge_layers(vec![
        partial_with_model("claude-opus-4-7"),
        RuntimeConfigPartial::default(),
    ]);
    let rt = merged.runtime.unwrap();
    assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn providers_array_replaces_wholesale() {
    let layer_a = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
    };
    let layer_b = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("c")]),
    };
    let merged = merge_layers(vec![layer_a, layer_b]);
    assert_eq!(merged.providers.as_ref().unwrap().len(), 1);
    assert_eq!(merged.providers.as_ref().unwrap()[0].name(), "c");
}

#[test]
fn finalize_fills_defaults() {
    let partial = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("only")]),
    };
    let cfg = partial.finalize().expect("ok");
    assert_eq!(cfg.runtime.session_root, PathBuf::from("./sessions"));
    assert_eq!(cfg.runtime.strategies_dir, PathBuf::from("./strategies"));
    // Auto-select rule: one provider, no explicit default_provider.
    assert_eq!(cfg.runtime.default_provider.as_deref(), Some("only"));
    assert!(cfg.runtime.default_model.is_none());
}

#[test]
fn finalize_preserves_explicit_default_provider() {
    let partial = RuntimeConfigPartial {
        runtime: Some(RuntimeSectionPartial {
            default_provider: Some("a".into()),
            ..Default::default()
        }),
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
    };
    let cfg = partial.finalize().expect("ok");
    assert_eq!(cfg.runtime.default_provider.as_deref(), Some("a"));
}

#[test]
fn finalize_ambiguous_provider_errors() {
    let partial = RuntimeConfigPartial {
        runtime: None,
        providers: Some(vec![anthropic_provider("a"), anthropic_provider("b")]),
    };
    let err = partial.finalize().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("multiple providers") || msg.contains("default_provider"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn finalize_empty_providers_yields_empty_runtime() {
    // Sprint 4.5 legacy bridge must run AFTER finalize when this happens.
    // finalize itself does NOT error on empty providers — it leaves them
    // empty so the caller (cogito-cli) can synthesize a default.
    let partial = RuntimeConfigPartial::default();
    let cfg = partial.finalize().expect("ok");
    assert!(cfg.providers.is_empty());
    assert!(cfg.runtime.default_provider.is_none());
}
```

- [ ] **Step 6.2: Verify the tests fail**

Run: `cargo test -p cogito-config --test merge`
Expected: compile error — `merge_layers`, `finalize` not in scope.

- [ ] **Step 6.3: Implement `merge.rs`**

Create `crates/cogito-config/src/merge.rs`:

```rust
//! Layered partial merge + `finalize` (apply defaults, run minimal
//! validation). See ADR-0017 §3 for precedence semantics.

use std::path::PathBuf;

use crate::loader::ConfigError;
use crate::types::{
    RuntimeConfig, RuntimeConfigPartial, RuntimeSection, RuntimeSectionPartial,
};

/// Merge a stack of partials in order. The first element is the lowest
/// precedence (e.g. `defaults`); the last element is the highest
/// (e.g. CLI). Later layers' `Some(_)` overrides earlier layers'.
///
/// Arrays (`providers`) replace wholesale — no element-wise merge.
#[must_use]
pub fn merge_layers(layers: Vec<RuntimeConfigPartial>) -> RuntimeConfigPartial {
    layers
        .into_iter()
        .fold(RuntimeConfigPartial::default(), merge_into)
}

fn merge_into(
    mut acc: RuntimeConfigPartial,
    next: RuntimeConfigPartial,
) -> RuntimeConfigPartial {
    if let Some(rt_next) = next.runtime {
        acc.runtime = Some(merge_runtime(acc.runtime.unwrap_or_default(), rt_next));
    }
    if let Some(providers_next) = next.providers {
        acc.providers = Some(providers_next);
    }
    acc
}

fn merge_runtime(
    mut acc: RuntimeSectionPartial,
    next: RuntimeSectionPartial,
) -> RuntimeSectionPartial {
    if next.session_root.is_some() {
        acc.session_root = next.session_root;
    }
    if next.default_provider.is_some() {
        acc.default_provider = next.default_provider;
    }
    if next.default_model.is_some() {
        acc.default_model = next.default_model;
    }
    if next.strategies_dir.is_some() {
        acc.strategies_dir = next.strategies_dir;
    }
    acc
}

impl RuntimeConfigPartial {
    /// Fill defaults and apply minimal validation:
    ///
    /// - `runtime.session_root`   → `"./sessions"`
    /// - `runtime.strategies_dir` → `"./strategies"`
    /// - `runtime.default_provider`: if absent AND exactly one provider
    ///   declared, auto-select its name; if absent AND multiple
    ///   providers declared, return `ConfigError::Validation`.
    /// - `runtime.default_model`: kept `None` if absent; surfaces
    ///   may supply via CLI.
    ///
    /// Empty `providers` is **not** an error here — Sprint 4.5's
    /// `cogito-cli` legacy bridge synthesizes a `default` provider
    /// in that case before constructing the gateway.
    pub fn finalize(self) -> Result<RuntimeConfig, ConfigError> {
        let rt = self.runtime.unwrap_or_default();
        let providers = self.providers.unwrap_or_default();

        let mut default_provider = rt.default_provider;
        if default_provider.is_none() && providers.len() >= 2 {
            return Err(ConfigError::Validation(format!(
                "multiple providers declared ({}) but no `default_provider` selected; \
                 set runtime.default_provider in cogito.toml or pass --provider on the CLI",
                providers.len()
            )));
        }
        if default_provider.is_none() && providers.len() == 1 {
            default_provider = Some(providers[0].name().to_string());
        }

        Ok(RuntimeConfig {
            runtime: RuntimeSection {
                session_root: rt.session_root.unwrap_or_else(|| PathBuf::from("./sessions")),
                default_provider,
                default_model: rt.default_model,
                strategies_dir: rt
                    .strategies_dir
                    .unwrap_or_else(|| PathBuf::from("./strategies")),
            },
            providers,
            strategies: std::collections::HashMap::new(),
        })
    }
}
```

- [ ] **Step 6.4: Register + re-export**

Modify `crates/cogito-config/src/lib.rs`:

```rust
pub mod merge;

pub use merge::merge_layers;
```

- [ ] **Step 6.5: Run tests**

Run: `cargo test -p cogito-config`
Expected: all unit + integration tests green.

- [ ] **Step 6.6: Lint**

Run: `just fmt && just fix cogito-config`

- [ ] **Step 6.7: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): add merge_layers + finalize (ADR-0017 §3)

Layered partial merge (`CLI > ENV > file > defaults`); arrays replace
wholesale per ADR. finalize applies defaults (session_root /
strategies_dir) and validates default_provider against the providers
array (auto-select if one, error if many + none chosen). Empty
providers is allowed — cogito-cli's legacy ENV bridge handles that
case post-finalize.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `${VAR}` interpolation (feature = file)

**Files:**
- Create: `crates/cogito-config/src/interpolate.rs`
- Create: `crates/cogito-config/tests/interpolate.rs`
- Modify: `crates/cogito-config/src/lib.rs`

Substitutes `${VAR}` and `${VAR:-default}` in every string value of a
parsed `toml::Value` tree. Runs after TOML parse, before
deserialization into `RuntimeConfigPartial`.

- [ ] **Step 7.1: Write the failing tests**

Create `crates/cogito-config/tests/interpolate.rs`:

```rust
//! Tests for ${VAR} / ${VAR:-default} interpolation. Single-threaded
//! because they share the process env.

#![cfg(feature = "file")]

use std::sync::Mutex;

use cogito_config::interpolate::interpolate_value;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn expand_simple_var() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("COGITO_TEST_VAR", "hello"); }

    let raw: toml::Value = toml::from_str(r#"key = "prefix-${COGITO_TEST_VAR}-suffix""#).unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    assert_eq!(expanded["key"].as_str(), Some("prefix-hello-suffix"));

    unsafe { std::env::remove_var("COGITO_TEST_VAR"); }
}

#[test]
fn default_used_when_unset() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("COGITO_NO_SUCH_VAR"); }

    let raw: toml::Value = toml::from_str(
        r#"url = "${COGITO_NO_SUCH_VAR:-https://api.example.com}""#,
    )
    .unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    assert_eq!(expanded["url"].as_str(), Some("https://api.example.com"));
}

#[test]
fn missing_var_no_default_errors() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("COGITO_REQUIRED"); }

    let raw: toml::Value = toml::from_str(r#"k = "${COGITO_REQUIRED}""#).unwrap();
    let err = interpolate_value(raw).unwrap_err();
    assert!(err.to_string().contains("COGITO_REQUIRED"));
}

#[test]
fn no_substitution_for_non_strings() {
    let _g = ENV_LOCK.lock().unwrap();
    let raw: toml::Value = toml::from_str("n = 42\nb = true").unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    assert_eq!(expanded["n"].as_integer(), Some(42));
    assert_eq!(expanded["b"].as_bool(), Some(true));
}

#[test]
fn nested_tables_and_arrays() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("COGITO_KEY_A", "from-env"); }

    let raw: toml::Value = toml::from_str(
        r#"
            [section]
            inner = "${COGITO_KEY_A}"
            [[items]]
            field = "literal"
            [[items]]
            field = "${COGITO_KEY_A:-fallback}"
        "#,
    )
    .unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    assert_eq!(expanded["section"]["inner"].as_str(), Some("from-env"));
    assert_eq!(expanded["items"][0]["field"].as_str(), Some("literal"));
    assert_eq!(expanded["items"][1]["field"].as_str(), Some("from-env"));

    unsafe { std::env::remove_var("COGITO_KEY_A"); }
}

#[test]
fn literal_dollar_passes_through() {
    let raw: toml::Value = toml::from_str(r#"k = "$5 cup of coffee""#).unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    // Lone `$` not followed by `{` is left untouched.
    assert_eq!(expanded["k"].as_str(), Some("$5 cup of coffee"));
}
```

- [ ] **Step 7.2: Verify failure**

Run: `cargo test -p cogito-config --test interpolate --features file`
Expected: compile error — `interpolate_value` not in scope.

- [ ] **Step 7.3: Implement `interpolate.rs`**

Create `crates/cogito-config/src/interpolate.rs`:

```rust
//! `${VAR}` and `${VAR:-default}` substitution over `toml::Value`
//! trees. See ADR-0017 §6.

use crate::loader::ConfigError;

/// Walk a `toml::Value` and interpolate every string in place. Numbers,
/// booleans, dates, etc. are returned unchanged.
///
/// # Errors
///
/// Returns `ConfigError::MissingEnv` if a `${VAR}` reference (without a
/// `:-default` fallback) names an environment variable that is unset
/// or empty.
pub fn interpolate_value(value: toml::Value) -> Result<toml::Value, ConfigError> {
    match value {
        toml::Value::String(s) => Ok(toml::Value::String(interpolate_str(&s)?)),
        toml::Value::Array(items) => {
            let out: Result<Vec<_>, _> = items.into_iter().map(interpolate_value).collect();
            Ok(toml::Value::Array(out?))
        }
        toml::Value::Table(t) => {
            let mut out = toml::map::Map::new();
            for (k, v) in t {
                out.insert(k, interpolate_value(v)?);
            }
            Ok(toml::Value::Table(out))
        }
        other => Ok(other),
    }
}

fn interpolate_str(s: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find closing `}`.
            let start = i + 2;
            let Some(end_rel) = s[start..].find('}') else {
                // Unclosed `${`; treat as literal.
                out.push('$');
                i += 1;
                continue;
            };
            let end = start + end_rel;
            let body = &s[start..end];
            let (var, default) = match body.find(":-") {
                Some(p) => (&body[..p], Some(&body[p + 2..])),
                None => (body, None),
            };
            let resolved = match std::env::var(var) {
                Ok(v) if !v.is_empty() => v,
                _ => match default {
                    Some(d) => d.to_string(),
                    None => {
                        return Err(ConfigError::MissingEnv(var.to_string()));
                    }
                },
            };
            out.push_str(&resolved);
            i = end + 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_var_uses_default() {
        // SAFETY: single-threaded test access; nothing else mutates this var.
        unsafe { std::env::set_var("COGITO_EMPTY_VAR", ""); }
        let s = interpolate_str("${COGITO_EMPTY_VAR:-fallback}").unwrap();
        assert_eq!(s, "fallback");
        unsafe { std::env::remove_var("COGITO_EMPTY_VAR"); }
    }

    #[test]
    fn unclosed_brace_passes_through() {
        let s = interpolate_str("${VAR no closing").unwrap();
        assert_eq!(s, "${VAR no closing");
    }
}
```

- [ ] **Step 7.4: Register module (feature-gated)**

Modify `crates/cogito-config/src/lib.rs`:

```rust
#[cfg(feature = "file")]
pub mod interpolate;
```

- [ ] **Step 7.5: Run tests**

Run: `cargo test -p cogito-config --features file --test interpolate -- --test-threads=1`
Expected: 6 integration tests pass.

Run: `cargo test -p cogito-config --features file`
Expected: all tests green.

- [ ] **Step 7.6: Lint**

Run: `just fmt && just fix cogito-config`

- [ ] **Step 7.7: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): add ${VAR} / ${VAR:-default} interpolation (ADR-0017 §6)

interpolate_value walks a toml::Value tree and substitutes string
fields against the process environment. Missing required var →
ConfigError::MissingEnv; missing var with `:-default` → uses the
default. Lone `$` and unclosed `${` pass through untouched. Feature
gated behind `file`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `FileConfigLoader` + search path + `load_runtime_config`

**Files:**
- Create: `crates/cogito-config/src/file.rs`
- Create: `crates/cogito-config/tests/file_loader.rs`
- Modify: `crates/cogito-config/src/lib.rs`

`FileConfigLoader::resolve(--config arg)` walks the four-step search
path (ADR-0017 §7). `load()` reads the file, interpolates, deserializes
into `RuntimeConfigPartial`. `load_runtime_config` is the
end-to-end convenience that chains File + Env loaders into a finalized
`RuntimeConfig`.

- [ ] **Step 8.1: Write the failing tests**

Create `crates/cogito-config/tests/file_loader.rs`:

```rust
//! Tests for FileConfigLoader: search path, file-not-found, deny_unknown_fields,
//! reserved-section tolerance.

#![cfg(feature = "file")]

use std::sync::Mutex;

use cogito_config::{ConfigLoader, FileConfigLoader};
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_env() {
    for k in ["COGITO_CONFIG", "XDG_CONFIG_HOME"] {
        unsafe { std::env::remove_var(k); }
    }
}

#[tokio::test]
async fn no_path_no_env_no_local_returns_empty_partial() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()); }
    // Run inside a tempdir to also avoid picking up the workspace's
    // ./cogito.toml (if any).
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
    let partial = loader.load().await.expect("ok");
    assert!(partial.runtime.is_none());
    assert!(partial.providers.is_none());

    std::env::set_current_dir(prev).unwrap();
    clear_env();
}

#[tokio::test]
async fn explicit_path_wins() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    let path = dir.path().join("custom.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "/from/explicit"
        "#,
    )
    .unwrap();

    let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
    let partial = loader.load().await.expect("ok");
    let rt = partial.runtime.expect("runtime");
    assert_eq!(rt.session_root.as_deref(), Some(std::path::Path::new("/from/explicit")));
}

#[tokio::test]
async fn cogito_config_env_var_used() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    let path = dir.path().join("env.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_model = "from-env-var-path"
        "#,
    )
    .unwrap();
    unsafe { std::env::set_var("COGITO_CONFIG", &path); }

    let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
    let partial = loader.load().await.expect("ok");
    assert_eq!(
        partial.runtime.unwrap().default_model.as_deref(),
        Some("from-env-var-path")
    );
    clear_env();
}

#[tokio::test]
async fn local_cogito_toml_used_when_no_explicit_or_env() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("cogito.toml"),
        r#"
            [runtime]
            default_provider = "local"
        "#,
    )
    .unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
    let partial = loader.load().await.expect("ok");
    assert_eq!(
        partial.runtime.unwrap().default_provider.as_deref(),
        Some("local")
    );

    std::env::set_current_dir(prev).unwrap();
    clear_env();
}

#[tokio::test]
async fn reserved_top_level_section_does_not_error() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "./s"

            [[plugins]]
            name = "future"
        "#,
    )
    .unwrap();
    let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
    let partial = loader.load().await.expect("ok");
    assert_eq!(
        partial.runtime.unwrap().session_root.as_deref(),
        Some(std::path::Path::new("./s"))
    );
}

#[tokio::test]
async fn unknown_inner_field_errors() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env();

    let dir = tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            bogus_field = "x"
        "#,
    )
    .unwrap();
    let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
    let err = loader.load().await.unwrap_err();
    assert!(err.to_string().contains("bogus_field"));
}
```

- [ ] **Step 8.2: Verify failure**

Run: `cargo test -p cogito-config --features file --test file_loader`
Expected: compile error — `FileConfigLoader` not in scope.

- [ ] **Step 8.3: Implement `file.rs`**

Create `crates/cogito-config/src/file.rs`:

```rust
//! `FileConfigLoader` — reads `cogito.toml` from a resolved path,
//! interpolates `${ENV_VAR}` placeholders, and deserializes into a
//! `RuntimeConfigPartial`. See ADR-0017 §7 for the search-path
//! decision.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::interpolate::interpolate_value;
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::RuntimeConfigPartial;

/// Resolves a `cogito.toml` path per the four-step search rule and
/// reads it on `load`. If no path is found, `load` returns
/// `RuntimeConfigPartial::default()`.
#[derive(Debug, Clone, Default)]
pub struct FileConfigLoader {
    resolved_path: Option<PathBuf>,
}

impl FileConfigLoader {
    /// Resolve the file path per the search order:
    ///
    /// 1. `--config <path>` arg (parameter)
    /// 2. `$COGITO_CONFIG`
    /// 3. `./cogito.toml`
    /// 4. `$XDG_CONFIG_HOME/cogito/config.toml` (if `XDG_CONFIG_HOME` is set)
    /// 5. No file → loader returns empty partial on load.
    ///
    /// Returns the loader (with the resolved path, if any). Path
    /// resolution itself never fails; load-time errors surface from
    /// `load()`.
    pub fn resolve<P: AsRef<Path>>(arg: Option<P>) -> Result<Self, ConfigError> {
        if let Some(p) = arg {
            return Ok(Self {
                resolved_path: Some(p.as_ref().to_path_buf()),
            });
        }
        if let Ok(v) = std::env::var("COGITO_CONFIG") {
            if !v.is_empty() {
                return Ok(Self {
                    resolved_path: Some(PathBuf::from(v)),
                });
            }
        }
        let local = PathBuf::from("./cogito.toml");
        if local.is_file() {
            return Ok(Self {
                resolved_path: Some(local),
            });
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let candidate = PathBuf::from(xdg).join("cogito").join("config.toml");
            if candidate.is_file() {
                return Ok(Self {
                    resolved_path: Some(candidate),
                });
            }
        }
        Ok(Self { resolved_path: None })
    }

    /// Path the loader will read on `load`, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.resolved_path.as_deref()
    }
}

#[async_trait]
impl ConfigLoader for FileConfigLoader {
    async fn load(&self) -> Result<RuntimeConfigPartial, ConfigError> {
        let Some(path) = &self.resolved_path else {
            tracing::debug!(target: "cogito::config", "no cogito.toml found; empty partial");
            return Ok(RuntimeConfigPartial::default());
        };
        let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.clone(),
            source: e,
        })?;
        let parsed: toml::Value =
            toml::from_str(&raw).map_err(|e| ConfigError::TomlParse {
                path: path.clone(),
                source: e,
            })?;
        let interpolated = interpolate_value(parsed)?;
        let partial: RuntimeConfigPartial = interpolated
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::TomlParse {
                path: path.clone(),
                source: e,
            })?;
        tracing::debug!(
            target: "cogito::config",
            path = %path.display(),
            "loaded cogito.toml"
        );
        Ok(partial)
    }
}

/// End-to-end convenience: load File + Env layers, merge, finalize.
/// Available behind `feature = "file"`.
///
/// CLI args are not part of this convenience — the surface (`cogito-cli`)
/// applies its own CLI patch as the highest-precedence layer after this
/// call returns.
pub async fn load_runtime_config<P: AsRef<Path>>(
    config_path: Option<P>,
) -> Result<crate::RuntimeConfig, ConfigError> {
    let file = FileConfigLoader::resolve(config_path)?;
    let env = crate::env::EnvConfigLoader::default();
    let layers = vec![file.load().await?, env.load().await?];
    let merged = crate::merge::merge_layers(layers);
    merged.finalize()
}
```

- [ ] **Step 8.4: Register feature-gated module + re-exports**

Modify `crates/cogito-config/src/lib.rs`:

```rust
#[cfg(feature = "file")]
pub mod file;

#[cfg(feature = "file")]
pub use file::{FileConfigLoader, load_runtime_config};
```

- [ ] **Step 8.5: Run tests**

Run: `cargo test -p cogito-config --features file -- --test-threads=1`
Expected: all green.

- [ ] **Step 8.6: Verify default-features build still clean**

Run: `cargo test -p cogito-config`
Expected: tests that depend on `feature = "file"` are excluded via
`#![cfg(feature = "file")]`; remaining tests pass.

- [ ] **Step 8.7: Lint**

Run: `just fmt && just fix cogito-config`

- [ ] **Step 8.8: Commit**

```bash
git add crates/cogito-config/
git commit -m "$(cat <<'EOF'
feat(config): FileConfigLoader + search path + load_runtime_config (ADR-0017 §7)

Reads cogito.toml from the four-step search path (--config arg >
COGITO_CONFIG > ./cogito.toml > $XDG_CONFIG_HOME/cogito/config.toml,
first-hit-wins), interpolates ${VAR} placeholders, deserializes into
RuntimeConfigPartial. load_runtime_config is the convenience that
chains File + Env loaders, merges, and finalizes; CLI patches stay
in cogito-cli. Feature-gated behind `file`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `cogito-cli` refactor — `--config`, config-driven gateway, legacy bridge

**Files:**
- Modify: `crates/cogito-cli/Cargo.toml`
- Modify: `crates/cogito-cli/src/chat.rs`

Replaces the inline `build_gateway` body with the new config-driven flow. Adds `--config` arg. Implements the Sprint 2 → 4.5 legacy bridge: when no `cogito.toml` is found AND `providers` is empty, synthesize a `default` provider from legacy ENV (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` + `OPENAI_BASE_URL`).

Note: this task does **not** add tests yet — Task 10 adds the four integration tests. This separation lets the refactor compile + manual-test first, then formalize behavior contracts.

> **Implementation hint:** Task 10 will move helpers into a `chat_config.rs` module and add a `lib.rs` target so integration tests can reach them. If you are implementing fresh and have the full picture, you may put helpers directly into `chat_config.rs` here (Step 9.2 below shows them inside `chat.rs` so the diff against the old code stays readable; either layout passes the smoke test). The end state required by Task 10 is: `chat_config.rs` owns the helpers, `chat.rs` calls into them, and a `[lib]` target exposes `chat_config` as `cogito_cli::chat_config`.

- [ ] **Step 9.1: Add the `cogito-config` dependency**

Modify `crates/cogito-cli/Cargo.toml`. Find the `[dependencies]` block and add:

```toml
cogito-config = { workspace = true, features = ["file"] }
```

- [ ] **Step 9.2: Rewrite `chat.rs`**

Modify `crates/cogito-cli/src/chat.rs`:

Replace the current `build_gateway` function and the top of `run` with the new flow. Here is the full new content of `chat.rs` from `use` block down to `pub async fn run`:

```rust
//! `cogito chat` — interactive REPL subcommand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use cogito_config::{
    EnvConfigLoader, FileConfigLoader, RuntimeConfig, RuntimeConfigPartial,
    RuntimeSectionPartial, merge_layers, ConfigLoader,
};
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_model::{ProviderConfig, build_gateway};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use tokio::io::{self, AsyncBufReadExt, BufReader};

/// Arguments for the `chat` subcommand.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Path to a `cogito.toml`. Highest precedence in the search path.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Model identifier (e.g. `claude-opus-4-7`, `gpt-4o`). Overrides
    /// `runtime.default_model` from the config.
    #[arg(long)]
    pub model: Option<String>,

    /// Provider name (matches `[[providers]] name = "..."` in the config).
    /// Overrides `runtime.default_provider`.
    #[arg(long)]
    pub provider: Option<String>,

    /// Base URL override applied to the selected provider AFTER merge.
    #[arg(long)]
    pub base_url: Option<String>,

    /// Directory where per-session JSONL files are stored. Overrides
    /// `runtime.session_root`.
    #[arg(long)]
    pub session_root: Option<PathBuf>,

    /// Resume an existing session by ULID. A new session is created if omitted.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,
}

/// Build the layered configuration: file + env + CLI args (in
/// ascending precedence), merge, finalize.
async fn load_layered_config(args: &ChatArgs) -> Result<RuntimeConfig> {
    let file = FileConfigLoader::resolve(args.config.as_ref())
        .context("resolving config file path")?;
    let env = EnvConfigLoader::default();
    let cli_partial = cli_args_to_partial(args);

    let layers = vec![
        file.load().await.context("loading config file")?,
        env.load().await.context("loading environment")?,
        cli_partial,
    ];
    merge_layers(layers)
        .finalize()
        .map_err(|e| anyhow!("finalizing config: {e}"))
}

fn cli_args_to_partial(args: &ChatArgs) -> RuntimeConfigPartial {
    let any = args.model.is_some() || args.provider.is_some() || args.session_root.is_some();
    RuntimeConfigPartial {
        runtime: any.then(|| RuntimeSectionPartial {
            session_root: args.session_root.clone(),
            default_provider: args.provider.clone(),
            default_model: args.model.clone(),
            strategies_dir: None,
        }),
        providers: None,
    }
}

/// Synthesize a `default` provider from legacy environment variables
/// when no `cogito.toml` and no explicit providers are declared. This
/// preserves the Sprint 2 workflow: `cogito chat --model claude-opus-4-7`
/// with only `ANTHROPIC_API_KEY` set continues to work.
///
/// Selection follows Sprint 2 inference: `claude-*` models → Anthropic,
/// otherwise OpenAI-compat.
fn synthesize_legacy_provider(model: &str) -> Result<ProviderConfig> {
    if model.starts_with("claude-") || std::env::var("ANTHROPIC_API_KEY").is_ok() {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set (no cogito.toml found either)")?;
        Ok(ProviderConfig::Anthropic {
            name: "default".into(),
            api_key: key,
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout_secs: None,
        })
    } else {
        let base_url = std::env::var("OPENAI_BASE_URL").context(
            "OPENAI_BASE_URL not set and no cogito.toml found; \
             set OPENAI_BASE_URL or declare providers in a config file",
        )?;
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        Ok(ProviderConfig::OpenAiCompat {
            name: "default".into(),
            api_key,
            base_url,
            auth_header: "Authorization".into(),
            auth_scheme: "Bearer".into(),
            timeout_secs: None,
        })
    }
}

/// Pick the provider entry for this run. Resolution order:
///
/// 1. If `cfg.providers` is empty, synthesize from legacy ENV
///    (Sprint 2 bridge).
/// 2. Else if `cfg.runtime.default_provider` is set, look it up by
///    name; error if not found.
/// 3. Else (auto-select rule already applied by `finalize`), if
///    exactly one provider exists, use it.
/// 4. Else, error.
///
/// Then apply CLI `--base-url` as a post-merge field patch on the
/// chosen provider.
fn select_provider(cfg: &RuntimeConfig, args: &ChatArgs) -> Result<ProviderConfig> {
    let model_for_synth = args
        .model
        .as_deref()
        .or(cfg.runtime.default_model.as_deref())
        .unwrap_or("");

    let mut chosen = if cfg.providers.is_empty() {
        synthesize_legacy_provider(model_for_synth)?
    } else {
        let name = cfg.runtime.default_provider.as_deref().ok_or_else(|| {
            anyhow!("no default_provider selected and no auto-select possible")
        })?;
        cfg.providers
            .iter()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| anyhow!("provider `{name}` not found in config"))?
    };

    if let Some(b) = &args.base_url {
        chosen = patch_base_url(chosen, b.clone());
    }
    Ok(chosen)
}

fn patch_base_url(cfg: ProviderConfig, new_base_url: String) -> ProviderConfig {
    match cfg {
        ProviderConfig::Anthropic {
            name,
            api_key,
            anthropic_version,
            timeout_secs,
            ..
        } => ProviderConfig::Anthropic {
            name,
            api_key,
            base_url: new_base_url,
            anthropic_version,
            timeout_secs,
        },
        ProviderConfig::OpenAiCompat {
            name,
            api_key,
            auth_header,
            auth_scheme,
            timeout_secs,
            ..
        } => ProviderConfig::OpenAiCompat {
            name,
            api_key,
            base_url: new_base_url,
            auth_header,
            auth_scheme,
            timeout_secs,
        },
    }
}

/// Entry point for the `chat` subcommand.
#[allow(clippy::print_stdout)]
pub async fn run(args: ChatArgs) -> Result<()> {
    let cfg = load_layered_config(&args).await?;
    let provider_cfg = select_provider(&cfg, &args)?;
    let gateway: Arc<dyn ModelGateway> =
        build_gateway(provider_cfg).map_err(|e| anyhow!("building gateway: {e}"))?;

    let model_id = args
        .model
        .clone()
        .or_else(|| cfg.runtime.default_model.clone())
        .ok_or_else(|| {
            anyhow!("--model required (or set runtime.default_model in cogito.toml)")
        })?;

    let store = Arc::new(JsonlStore::new(cfg.runtime.session_root.clone()));
    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let mut strategy = HarnessStrategy::default_with_model(&model_id);
    if let Some(sys) = args.system {
        strategy.system_prompt = sys;
    }

    let runtime = Runtime::builder()
        .store(store)
        .model(gateway)
        .tools(tools)
        .strategy(strategy)
        .build()
        .context("building runtime")?;

    let session_id = match args.session_id {
        Some(s) => s
            .parse::<SessionId>()
            .context("invalid session_id (need ULID)")?,
        None => SessionId::new(),
    };

    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .map_err(|e| anyhow!("open_session: {e:?}"))?;

    tracing::info!(%session_id, "cogito chat started (type /quit to exit, Ctrl-C to cancel turn)");

    let cancel_handle = handle.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = cancel_handle.cancel_turn().await;
        }
    });

    let mut stdin = BufReader::new(io::stdin());
    let mut line_buf = Vec::new();
    let mut sub = handle.subscribe();

    loop {
        tokio::select! {
            read = stdin.read_until(b'\n', &mut line_buf) => match read {
                Ok(0) => break,
                Ok(_) => {
                    while matches!(line_buf.last(), Some(b'\n' | b'\r')) {
                        line_buf.pop();
                    }
                    let l = String::from_utf8_lossy(&line_buf).into_owned();
                    line_buf.clear();
                    if l.trim() == "/quit" {
                        break;
                    }
                    if l.trim().is_empty() {
                        continue;
                    }
                    handle.send_user(l).await.context("send_user")?;
                }
                Err(e) => return Err(e).context("stdin read"),
            },
            evt = sub.recv() => match evt {
                Ok(StreamEvent::TextDelta { chunk }) => {
                    use std::io::Write as _;
                    print!("{chunk}");
                    let _ = std::io::stdout().flush();
                }
                Ok(_) => {}
                Err(_) => break,
            },
        }
    }

    let _ = handle.shutdown(Duration::from_secs(30)).await;
    Ok(())
}
```

- [ ] **Step 9.3: Confirm clap arg defaults**

Note that `--model` is now `Option<String>` instead of required `String`. Sprint 2's `just chat` invocations pass `--model`, so behaviour is preserved; users who set `runtime.default_model` in `cogito.toml` can omit `--model`.

- [ ] **Step 9.4: Build + manual smoke test**

Run: `cargo build -p cogito-cli`
Expected: clean build.

Run (legacy path):
```bash
ANTHROPIC_API_KEY=sk-fake just chat --model claude-opus-4-7 --session-root /tmp/chat-smoke << 'EOF'
/quit
EOF
```
Expected: starts cleanly, exits on `/quit`. (May fail to authenticate against Anthropic with a fake key — that is fine, the surface wiring is what we are testing.)

- [ ] **Step 9.5: Lint**

Run: `just fmt && just fix cogito-cli`

- [ ] **Step 9.6: Commit**

```bash
git add crates/cogito-cli/
git commit -m "$(cat <<'EOF'
refactor(cli): drive chat from cogito-config (Sprint 4.5)

Replace the inline build_gateway+ENV-only flow with the three-layer
config pipeline (CLI > ENV > file). `--config <path>` arg added; the
legacy ENV bridge (synthesize a `default` provider from
ANTHROPIC_API_KEY or OPENAI_API_KEY + OPENAI_BASE_URL) keeps Sprint 2
invocations working with zero migration. `--base-url` becomes a
post-merge field patch on the chosen provider; `--system` keeps
overriding the strategy's system_prompt. Integration tests follow
in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `cogito-cli` integration tests

**Files:**
- Create: `crates/cogito-cli/tests/config_legacy_env_bridge.rs`
- Create: `crates/cogito-cli/tests/config_file_only.rs`
- Create: `crates/cogito-cli/tests/config_cli_overrides.rs`
- Create: `crates/cogito-cli/tests/config_anthropic_compat_third_party.rs`
- Modify: `crates/cogito-cli/Cargo.toml` (`[dev-dependencies] tempfile`)

These four tests cover the user-visible surface of Issue #1 sub-needs 1 + 2. They exercise the `select_provider` + `build_gateway` boundary without spinning up the full Runtime (which would require network mocking). To make the helpers accessible from integration tests, expose them from `chat.rs` with `pub(crate)` and re-export from `lib.rs` under a `#[cfg(any(test, feature = "test-internals"))]` gate. **Simpler alternative**: extract the helpers into a sibling module `chat_config.rs` with `pub` visibility, accessed via the crate's own integration-test root.

The simpler alternative is taken below to avoid feature-flag overhead.

- [ ] **Step 10.1: Extract config helpers into `chat_config.rs`**

Create `crates/cogito-cli/src/chat_config.rs`. Move these functions from `chat.rs` to `chat_config.rs` and change their visibility to `pub`:

```rust
//! Configuration helpers for `cogito chat`. Exposed `pub` so the
//! integration tests under `crates/cogito-cli/tests/` can exercise
//! the boundary without going through the full Runtime.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use cogito_config::{
    ConfigLoader, EnvConfigLoader, FileConfigLoader, RuntimeConfig,
    RuntimeConfigPartial, RuntimeSectionPartial, merge_layers,
};
use cogito_model::ProviderConfig;

/// Subset of `ChatArgs` needed by the config helpers. The CLI build
/// passes a real `ChatArgs`; tests pass a `ChatConfigInputs` directly.
#[derive(Debug, Default, Clone)]
pub struct ChatConfigInputs {
    pub config_path: Option<PathBuf>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub session_root: Option<PathBuf>,
}

pub async fn load_layered_config(inputs: &ChatConfigInputs) -> Result<RuntimeConfig> {
    let file = FileConfigLoader::resolve(inputs.config_path.as_ref())
        .context("resolving config file path")?;
    let env = EnvConfigLoader::default();
    let cli_partial = cli_inputs_to_partial(inputs);

    let layers = vec![
        file.load().await.context("loading config file")?,
        env.load().await.context("loading environment")?,
        cli_partial,
    ];
    merge_layers(layers)
        .finalize()
        .map_err(|e| anyhow!("finalizing config: {e}"))
}

fn cli_inputs_to_partial(inputs: &ChatConfigInputs) -> RuntimeConfigPartial {
    let any = inputs.model.is_some()
        || inputs.provider.is_some()
        || inputs.session_root.is_some();
    RuntimeConfigPartial {
        runtime: any.then(|| RuntimeSectionPartial {
            session_root: inputs.session_root.clone(),
            default_provider: inputs.provider.clone(),
            default_model: inputs.model.clone(),
            strategies_dir: None,
        }),
        providers: None,
    }
}

pub fn synthesize_legacy_provider(model: &str) -> Result<ProviderConfig> {
    if model.starts_with("claude-") || std::env::var("ANTHROPIC_API_KEY").is_ok() {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set (no cogito.toml found either)")?;
        Ok(ProviderConfig::Anthropic {
            name: "default".into(),
            api_key: key,
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout_secs: None,
        })
    } else {
        let base_url = std::env::var("OPENAI_BASE_URL").context(
            "OPENAI_BASE_URL not set and no cogito.toml found; \
             set OPENAI_BASE_URL or declare providers in a config file",
        )?;
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        Ok(ProviderConfig::OpenAiCompat {
            name: "default".into(),
            api_key,
            base_url,
            auth_header: "Authorization".into(),
            auth_scheme: "Bearer".into(),
            timeout_secs: None,
        })
    }
}

pub fn select_provider(
    cfg: &RuntimeConfig,
    inputs: &ChatConfigInputs,
) -> Result<ProviderConfig> {
    let model_for_synth = inputs
        .model
        .as_deref()
        .or(cfg.runtime.default_model.as_deref())
        .unwrap_or("");

    let mut chosen = if cfg.providers.is_empty() {
        synthesize_legacy_provider(model_for_synth)?
    } else {
        let name = cfg.runtime.default_provider.as_deref().ok_or_else(|| {
            anyhow!("no default_provider selected and no auto-select possible")
        })?;
        cfg.providers
            .iter()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| anyhow!("provider `{name}` not found in config"))?
    };

    if let Some(b) = &inputs.base_url {
        chosen = patch_base_url(chosen, b.clone());
    }
    Ok(chosen)
}

pub fn patch_base_url(cfg: ProviderConfig, new_base_url: String) -> ProviderConfig {
    match cfg {
        ProviderConfig::Anthropic {
            name,
            api_key,
            anthropic_version,
            timeout_secs,
            ..
        } => ProviderConfig::Anthropic {
            name,
            api_key,
            base_url: new_base_url,
            anthropic_version,
            timeout_secs,
        },
        ProviderConfig::OpenAiCompat {
            name,
            api_key,
            auth_header,
            auth_scheme,
            timeout_secs,
            ..
        } => ProviderConfig::OpenAiCompat {
            name,
            api_key,
            base_url: new_base_url,
            auth_header,
            auth_scheme,
            timeout_secs,
        },
    }
}
```

- [ ] **Step 10.2: Update `chat.rs` and `main.rs` to use `chat_config`**

Modify `crates/cogito-cli/src/main.rs`. Add `mod chat_config;` alongside `mod chat;`:

```rust
mod chat;
mod chat_config;
```

Modify `crates/cogito-cli/src/chat.rs`. Remove the now-duplicated functions (`load_layered_config`, `cli_args_to_partial`, `synthesize_legacy_provider`, `select_provider`, `patch_base_url`) and replace their call sites in `run` with `crate::chat_config::*`. The relevant `run` snippet becomes:

```rust
pub async fn run(args: ChatArgs) -> Result<()> {
    let inputs = crate::chat_config::ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    };
    let cfg = crate::chat_config::load_layered_config(&inputs).await?;
    let provider_cfg = crate::chat_config::select_provider(&cfg, &inputs)?;
    let gateway: Arc<dyn ModelGateway> =
        cogito_model::build_gateway(provider_cfg).map_err(|e| anyhow!("building gateway: {e}"))?;
    // ... rest unchanged
}
```

- [ ] **Step 10.3: Expose `chat_config` to integration tests via a library target**

The default `cogito-cli` is a binary-only crate. Integration tests live under `tests/`, but they cannot reach into the binary's internal modules. The fix: add a library target so the binary and tests both depend on it.

Modify `crates/cogito-cli/Cargo.toml`. Add (or merge into existing):

```toml
[lib]
path = "src/lib.rs"

[[bin]]
name = "cogito"
path = "src/main.rs"

[dev-dependencies]
tempfile = { workspace = true }
tokio    = { workspace = true, features = ["macros", "rt-multi-thread"] }
temp-env = { workspace = true, features = ["async_closure"] }
```

Create `crates/cogito-cli/src/lib.rs`:

```rust
//! cogito-cli library: exposes `chat_config` helpers for integration
//! tests. The binary entry point lives in `main.rs`.

pub mod chat_config;
```

Modify `crates/cogito-cli/src/main.rs` to import `chat_config` via the
crate root (since the lib target now owns it):

```rust
use cogito_cli::chat_config; // available as `cogito_cli::chat_config` from main.rs too
mod chat;
```

Then update `chat.rs`'s use sites from `crate::chat_config` to
`cogito_cli::chat_config` (or `use crate::chat_config` if the binary
target's crate name is `cogito_cli`; `cargo` exposes the lib under the
package name).

- [ ] **Step 10.4: Write integration test 1 — legacy ENV bridge**

Create `crates/cogito-cli/tests/config_legacy_env_bridge.rs`:

```rust
//! Issue #1 sub-need 1 (Anthropic flavour): no cogito.toml, only
//! ANTHROPIC_API_KEY set. Sprint 4.5 must reproduce Sprint 2 behaviour.

use std::sync::Mutex;

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_all() {
    for k in [
        "COGITO_CONFIG",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "COGITO_SESSION_ROOT",
        "COGITO_DEFAULT_PROVIDER",
        "COGITO_DEFAULT_MODEL",
        "COGITO_STRATEGIES_DIR",
        "XDG_CONFIG_HOME",
    ] {
        unsafe { std::env::remove_var(k); }
    }
}

#[tokio::test]
async fn legacy_anthropic_bridge() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_all();
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test-bridge"); }
    unsafe { std::env::set_var("XDG_CONFIG_HOME", "/nonexistent/path"); }

    let inputs = ChatConfigInputs {
        model: Some("claude-opus-4-7".into()),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    assert!(cfg.providers.is_empty(), "no cogito.toml ⇒ empty providers pre-bridge");

    let chosen = select_provider(&cfg, &inputs).expect("select with bridge");
    match chosen {
        ProviderConfig::Anthropic { name, api_key, base_url, .. } => {
            assert_eq!(name, "default");
            assert_eq!(api_key, "sk-test-bridge");
            assert_eq!(base_url, "https://api.anthropic.com");
        }
        _ => panic!("expected Anthropic"),
    }
    clear_all();
}

#[tokio::test]
async fn legacy_openai_compat_bridge() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_all();
    unsafe {
        std::env::set_var("OPENAI_BASE_URL", "http://vllm.internal:8000/v1");
        std::env::set_var("OPENAI_API_KEY", "sk-openai");
        std::env::set_var("XDG_CONFIG_HOME", "/nonexistent/path");
    }

    let inputs = ChatConfigInputs {
        model: Some("qwen-72b".into()),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    let chosen = select_provider(&cfg, &inputs).expect("select");
    match chosen {
        ProviderConfig::OpenAiCompat { name, base_url, api_key, .. } => {
            assert_eq!(name, "default");
            assert_eq!(base_url, "http://vllm.internal:8000/v1");
            assert_eq!(api_key.as_deref(), Some("sk-openai"));
        }
        _ => panic!("expected OpenAiCompat"),
    }
    clear_all();
}
```

- [ ] **Step 10.5: Write integration test 2 — file-only**

Create `crates/cogito-cli/tests/config_file_only.rs`:

```rust
//! File-only configuration: cogito.toml declares an Anthropic provider
//! with `${ANTHROPIC_API_KEY}` interpolated.

use std::sync::Mutex;

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn file_declares_anthropic_with_env_interpolation() {
    let _g = ENV_LOCK.lock().unwrap();
    for k in ["COGITO_CONFIG", "ANTHROPIC_API_KEY", "XDG_CONFIG_HOME"] {
        unsafe { std::env::remove_var(k); }
    }
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-file-test"); }

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "./sessions"
            default_model = "claude-opus-4-7"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "${ANTHROPIC_API_KEY}"
            base_url = "https://api.anthropic.com"
        "#,
    )
    .unwrap();

    let inputs = ChatConfigInputs {
        config_path: Some(path),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    assert_eq!(cfg.runtime.default_provider.as_deref(), Some("anthropic-prod"));
    let chosen = select_provider(&cfg, &inputs).expect("select");
    match chosen {
        ProviderConfig::Anthropic { name, api_key, .. } => {
            assert_eq!(name, "anthropic-prod");
            assert_eq!(api_key, "sk-file-test");
        }
        _ => panic!("expected Anthropic"),
    }

    unsafe { std::env::remove_var("ANTHROPIC_API_KEY"); }
}
```

- [ ] **Step 10.6: Write integration test 3 — CLI overrides file**

Create `crates/cogito-cli/tests/config_cli_overrides.rs`:

```rust
//! `--base-url` flag must override the chosen provider's base_url.

use std::sync::Mutex;

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn cli_base_url_overrides_file_base_url() {
    let _g = ENV_LOCK.lock().unwrap();
    for k in ["COGITO_CONFIG", "XDG_CONFIG_HOME"] {
        unsafe { std::env::remove_var(k); }
    }

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [[providers]]
            name = "anthropic-only"
            kind = "anthropic"
            api_key = "k"
            base_url = "https://from-file"
        "#,
    )
    .unwrap();

    let inputs = ChatConfigInputs {
        config_path: Some(path),
        base_url: Some("https://from-cli".into()),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    let chosen = select_provider(&cfg, &inputs).expect("select");
    match chosen {
        ProviderConfig::Anthropic { base_url, .. } => {
            assert_eq!(base_url, "https://from-cli");
        }
        _ => panic!("expected Anthropic"),
    }
}
```

- [ ] **Step 10.7: Write integration test 4 — Anthropic-compat third party**

Create `crates/cogito-cli/tests/config_anthropic_compat_third_party.rs`:

```rust
//! Issue #1 sub-need 2: same kind=anthropic but pointing at an
//! internal endpoint. Two providers coexist (prod + internal); the
//! user picks via runtime.default_provider in the file.

use std::sync::Mutex;

use cogito_cli::chat_config::{ChatConfigInputs, load_layered_config, select_provider};
use cogito_model::ProviderConfig;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn two_anthropic_providers_internal_selected() {
    let _g = ENV_LOCK.lock().unwrap();
    for k in ["COGITO_CONFIG", "XDG_CONFIG_HOME"] {
        unsafe { std::env::remove_var(k); }
    }

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_provider = "anthropic-internal"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "k-prod"
            base_url = "https://api.anthropic.com"

            [[providers]]
            name = "anthropic-internal"
            kind = "anthropic"
            api_key = "k-internal"
            base_url = "https://internal.api/anthropic/v1"
        "#,
    )
    .unwrap();

    let inputs = ChatConfigInputs {
        config_path: Some(path),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    assert_eq!(cfg.providers.len(), 2);

    let chosen = select_provider(&cfg, &inputs).expect("select");
    match chosen {
        ProviderConfig::Anthropic { name, api_key, base_url, .. } => {
            assert_eq!(name, "anthropic-internal");
            assert_eq!(api_key, "k-internal");
            assert_eq!(base_url, "https://internal.api/anthropic/v1");
        }
        _ => panic!("expected Anthropic"),
    }
}

#[tokio::test]
async fn cli_provider_flag_overrides_file_default() {
    let _g = ENV_LOCK.lock().unwrap();
    for k in ["COGITO_CONFIG", "XDG_CONFIG_HOME"] {
        unsafe { std::env::remove_var(k); }
    }

    let dir = tempdir().unwrap();
    let path = dir.path().join("cogito.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_provider = "anthropic-prod"

            [[providers]]
            name = "anthropic-prod"
            kind = "anthropic"
            api_key = "k-prod"

            [[providers]]
            name = "anthropic-internal"
            kind = "anthropic"
            api_key = "k-internal"
            base_url = "https://internal/v1"
        "#,
    )
    .unwrap();

    let inputs = ChatConfigInputs {
        config_path: Some(path),
        provider: Some("anthropic-internal".into()),
        ..Default::default()
    };
    let cfg = load_layered_config(&inputs).await.expect("load");
    let chosen = select_provider(&cfg, &inputs).expect("select");
    match chosen {
        ProviderConfig::Anthropic { name, .. } => assert_eq!(name, "anthropic-internal"),
        _ => panic!("expected Anthropic"),
    }
}
```

- [ ] **Step 10.8: Run tests single-threaded**

Run: `cargo test -p cogito-cli -- --test-threads=1`
Expected: all four integration tests pass.

(Single-threaded because each test mutates the process env via the shared `ENV_LOCK`. `--test-threads=1` is a safety belt even with the mutex.)

- [ ] **Step 10.9: Full CI gate**

Run: `just ci`
Expected: workspace green.

- [ ] **Step 10.10: Commit**

```bash
git add crates/cogito-cli/
git commit -m "$(cat <<'EOF'
test(cli): integration coverage for Sprint 4.5 config surface

Four scenarios closing the loop on Issue #1 sub-needs 1 + 2:

- legacy_env_bridge: no cogito.toml, only ANTHROPIC_API_KEY or
  OPENAI_BASE_URL set; CLI synthesizes a `default` provider.
- file_only: cogito.toml + `${ANTHROPIC_API_KEY}` interpolation;
  provider auto-selected when sole.
- cli_overrides: `--base-url` patches the chosen provider after
  merge.
- anthropic_compat_third_party: two kind="anthropic" providers
  coexist; runtime.default_provider picks one; `--provider` overrides.

Helpers extracted to chat_config.rs and exposed via a thin lib
target so tests can drive ChatConfigInputs directly without
spinning up a Runtime.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Docs sync — ROADMAP, H10 footnote, CHANGELOG

**Files:**
- Modify: `ROADMAP.md`
- Modify: `docs/components/H10-strategy-selector.md`
- Modify: `CHANGELOG.md` (if present; otherwise skip)

- [ ] **Step 11.1: Add Sprint 4.5 to ROADMAP**

Modify `ROADMAP.md`. Find the section header `#### Sprint 5 · Multi-model Strategy` and insert this block **before** it:

```markdown
#### Sprint 4.5 · 配置文件 + base_url override (0.5–1 day)

- [x] `cogito-config` crate(value types + ConfigLoader trait + EnvConfigLoader + merge)
- [x] `cogito-config` feature `file` → FileConfigLoader (`cogito.toml`)
- [x] `cogito-model::ProviderConfig` + `build_gateway` 工厂
- [x] `cogito-cli` 重构 `chat.rs`:`--config` 参数 + 三层 merge
- [x] Legacy ENV bridge:`cogito.toml` 缺席时合成 `default` provider
- [x] 单元/集成测试覆盖 merge、插值、搜索路径、CLI 流程
- [x] 文档:ADR-0017 落地、H10 doc 注脚、ROADMAP 更新

Closes GitLab Issue #1 sub-needs 1 + 2. Sub-need 3 (OpenAI Responses
adapter) remains scheduled for Sprint 5.

```

- [ ] **Step 11.2: H10 doc footnote**

Modify `docs/components/H10-strategy-selector.md`. Find the heading
`## v0.x Sprint 5 scope (designed, not implemented)` and append at
the end of that section (before `## Example strategy YAML (Sprint 5+)`):

```markdown
> **2026-05-21 update (ADR-0017 §9):** Strategy file basename
> (without `.yaml`) is the canonical strategy name; the YAML body
> drops `name:` and `applicable_models:` fields. The two existing
> draft files (`strategies/claude-opus.yaml`, `strategies/gpt-4.yaml`)
> will be rewritten when Sprint 5 lands the loader.
```

- [ ] **Step 11.3: CHANGELOG (if present)**

Run: `ls CHANGELOG.md 2>/dev/null`
- If CHANGELOG.md exists: prepend a Sprint 4.5 block under
  `[Unreleased]`:
  ```markdown
  ### Added — Sprint 4.5 (config-file loading)

  - `cogito-config` crate: value types, `ConfigLoader` trait,
    `EnvConfigLoader`, layered partial merge, `${VAR}` interpolation,
    `FileConfigLoader` (feature `file`).
  - `cogito-model::ProviderConfig` (tagged-union over provider kinds)
    + `build_gateway` factory.
  - `cogito chat`: `--config <path>`; legacy ENV bridge preserves
    Sprint 2 invocations.

  ### Changed

  - `cogito chat --model` now optional (falls back to
    `runtime.default_model` in `cogito.toml`).
  ```
- If CHANGELOG.md does not exist: skip; do not create one.

- [ ] **Step 11.4: Commit**

```bash
git add ROADMAP.md docs/components/H10-strategy-selector.md CHANGELOG.md 2>/dev/null || git add ROADMAP.md docs/components/H10-strategy-selector.md
git commit -m "$(cat <<'EOF'
docs(roadmap): record Sprint 4.5 completion + H10 schema note

ROADMAP Sprint 4.5 subsection marked done. H10 doc footnoted with
the filename = strategy name + dropped applicable_models decision
from ADR-0017 §9.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Acceptance Criteria

Before declaring Sprint 4.5 complete:

1. `just ci` is green on the workspace (fmt + clippy pedantic + tests across all crates).
2. `cargo test -p cogito-config` (default features) and `cargo test -p cogito-config --features file -- --test-threads=1` both pass.
3. `cargo test -p cogito-cli -- --test-threads=1` passes (4 integration test files).
4. Manual smoke test 1: `ANTHROPIC_API_KEY=sk-... just chat --model claude-opus-4-7` runs a complete turn (legacy bridge path).
5. Manual smoke test 2: write a `cogito.toml` declaring real + internal Anthropic providers; `cogito chat --provider anthropic-internal --model claude-opus-4-7` sends to internal endpoint (verifiable via tcpdump / mitmproxy / intentionally bad endpoint).
6. ADR-0017 has not been modified during implementation. If a decision needed adjustment, an ADR amendment was made before the corresponding code change (not after).
7. ROADMAP Sprint 4.5 subsection added and all checkboxes ticked.
8. No `#[ignore]` attributes added to make tests pass.
9. No `unwrap_used` / `expect_used` / `panic` warnings in production code (test code is exempt by workspace lint config).

---

## Self-Review Notes

**Spec coverage:**
- Spec §1.1 In-scope items 1–8: covered by Tasks 1–11.
- Spec §1.2 Out-of-scope: respected throughout (no strategy loader; no OpenAI Responses; no plugin schema; no DB loader; no profile; no hot reload; no array element-wise merge).
- Spec §5.1 unit tests: covered by Tasks 3, 5, 6, 7, 8 (inline + `tests/` integration).
- Spec §5.2 cogito-model tests: covered by Task 1.
- Spec §5.3 cogito-cli integration tests: covered by Task 10.
- Spec §6 implementation order: this plan follows it (model → config skeleton → types → loader → env → merge → interpolate → file → CLI → tests → docs).
- Spec §8 acceptance criteria: matched in this plan's Acceptance Criteria section.

**Type / name consistency:**
- `ProviderConfig` enum, `build_gateway` function: consistent across Tasks 1, 9, 10.
- `RuntimeConfig`, `RuntimeConfigPartial`, `RuntimeSection`, `RuntimeSectionPartial`: consistent across Tasks 3, 6, 8, 9, 10.
- `ConfigLoader`, `ConfigError`: consistent across Tasks 4, 5, 7, 8.
- `EnvConfigLoader`, `FileConfigLoader`: consistent across Tasks 5, 8, 9, 10.
- `load_runtime_config` convenience: defined in Task 8, used as a reference shape in CLI refactor Task 9 (which uses lower-level functions because CLI also needs the cli_partial layer).

**No placeholders confirmed:** Every code step shows complete content; every test step shows the test code; every command has the exact invocation; no "TBD" / "fill in details" / "similar to Task N" references.

---

## References

- [Sprint 4.5 design spec](../specs/2026-05-21-sprint-4-5-config-file-design.md)
- [ADR-0017 — Cogito Runtime configuration model](../../adr/0017-cogito-runtime-configuration-model.md)
- [Configuration overview](../../configuration/overview.md)
- [H10 Strategy Selector](../../components/H10-strategy-selector.md)
- [`CLAUDE.md` §Coding standards](../../../CLAUDE.md)
- ROADMAP §"Sprint 4 (Async Jobs)" — predecessor sprint
- GitLab Issue gitlab.sz.sensetime.com/compass/cogito#1

# Plugin Loader — Skills + MCP (ADR-0021) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load local-path plugin directories that bundle Skills + MCP servers, namespace their artifacts `<plugin_id>:<name>`, and fold them into the existing skill registry and MCP provider — wired into the CLI.

**Architecture:** A new `cogito-plugin` (Hands) crate resolves `[[plugins]]` entries into a `PluginContributions { skill_roots, mcp_servers }` value. It does **not** build providers — the existing `SkillRegistry` keeps cross-scope precedence (it gains a `plugin_roots` input on `ScanConfig`), and the namespaced `McpServerConfig`s are concatenated into the existing `build_mcp_provider` call. `cogito-config` gains a `[[plugins]]` section (`PluginEntry` defined in `cogito-plugin`, aggregated by config, mirroring the existing `cogito-config → cogito-mcp` edge).

**Tech Stack:** Rust 2024, `serde`/`toml`, `cargo nextest`. New crate: `cogito-plugin`. Modified: `cogito-skills`, `cogito-config`, `cogito-cli`.

**Plan scope:** Plan 2 of 2 for Sprint 12. Depends on Plan 1 (ADR-0028) being merged first.

**Authoritative design:** [ADR-0021](../../adr/0021-plugin-manifest-and-loader.md) + [Sprint 12 spec](../specs/2026-05-30-sprint-12-saas-session-plugin-design.md) §4.

**Deferred (reserved dirs, not loaded):** hooks (`hooks/`), subagent roles (`agents/`), slash commands (`commands/`). Their presence in a plugin is not an error; the loader ignores them.

**Ground-truth notes (verified against source 2026-05-30):**
- `SkillRegistry::scan(config: ScanConfig)` takes `ScanConfig` **by value** (`#[allow(clippy::needless_pass_by_value)]`), returns `Result<Self, SkillRegistryError>`. It calls `discover_skills(&config)`. `ScanConfig` derives `Default`; today its fields are `workspace_root: Option<PathBuf>`, `user_dir: Option<PathBuf>`, `include_system: bool`.
- `discover_skills` (in `cogito-skills/src/discovery.rs`) walks Repo + User scopes via the helper `scan_skills_dir(dir, &SkillSource, &mut out)`, which iterates child dirs of `dir` looking for `<name>/SKILL.md`. `DiscoveredSkill { parsed: ParsedSkill, source: SkillSource, dir }`.
- `SkillSource::Plugin { plugin_id: String }` already exists in `cogito-protocol/src/skill.rs`; skill names are namespaced `<plugin_id>:<name>`.
- `cogito-skills` exports only `SkillRegistry` / `SkillRegistryError` from `lib.rs`; `discovery::ScanConfig` is referenced fully-qualified by the CLI (`cogito_skills::discovery::ScanConfig`).
- `McpServerConfig` is in `cogito-mcp/src/config.rs` (re-exported `cogito_mcp::McpServerConfig`); `cogito-config` already depends on `cogito-mcp` and uses it. `name` is the field renamed for namespacing; `transport` is `#[serde(flatten)]`.
- CLI: `build_skill_provider(cfg)` in `chat_config.rs` builds `ScanConfig` and calls `SkillRegistry::scan(scan)` (by value). `build_tool_provider(cfg, job_mgr)` in `chat.rs` calls `cogito_mcp::build_mcp_provider(&cfg.mcp_servers)`. `run` opens the session via `runtime.open_session(...)`.

---

## File structure

| File | Responsibility | Action |
|---|---|---|
| `crates/cogito-skills/src/discovery.rs` | `PluginSkillRoot`; `ScanConfig.plugin_roots`; Plugin-scope walk + namespacing | Modify |
| `crates/cogito-skills/src/lib.rs` | Export `PluginSkillRoot` + `discovery::ScanConfig` | Modify |
| `crates/cogito-plugin/Cargo.toml` | Crate manifest + deps | Create |
| `crates/cogito-plugin/src/lib.rs` | `PluginEntry`, `ArtifactOverride`, `PluginContributions`, `PluginError` | Create |
| `crates/cogito-plugin/src/manifest.rs` | `.cogito-plugin/plugin.toml` + `.claude-plugin/plugin.json` metadata fallback | Create |
| `crates/cogito-plugin/src/discovery.rs` | `PluginSet::load`: resolution, scan, namespacing, overrides, id-uniqueness | Create |
| `Cargo.toml` (workspace) | Register crate + `[workspace.dependencies]` entry | Modify |
| `crates/cogito-config/Cargo.toml` | Depend on `cogito-plugin` | Modify |
| `crates/cogito-config/src/types.rs` | `RuntimeConfig.plugins` + partial + finalize | Modify |
| `crates/cogito-cli/Cargo.toml` | Depend on `cogito-plugin` | Modify |
| `crates/cogito-cli/src/chat_config.rs` | `build_skill_provider` accepts plugin roots | Modify |
| `crates/cogito-cli/src/chat.rs` | Load plugins; fold MCP servers | Modify |
| `crates/cogito-plugin/tests/loader.rs` | Unit tests | Create |
| `crates/cogito-plugin/tests/integration.rs` | Acceptance tests | Create |

---

## Task 1: `cogito-skills` — `PluginSkillRoot` + `ScanConfig.plugin_roots` + Plugin scope

Done first because `cogito-plugin` (Task 2) imports `cogito_skills::PluginSkillRoot`.

**Files:**
- Modify: `crates/cogito-skills/src/discovery.rs`, `crates/cogito-skills/src/lib.rs`
- Test: add a `#[cfg(test)]` module to `discovery.rs`

- [ ] **Step 1: Write the failing test**

At the bottom of `crates/cogito-skills/src/discovery.rs`, add:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod plugin_scope_tests {
    use super::*;

    #[test]
    fn discovers_plugin_skill_namespaced() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        let sdir = skills_dir.join("review-rust");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(
            sdir.join("SKILL.md"),
            "---\nname: review-rust\ndescription: d\n---\nbody\n",
        )
        .unwrap();

        let cfg = ScanConfig {
            workspace_root: None,
            user_dir: None,
            include_system: false,
            plugin_roots: vec![PluginSkillRoot {
                plugin_id: "code-review".to_string(),
                dir: skills_dir,
            }],
        };
        let found = discover_skills(&cfg).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].parsed.name, "code-review:review-rust");
        assert!(matches!(
            found[0].source,
            cogito_protocol::skill::SkillSource::Plugin { .. }
        ));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-skills discovers_plugin_skill_namespaced`
Expected: FAIL to compile — `PluginSkillRoot` and `ScanConfig.plugin_roots` do not exist.

- [ ] **Step 3: Add `PluginSkillRoot` and the `plugin_roots` field**

In `crates/cogito-skills/src/discovery.rs`, add the struct after `ScanConfig`:

```rust
/// One plugin's skills directory, contributed by the Plugin loader
/// (ADR-0021). Skills found here are registered at Plugin scope and
/// namespaced `<plugin_id>:<name>`.
#[derive(Clone, Debug)]
pub struct PluginSkillRoot {
    /// Globally-unique plugin id (the namespace prefix).
    pub plugin_id: String,
    /// The plugin's `skills/` directory (contains `<name>/SKILL.md`).
    pub dir: PathBuf,
}
```

Add the field to `ScanConfig` (after `include_system`):

```rust
    /// Plugin scope: plugin skill roots to register, each namespaced
    /// `<plugin_id>:<name>`. Empty skips Plugin scope. Populated by the
    /// Plugin loader (ADR-0021).
    pub plugin_roots: Vec<PluginSkillRoot>,
```

(`ScanConfig` derives `Default`; `Vec` defaults empty, so existing constructions using explicit literals must add `plugin_roots` — there is exactly one such literal, in the CLI, fixed in Task 6. The unit tests here construct full literals.)

- [ ] **Step 4: Register plugin roots in `discover_skills`**

In `discover_skills`, after the `if config.include_system { ... }` block and before `Ok(out)`, add:

```rust
    for root in &config.plugin_roots {
        let before = out.len();
        scan_skills_dir(
            &root.dir,
            &SkillSource::Plugin {
                plugin_id: root.plugin_id.clone(),
            },
            &mut out,
        )?;
        // Namespace each newly-discovered plugin skill `<plugin_id>:<name>`.
        for d in &mut out[before..] {
            d.parsed.name = format!("{}:{}", root.plugin_id, d.parsed.name);
        }
    }
```

> `ParsedSkill::name` is mutated here within the same crate. If the field is not `pub`/`pub(crate)`, widen its visibility in `metadata.rs` (it is already read cross-module as `d.parsed.name`).

- [ ] **Step 5: Export the new type**

In `crates/cogito-skills/src/lib.rs`, extend the re-export so the loader and CLI can name these types:

```rust
pub use discovery::{PluginSkillRoot, ScanConfig};
pub use registry::{SkillRegistry, SkillRegistryError};
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo nextest run -p cogito-skills discovers_plugin_skill_namespaced`
Expected: PASS.

- [ ] **Step 7: Confirm scan registers the namespaced skill end-to-end**

Add one more test to the same module, then run it:

```rust
    #[test]
    fn registry_registers_namespaced_plugin_skill() {
        use cogito_protocol::skill::SkillProvider;
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        let sdir = skills_dir.join("s1");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(sdir.join("SKILL.md"), "---\nname: s1\ndescription: d\n---\nb\n").unwrap();

        let cfg = ScanConfig {
            workspace_root: None,
            user_dir: None,
            include_system: false,
            plugin_roots: vec![PluginSkillRoot { plugin_id: "p1".into(), dir: skills_dir }],
        };
        let reg = crate::registry::SkillRegistry::scan(cfg).unwrap();
        assert!(reg.is_registered("p1:s1"));
    }
```

Run: `cargo nextest run -p cogito-skills registry_registers_namespaced_plugin_skill`
Expected: PASS. (`scan` takes `ScanConfig` by value — pass `cfg`, not `&cfg`.)

- [ ] **Step 8: Whole crate + commit**

Run: `make test CRATE=cogito-skills`
Expected: all PASS (existing Repo/User/sigil tests unaffected; `plugin_roots` defaults empty).

```bash
git add crates/cogito-skills/src/discovery.rs crates/cogito-skills/src/lib.rs crates/cogito-skills/src/metadata.rs
git commit -m "feat(skills): PluginSkillRoot + ScanConfig.plugin_roots register Plugin-scope skills (ADR-0021)"
```

---

## Task 2: Scaffold the `cogito-plugin` crate

> New-crate approval: `cogito-plugin` is pre-listed in ROADMAP Sprint 12 + ARCHITECTURE workspace layout. No further approval needed.

**Files:**
- Create: `crates/cogito-plugin/Cargo.toml`, `crates/cogito-plugin/src/lib.rs`, `src/manifest.rs`, `src/discovery.rs`
- Modify: root `Cargo.toml`

- [ ] **Step 1: Create the crate manifest**

Create `crates/cogito-plugin/Cargo.toml`:

```toml
[package]
name = "cogito-plugin"
description = "Local-path plugin loader for the cogito Agent Runtime: manifest parsing, artifact discovery, and provider contributions (Skills + MCP) for v0.2."
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
cogito-protocol = { workspace = true }
cogito-mcp      = { workspace = true }
cogito-skills   = { workspace = true }

serde      = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
toml       = { workspace = true }
thiserror  = { workspace = true }
tracing    = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2: Register the crate in the workspace**

In the root `Cargo.toml`, add `"crates/cogito-plugin",` to `members` (next to `"crates/cogito-mcp",`), and in `[workspace.dependencies]` add (next to the `cogito-mcp` line):

```toml
cogito-plugin = { path = "crates/cogito-plugin" }
```

- [ ] **Step 3: Create `lib.rs` with the public value types**

Create `crates/cogito-plugin/src/lib.rs`:

```rust
//! Local-path plugin loader (v0.2, Skills + MCP). See ADR-0021.
//!
//! A plugin is a directory with a `.cogito-plugin/plugin.toml` manifest
//! (or a `.claude-plugin/plugin.json` read for metadata only) bundling
//! `skills/` and `mcp.toml`. [`PluginSet::load`] resolves declared
//! entries into [`PluginContributions`] that the caller folds into the
//! existing `SkillRegistry` and `build_mcp_provider`.

#![forbid(unsafe_code)]

mod discovery;
mod manifest;

pub use discovery::PluginSet;
pub use manifest::PluginManifest;

use std::path::PathBuf;

use cogito_mcp::McpServerConfig;
use cogito_skills::PluginSkillRoot;

/// One `[[plugins]]` entry from `cogito.toml`. Owned here; aggregated by
/// `cogito-config` (mirrors the `cogito-config → cogito-mcp` edge).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PluginEntry {
    /// Path to the plugin directory (absolute, or relative to `cogito.toml`).
    pub path: String,
    /// Whether the plugin is active. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-artifact enable/disable overrides.
    #[serde(default)]
    pub artifact_overrides: Vec<ArtifactOverride>,
}

fn default_true() -> bool {
    true
}

/// Fine-grained override disabling a single bundled artifact.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ArtifactOverride {
    /// Target plugin id.
    pub plugin: String,
    /// Artifact kind: `"skill"` or `"mcp"` (v0.2).
    pub kind: String,
    /// Bare artifact name (pre-namespacing).
    pub name: String,
    /// Whether the artifact is enabled.
    pub enabled: bool,
}

/// Everything a plugin set contributes, ready to fold into the existing
/// registries. No providers are built here.
#[derive(Debug, Default)]
pub struct PluginContributions {
    /// Plugin skill roots, for `SkillRegistry` Plugin scope.
    pub skill_roots: Vec<PluginSkillRoot>,
    /// Namespaced MCP server configs, to concatenate before
    /// `build_mcp_provider`.
    pub mcp_servers: Vec<McpServerConfig>,
}

/// Errors raised while loading a plugin set.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// A declared plugin path does not exist or is not a directory.
    #[error("plugin path not found or not a directory: {0}")]
    PathNotFound(PathBuf),
    /// The manifest could not be read or parsed.
    #[error("invalid plugin manifest at {path}: {source}")]
    Manifest {
        /// Manifest path.
        path: PathBuf,
        /// Underlying parse error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Two plugins declared the same id.
    #[error("duplicate plugin id `{id}` (declared at {first} and {second})")]
    DuplicateId {
        /// The conflicting id.
        id: String,
        /// First plugin path.
        first: PathBuf,
        /// Second plugin path.
        second: PathBuf,
    },
}
```

- [ ] **Step 4: Create stub `manifest.rs` and `discovery.rs` so it compiles**

Create `crates/cogito-plugin/src/manifest.rs`:

```rust
//! Plugin manifest parsing (filled in Task 3).
#![allow(dead_code)]

/// Internal manifest model after parsing.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Globally unique plugin id (the namespace prefix).
    pub id: String,
    /// Optional semver.
    pub version: Option<String>,
    /// Optional human description.
    pub description: Option<String>,
    /// Skills directory relative to the plugin root.
    pub skills_dir: String,
    /// MCP file relative to the plugin root.
    pub mcp_file: String,
}
```

Create `crates/cogito-plugin/src/discovery.rs`:

```rust
//! Plugin discovery + contribution assembly (filled in Task 4).
#![allow(dead_code)]

use std::path::Path;

use crate::{PluginContributions, PluginEntry, PluginError};

/// Resolves declared plugin entries into contributions.
pub struct PluginSet;

impl PluginSet {
    /// Load all enabled plugins, namespacing and de-conflicting artifacts.
    ///
    /// # Errors
    /// Returns [`PluginError`] on missing paths, bad manifests, or
    /// duplicate plugin ids.
    pub fn load(
        _entries: &[PluginEntry],
        _config_dir: &Path,
    ) -> Result<PluginContributions, PluginError> {
        Ok(PluginContributions::default())
    }
}
```

- [ ] **Step 5: Build + commit**

Run: `cargo build -p cogito-plugin`
Expected: builds clean.

```bash
git add crates/cogito-plugin/ Cargo.toml
git commit -m "feat(plugin): scaffold cogito-plugin crate (ADR-0021)"
```

---

## Task 3: Manifest parsing (TOML primary + JSON metadata fallback)

**Files:**
- Modify: `crates/cogito-plugin/src/manifest.rs`
- Test: `crates/cogito-plugin/tests/loader.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-plugin/tests/loader.rs`:

```rust
use std::fs;

use cogito_plugin::PluginManifest;

#[test]
fn parses_toml_manifest_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("code-review");
    fs::create_dir_all(plugin_dir.join(".cogito-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".cogito-plugin/plugin.toml"),
        "[plugin]\nid = \"code-review\"\nversion = \"0.1.0\"\ndescription = \"x\"\n",
    )
    .unwrap();

    let m = PluginManifest::load_from_dir(&plugin_dir).unwrap();
    assert_eq!(m.id, "code-review");
    assert_eq!(m.version.as_deref(), Some("0.1.0"));
    assert_eq!(m.skills_dir, "skills");
    assert_eq!(m.mcp_file, "mcp.toml");
}

#[test]
fn falls_back_to_claude_json_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("legacy");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin/plugin.json"),
        "{ \"name\": \"legacy\", \"version\": \"2.0.0\" }",
    )
    .unwrap();

    let m = PluginManifest::load_from_dir(&plugin_dir).unwrap();
    assert_eq!(m.id, "legacy");
    assert_eq!(m.skills_dir, "skills");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cogito-plugin parses_toml_manifest_with_defaults falls_back_to_claude_json_metadata`
Expected: FAIL to compile — `PluginManifest::load_from_dir` does not exist.

- [ ] **Step 3: Implement the parsers**

Replace `crates/cogito-plugin/src/manifest.rs`:

```rust
//! Plugin manifest parsing: `.cogito-plugin/plugin.toml` primary,
//! `.claude-plugin/plugin.json` metadata-only fallback. See ADR-0021 §1.

use std::path::Path;

use serde::Deserialize;

use crate::PluginError;

/// Internal manifest model after parsing.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Globally unique plugin id (the namespace prefix).
    pub id: String,
    /// Optional semver.
    pub version: Option<String>,
    /// Optional human description.
    pub description: Option<String>,
    /// Skills directory relative to the plugin root.
    pub skills_dir: String,
    /// MCP file relative to the plugin root.
    pub mcp_file: String,
}

fn default_skills_dir() -> String {
    "skills".to_string()
}
fn default_mcp_file() -> String {
    "mcp.toml".to_string()
}

#[derive(Debug, Deserialize)]
struct TomlManifest {
    plugin: TomlPluginSection,
}

#[derive(Debug, Deserialize)]
struct TomlPluginSection {
    id: String,
    version: Option<String>,
    description: Option<String>,
    #[serde(default = "default_skills_dir")]
    skills_dir: String,
    #[serde(default = "default_mcp_file")]
    mcp_file: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeJsonManifest {
    name: String,
    version: Option<String>,
    description: Option<String>,
}

impl PluginManifest {
    /// Load a manifest from a plugin directory. Prefers
    /// `.cogito-plugin/plugin.toml`; falls back to
    /// `.claude-plugin/plugin.json` (metadata only). Both absent → error.
    ///
    /// # Errors
    /// Returns [`PluginError::Manifest`] if no manifest exists or parsing
    /// fails.
    pub fn load_from_dir(plugin_dir: &Path) -> Result<Self, PluginError> {
        let toml_path = plugin_dir.join(".cogito-plugin/plugin.toml");
        if toml_path.is_file() {
            let text = std::fs::read_to_string(&toml_path).map_err(|e| PluginError::Manifest {
                path: toml_path.clone(),
                source: Box::new(e),
            })?;
            let parsed: TomlManifest =
                toml::from_str(&text).map_err(|e| PluginError::Manifest {
                    path: toml_path.clone(),
                    source: Box::new(e),
                })?;
            return Ok(Self {
                id: parsed.plugin.id,
                version: parsed.plugin.version,
                description: parsed.plugin.description,
                skills_dir: parsed.plugin.skills_dir,
                mcp_file: parsed.plugin.mcp_file,
            });
        }

        let json_path = plugin_dir.join(".claude-plugin/plugin.json");
        if json_path.is_file() {
            let text = std::fs::read_to_string(&json_path).map_err(|e| PluginError::Manifest {
                path: json_path.clone(),
                source: Box::new(e),
            })?;
            let parsed: ClaudeJsonManifest =
                serde_json::from_str(&text).map_err(|e| PluginError::Manifest {
                    path: json_path.clone(),
                    source: Box::new(e),
                })?;
            return Ok(Self {
                id: parsed.name,
                version: parsed.version,
                description: parsed.description,
                skills_dir: default_skills_dir(),
                mcp_file: default_mcp_file(),
            });
        }

        Err(PluginError::Manifest {
            path: plugin_dir.to_path_buf(),
            source: "no .cogito-plugin/plugin.toml or .claude-plugin/plugin.json".into(),
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p cogito-plugin parses_toml_manifest_with_defaults falls_back_to_claude_json_metadata`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-plugin/src/manifest.rs crates/cogito-plugin/tests/loader.rs
git commit -m "feat(plugin): parse plugin.toml + claude-plugin json metadata fallback (ADR-0021)"
```

---

## Task 4: Discovery, namespacing, overrides, id-uniqueness → `PluginContributions`

**Files:**
- Modify: `crates/cogito-plugin/src/discovery.rs`
- Test: `crates/cogito-plugin/tests/loader.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/cogito-plugin/tests/loader.rs`:

```rust
use cogito_plugin::{ArtifactOverride, PluginEntry, PluginSet};

fn write_plugin(root: &std::path::Path, id: &str, with_mcp: bool, skill: Option<&str>) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join(".cogito-plugin")).unwrap();
    fs::write(
        dir.join(".cogito-plugin/plugin.toml"),
        format!("[plugin]\nid = \"{id}\"\n"),
    )
    .unwrap();
    if let Some(skill_name) = skill {
        let sdir = dir.join("skills").join(skill_name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            format!("---\nname: {skill_name}\ndescription: d\n---\nbody\n"),
        )
        .unwrap();
    }
    if with_mcp {
        fs::write(
            dir.join("mcp.toml"),
            "[[mcp_servers]]\nname = \"github\"\ntransport = \"stdio\"\ncommand = \"echo\"\n",
        )
        .unwrap();
    }
}

#[test]
fn loads_namespaces_and_keeps_skill_root() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "code-review", true, Some("review-rust"));
    let entries = vec![PluginEntry {
        path: "code-review".into(),
        enabled: true,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert_eq!(c.skill_roots.len(), 1);
    assert_eq!(c.skill_roots[0].plugin_id, "code-review");
    assert_eq!(c.mcp_servers.len(), 1);
    assert_eq!(c.mcp_servers[0].name, "code-review:github");
}

#[test]
fn disabled_plugin_contributes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", true, Some("s1"));
    let entries = vec![PluginEntry {
        path: "p1".into(),
        enabled: false,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert!(c.skill_roots.is_empty());
    assert!(c.mcp_servers.is_empty());
}

#[test]
fn artifact_override_disables_one_mcp_server() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", true, Some("s1"));
    let entries = vec![PluginEntry {
        path: "p1".into(),
        enabled: true,
        artifact_overrides: vec![ArtifactOverride {
            plugin: "p1".into(),
            kind: "mcp".into(),
            name: "github".into(),
            enabled: false,
        }],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert!(c.mcp_servers.is_empty());
    assert_eq!(c.skill_roots.len(), 1);
}

#[test]
fn duplicate_plugin_id_is_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(&tmp.path().join("a"), "dup", false, None);
    write_plugin(&tmp.path().join("b"), "dup", false, None);
    let entries = vec![
        PluginEntry { path: "a/dup".into(), enabled: true, artifact_overrides: vec![] },
        PluginEntry { path: "b/dup".into(), enabled: true, artifact_overrides: vec![] },
    ];
    let err = PluginSet::load(&entries, tmp.path()).unwrap_err();
    assert!(matches!(err, cogito_plugin::PluginError::DuplicateId { .. }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p cogito-plugin loads_namespaces_and_keeps_skill_root disabled_plugin_contributes_nothing artifact_override_disables_one_mcp_server duplicate_plugin_id_is_fatal`
Expected: FAIL — stub `load` returns empty contributions.

- [ ] **Step 3: Implement discovery**

Replace `crates/cogito-plugin/src/discovery.rs`:

```rust
//! Plugin discovery: resolve entries, parse manifests, namespace
//! artifacts, apply overrides, enforce id-uniqueness. See ADR-0021.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cogito_mcp::McpServerConfig;
use cogito_skills::PluginSkillRoot;
use serde::Deserialize;

use crate::manifest::PluginManifest;
use crate::{PluginContributions, PluginEntry, PluginError};

/// Resolves declared plugin entries into contributions.
pub struct PluginSet;

#[derive(Debug, Deserialize)]
struct McpFile {
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
}

impl PluginSet {
    /// Load all enabled plugins, namespacing and de-conflicting artifacts.
    ///
    /// # Errors
    /// Returns [`PluginError`] on missing paths, bad manifests, or
    /// duplicate plugin ids.
    pub fn load(
        entries: &[PluginEntry],
        config_dir: &Path,
    ) -> Result<PluginContributions, PluginError> {
        let mut out = PluginContributions::default();
        let mut seen: HashMap<String, PathBuf> = HashMap::new();

        for entry in entries {
            if !entry.enabled {
                continue;
            }
            let plugin_dir = resolve_path(config_dir, &entry.path);
            if !plugin_dir.is_dir() {
                return Err(PluginError::PathNotFound(plugin_dir));
            }

            let manifest = PluginManifest::load_from_dir(&plugin_dir)?;

            if let Some(first) = seen.insert(manifest.id.clone(), plugin_dir.clone()) {
                return Err(PluginError::DuplicateId {
                    id: manifest.id,
                    first,
                    second: plugin_dir,
                });
            }

            collect_skills(&plugin_dir, &manifest, entry, &mut out);
            collect_mcp(&plugin_dir, &manifest, entry, &mut out)?;
        }

        Ok(out)
    }
}

fn resolve_path(config_dir: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        config_dir.join(p)
    }
}

fn is_disabled(entry: &PluginEntry, plugin_id: &str, kind: &str, name: &str) -> bool {
    entry
        .artifact_overrides
        .iter()
        .any(|o| o.plugin == plugin_id && o.kind == kind && o.name == name && !o.enabled)
}

fn collect_skills(
    plugin_dir: &Path,
    manifest: &PluginManifest,
    entry: &PluginEntry,
    out: &mut PluginContributions,
) {
    let skills_dir = plugin_dir.join(&manifest.skills_dir);
    if !skills_dir.is_dir() {
        return;
    }
    // v0.2: per-skill disable is coarse. Register the root unless every
    // skill subdir is overridden off. (Finer per-skill filtering is a
    // follow-up; SkillRegistry::scan consumes a directory, not a name list.)
    let any_enabled = std::fs::read_dir(&skills_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !is_disabled(entry, &manifest.id, "skill", &name)
        });
    if any_enabled {
        out.skill_roots.push(PluginSkillRoot {
            plugin_id: manifest.id.clone(),
            dir: skills_dir,
        });
    }
}

fn collect_mcp(
    plugin_dir: &Path,
    manifest: &PluginManifest,
    entry: &PluginEntry,
    out: &mut PluginContributions,
) -> Result<(), PluginError> {
    let mcp_path = plugin_dir.join(&manifest.mcp_file);
    if !mcp_path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&mcp_path).map_err(|e| PluginError::Manifest {
        path: mcp_path.clone(),
        source: Box::new(e),
    })?;
    let parsed: McpFile = toml::from_str(&text).map_err(|e| PluginError::Manifest {
        path: mcp_path.clone(),
        source: Box::new(e),
    })?;
    for mut server in parsed.mcp_servers {
        if is_disabled(entry, &manifest.id, "mcp", &server.name) {
            continue;
        }
        server.name = format!("{}:{}", manifest.id, server.name);
        out.mcp_servers.push(server);
    }
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p cogito-plugin loads_namespaces_and_keeps_skill_root disabled_plugin_contributes_nothing artifact_override_disables_one_mcp_server duplicate_plugin_id_is_fatal`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-plugin/src/discovery.rs crates/cogito-plugin/tests/loader.rs
git commit -m "feat(plugin): discovery, namespacing, overrides, id-uniqueness (ADR-0021)"
```

---

## Task 5: `cogito-config` — `[[plugins]]` section

**Files:**
- Modify: `crates/cogito-config/Cargo.toml`, `crates/cogito-config/src/types.rs`
- Test: the config crate's test module

- [ ] **Step 1: Add the dependency**

In `crates/cogito-config/Cargo.toml` `[dependencies]`, add next to `cogito-mcp`:

```toml
cogito-plugin   = { workspace = true }
```

- [ ] **Step 2: Inspect how `mcp_servers` flows through partial/merge/finalize**

Run: `grep -n "mcp_servers\|RuntimeConfig {\|RuntimeConfigPartial\|deserialize_mcp\|try_into" crates/cogito-config/src/types.rs`
Expected: shows (a) the `RuntimeConfig.mcp_servers` field, (b) the `RuntimeConfigPartial.mcp_servers: Option<Vec<toml::Value>>` raw field, (c) the merge arm, and (d) the finalize step that turns raw `toml::Value`s into `McpServerConfig` (the `try_into` call near line ~165). Mirror each for `plugins`.

- [ ] **Step 3: Write the failing test**

In the config test module (`grep -rn "mod tests\|#\[cfg(test)\]" crates/cogito-config/src/types.rs` to find it, or the dedicated tests file the crate uses for merge), add a test mirroring the existing `mcp_servers` parse test. Use the same finalize/parse helper the neighboring tests use:

```rust
#[test]
fn parses_plugins_section() {
    // Build the same way the mcp_servers test builds a finalized RuntimeConfig
    // from a TOML string (reuse the neighboring helper).
    let toml = r#"
[[plugins]]
path = "./plugins/code-review"

[[plugins]]
path = "./plugins/sql"
enabled = false
"#;
    let cfg = finalize_from_toml_for_test(toml); // same helper used by the mcp_servers test
    assert_eq!(cfg.plugins.len(), 2);
    assert_eq!(cfg.plugins[0].path, "./plugins/code-review");
    assert!(cfg.plugins[0].enabled);
    assert!(!cfg.plugins[1].enabled);
}
```

> Replace `finalize_from_toml_for_test` with the actual helper the `mcp_servers` test uses (found in Step 2's grep). If the crate has no such helper, follow the exact construction the existing `mcp_servers` test performs.

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo nextest run -p cogito-config parses_plugins_section`
Expected: FAIL — `RuntimeConfig` has no `plugins` field.

- [ ] **Step 5: Add `plugins` to the config types**

In `crates/cogito-config/src/types.rs`:

Add the import next to `use cogito_mcp::{...}`:

```rust
use cogito_plugin::PluginEntry;
```

Add to `RuntimeConfig` (next to `mcp_servers`):

```rust
    /// Loaded plugin entries (ADR-0021).
    pub plugins: Vec<PluginEntry>,
```

Add to `RuntimeConfigPartial` (next to the raw `mcp_servers`):

```rust
    /// Raw `[[plugins]]` entries; deserialized during finalize.
    pub plugins: Option<Vec<toml::Value>>,
```

In the merge function, add a `plugins` arm mirroring the `mcp_servers` whole-array-replace arm. In `finalize`, mirror the `mcp_servers` deserialize:

```rust
        let plugins: Vec<PluginEntry> = partial
            .plugins
            .unwrap_or_default()
            .into_iter()
            .map(toml::Value::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| /* same ConfigError variant the mcp_servers finalize uses */)?;
```

Add `plugins` to the `RuntimeConfig { ... }` constructor in `finalize` and to any other `RuntimeConfig { ... }` literal (run `grep -n "RuntimeConfig {" crates/cogito-config/src/`); each construction site needs the new field.

> Match the exact `ConfigError` variant + message style used by the `mcp_servers` finalize step (found in Step 2). Do not invent a new error variant.

- [ ] **Step 6: Run the test + crate**

Run: `cargo nextest run -p cogito-config parses_plugins_section && make test CRATE=cogito-config`
Expected: PASS. If a pre-existing test asserts `[[plugins]]` is *ignored* (reserved-section test), update it to assert the entries now parse (`grep -n "plugins" crates/cogito-config` to find it).

- [ ] **Step 7: Commit**

```bash
git add crates/cogito-config/Cargo.toml crates/cogito-config/src/types.rs
git commit -m "feat(config): [[plugins]] section parsed into RuntimeConfig (ADR-0021)"
```

---

## Task 6: CLI wiring — fold contributions into providers

**Files:**
- Modify: `crates/cogito-cli/Cargo.toml`, `crates/cogito-cli/src/chat_config.rs`, `crates/cogito-cli/src/chat.rs`

- [ ] **Step 1: Add the dependency**

In `crates/cogito-cli/Cargo.toml` `[dependencies]`, add:

```toml
cogito-plugin = { workspace = true }
```

- [ ] **Step 2: Load contributions once in `run`**

In `crates/cogito-cli/src/chat.rs`, inside `run`, after the config is loaded (where `cfg` is in scope, before `build_tool_provider`/`build_skill_provider` are called ~line 613), add:

```rust
    let config_dir = std::env::current_dir().context("reading current dir for plugin load")?;
    let contributions = cogito_plugin::PluginSet::load(&cfg.plugins, &config_dir)
        .map_err(|e| anyhow!("loading plugins: {e}"))?;
```

> `current_dir()` matches the existing `build_skill_provider` workspace-root choice, keeping skill discovery consistent. (`anyhow`/`Context` are already imported in `chat.rs`.)

- [ ] **Step 3: Fold plugin skill roots into the skill scan**

In `crates/cogito-cli/src/chat_config.rs`, change `build_skill_provider` to accept plugin roots and add them to the `ScanConfig` literal (which now has a `plugin_roots` field from Task 1):

```rust
pub fn build_skill_provider(
    cfg: &RuntimeConfig,
    plugin_roots: Vec<cogito_skills::PluginSkillRoot>,
) -> Result<Option<Arc<dyn SkillProvider>>> {
    let enabled = cfg.skills.as_ref().is_none_or(|s| s.enabled);
    if !enabled {
        return Ok(None);
    }
    let section = cfg.skills.clone().unwrap_or_default();
    let scan = cogito_skills::discovery::ScanConfig {
        workspace_root: Some(
            std::env::current_dir().context("reading current dir for skill scan")?,
        ),
        user_dir: section
            .user_dir
            .map(PathBuf::from)
            .or_else(default_user_skills_dir),
        include_system: section.include_system,
        plugin_roots,
    };
    let registry =
        cogito_skills::SkillRegistry::scan(scan).map_err(|e| anyhow!("scanning skills: {e}"))?;
    Ok(Some(Arc::new(registry) as Arc<dyn SkillProvider>))
}
```

Update the call site in `chat.rs` (~line 614):

```rust
    let skills = crate::chat_config::build_skill_provider(&cfg, contributions.skill_roots)?;
```

- [ ] **Step 4: Fold plugin MCP servers into the tool provider**

In `chat.rs`, change `build_tool_provider` to take the plugin MCP servers and concatenate them before `build_mcp_provider`. Update its signature:

```rust
async fn build_tool_provider(
    cfg: &RuntimeConfig,
    job_mgr: Arc<LocalJobManager>,
    plugin_mcp_servers: Vec<cogito_mcp::McpServerConfig>,
) -> Result<Arc<dyn ToolProvider>> {
```

Replace the `cogito_mcp::build_mcp_provider(&cfg.mcp_servers)` call with:

```rust
    let mut mcp_cfgs = cfg.mcp_servers.clone();
    mcp_cfgs.extend(plugin_mcp_servers);
    let mcp_build = cogito_mcp::build_mcp_provider(&mcp_cfgs).await;
```

Update the call site (~line 613):

```rust
    let tools = build_tool_provider(&cfg, Arc::clone(&job_mgr), contributions.mcp_servers).await?;
```

> Single-tenant CLI folds contributions into the Runtime's default providers — no `open_session_with` needed here. Per-session injection / `update_session` is the consumer-server path, exercised by Plan 1's chaos test and Task 7's contribution test.

- [ ] **Step 5: Build the CLI**

Run: `cargo build -p cogito-cli`
Expected: builds clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-cli/Cargo.toml crates/cogito-cli/src/chat.rs crates/cogito-cli/src/chat_config.rs
git commit -m "feat(cli): load plugins and fold skills+mcp into chat providers (ADR-0021)"
```

---

## Task 7: Integration / acceptance test

**Files:**
- Test: `crates/cogito-plugin/tests/integration.rs`

ADR-0021 acceptance, honest about the live-MCP caveat (no in-process MCP server exists, so MCP is asserted at the contribution/fold level; the skill is asserted end-to-end through the real `SkillRegistry`).

- [ ] **Step 1: Write the integration test**

Create `crates/cogito-plugin/tests/integration.rs`:

```rust
//! ADR-0021 acceptance: a local plugin contributes a skill and an MCP
//! server; both are namespaced and foldable. The skill is verified
//! end-to-end through the real SkillRegistry. A mid-session "add a
//! second plugin" recomposition (the input update_session would receive)
//! is verified at the contribution level.

use std::fs;

use cogito_plugin::{PluginEntry, PluginSet};

fn write_plugin(root: &std::path::Path, id: &str, server: &str, skill: &str) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join(".cogito-plugin")).unwrap();
    fs::write(dir.join(".cogito-plugin/plugin.toml"), format!("[plugin]\nid = \"{id}\"\n")).unwrap();
    let sdir = dir.join("skills").join(skill);
    fs::create_dir_all(&sdir).unwrap();
    fs::write(sdir.join("SKILL.md"), format!("---\nname: {skill}\ndescription: d\n---\nbody\n")).unwrap();
    fs::write(
        dir.join("mcp.toml"),
        format!("[[mcp_servers]]\nname = \"{server}\"\ntransport = \"stdio\"\ncommand = \"echo\"\n"),
    )
    .unwrap();
}

#[test]
fn plugin_skill_and_mcp_are_contributed_and_namespaced() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "code-review", "github", "review-rust");

    let entries = vec![PluginEntry {
        path: "code-review".into(),
        enabled: true,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();

    let mcp_names: Vec<_> = c.mcp_servers.iter().map(|s| s.name.clone()).collect();
    assert_eq!(mcp_names, vec!["code-review:github"]);

    // Skill reachable end-to-end through the real SkillRegistry (by value).
    let cfg = cogito_skills::discovery::ScanConfig {
        workspace_root: None,
        user_dir: None,
        include_system: false,
        plugin_roots: c.skill_roots,
    };
    let registry = cogito_skills::SkillRegistry::scan(cfg).unwrap();
    use cogito_protocol::skill::SkillProvider;
    assert!(registry.is_registered("code-review:review-rust"));
}

#[test]
fn mid_session_add_appends_second_plugin_mcp() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", "srv1", "s1");
    write_plugin(tmp.path(), "p2", "srv2", "s2");

    let c1 = PluginSet::load(
        &[PluginEntry { path: "p1".into(), enabled: true, artifact_overrides: vec![] }],
        tmp.path(),
    )
    .unwrap();
    let c2 = PluginSet::load(
        &[
            PluginEntry { path: "p1".into(), enabled: true, artifact_overrides: vec![] },
            PluginEntry { path: "p2".into(), enabled: true, artifact_overrides: vec![] },
        ],
        tmp.path(),
    )
    .unwrap();

    let before: Vec<_> = c1.mcp_servers.iter().map(|s| s.name.clone()).collect();
    let after: Vec<_> = c2.mcp_servers.iter().map(|s| s.name.clone()).collect();
    assert_eq!(before, vec!["p1:srv1"]);
    assert_eq!(after, vec!["p1:srv1", "p2:srv2"]);
}
```

> Keeping these in `cogito-plugin` (not `cogito-core`) avoids a plugin→core test dependency, preserving layer discipline. The live `update_session` swap is proven by Plan 1's `session_spec_mutated_then_resume`.

- [ ] **Step 2: Run the integration tests**

Run: `cargo nextest run -p cogito-plugin plugin_skill_and_mcp_are_contributed_and_namespaced mid_session_add_appends_second_plugin_mcp`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-plugin/tests/integration.rs
git commit -m "test(plugin): acceptance — skill+mcp contributed, namespaced, mid-session add (ADR-0021)"
```

---

## Task 8: Final verification + CI gate

**Files:** none (verification only)

- [ ] **Step 1: Format + lint the touched crates**

Run: `make fmt && make fix CRATE=cogito-plugin && make fix CRATE=cogito-skills && make fix CRATE=cogito-config && make fix CRATE=cogito-cli`
Expected: clean; no `unwrap`/`expect`/`panic` in non-test code.

- [ ] **Step 2: Test each touched crate**

Run: `make test CRATE=cogito-plugin && make test CRATE=cogito-skills && make test CRATE=cogito-config && make test CRATE=cogito-cli`
Expected: all PASS.

- [ ] **Step 3: Workspace CI (includes layer-check)**

Run: `make ci`
Expected: green. Layer-check confirms `cogito-plugin` imports only `cogito-protocol` / `cogito-mcp` / `cogito-skills`, and `cogito-core` gained no dependency on `cogito-plugin`.

- [ ] **Step 4: Commit any fmt-only changes**

```bash
git add -A
git commit -m "chore: fmt/clippy after ADR-0021 plugin loader" || echo "nothing to commit"
```

---

## Self-review notes (for the executor)

- **Spec coverage (ADR-0021):** §1 manifest → Task 3; §2 Skills+MCP scope → Tasks 1/4/6; §3 namespacing → Task 4 (MCP) + Task 1 (skills); §4 enable/disable + overrides → Task 4; §5 id-uniqueness fatal → Task 4; §6 local-path-only → Task 4 `resolve_path`; §7 contributions → Tasks 1/4; §8 wiring → Task 6; §9 crate layout → Task 2.
- **Ordering rationale:** `PluginSkillRoot` lives in `cogito-skills` (Task 1) and is imported by `cogito-plugin` (Task 2) — leaf change first prevents a dangling import.
- **Verified identifiers:** `SkillRegistry::scan(ScanConfig)` by value; `discover_skills(&ScanConfig)`; `scan_skills_dir(dir, &SkillSource, &mut out)` over `<name>/SKILL.md` child dirs; `SkillSource::Plugin { plugin_id }`; `McpServerConfig.name` + `#[serde(flatten)] transport`; CLI `build_skill_provider`/`build_tool_provider` shapes.
- **Dependency edges added:** `cogito-config → cogito-plugin`, `cogito-plugin → cogito-mcp`/`cogito-skills`. No cycle; `cogito-core` untouched.
- **Known v0.2 simplification (Task 4):** per-skill disable is coarse (root-level); per-MCP disable + whole-plugin disable are exact. Documented in `collect_skills`.
- **Live-MCP caveat:** MCP asserted at fold level (Task 7), consistent with the still-deferred Sprint-4 live-server acceptance test.
```

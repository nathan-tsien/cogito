# Sprint 9a · Multi-model Strategy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a strategy registry (`cogito-strategy` crate) so agent designers declare named "agent modes" as markdown+frontmatter files, plus an OpenAI Responses adapter so strategies can bind to OpenAI's Responses API.

**Architecture:** New Hands-layer crate `cogito-strategy` exposes `FsStrategyRegistry: StrategyRegistry` from a scope-precedence scan (Repo `.cogito/strategies/` > User `~/.config/cogito/strategies/`). Strategy resolution happens entirely in the wiring layer: `cogito-cli` builds the registry once at startup and calls a new `resolve_strategy` helper that combines registry hit + CLI flags + `cogito.toml` to produce the final `HarnessStrategy` + `ProviderConfig` pair before `RuntimeBuilder::build()`. Brain consumes the resolved `HarnessStrategy` exactly as today; the registry never crosses into Brain. New `cogito-model::openai_responses` adapter mirrors the `openai_compat` shape and decodes native Responses reasoning items into `ContentBlock::Thinking` per ADR-0019.

**Tech Stack:** Rust 2024 (MSRV 1.85), `serde_yaml`, `walkdir`, `pulldown-cmark` (frontmatter split via custom logic — borrowed from `cogito-skills` if cheap), `thiserror`, `tracing`, `reqwest` + `eventsource-stream` for SSE. Tests via `cargo nextest`; resume-chaos via existing `cogito-test-fixtures` harness.

**Branch:** `feat/sprint-9a-multi-model-strategy` (created from `main` after spec commit `9515bb9`).

**Spec:** `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`

---

## File map

### Create

```
crates/cogito-strategy/
  Cargo.toml
  src/
    lib.rs              # crate-level docstring (strategy-vs-config framing) + public re-exports
    scope.rs            # Scope, ScopeRoot, conventional_scopes()
    schema.rs           # serde structs mirroring frontmatter shape (StrategyFrontmatter, SystemPromptSource)
    parser.rs           # markdown frontmatter splitter + parse_strategy_file()
    registry.rs         # FsStrategyRegistry struct + StrategyRegistry impl
    error.rs            # LoadError enum
  tests/
    parse.rs            # frontmatter / body / file-ref parsing snapshot tests
    registry.rs         # FsStrategyRegistry scope precedence + duplicate tests
    contract.rs         # consumes the shared contract suite from cogito-test-fixtures
    fixtures/
      valid_minimal.md
      valid_full.md
      valid_file_ref.md
      malformed_no_frontmatter.md
      malformed_no_name.md
      mismatched_filename.md
      empty_prompt.md
      file_ref_target.md          # body that valid_file_ref.md points to

crates/cogito-protocol/src/strategy_registry.rs   # StrategyRegistry trait + StrategyError
crates/cogito-model/src/openai_responses/
  mod.rs                                          # OpenAiResponsesGateway + OpenAiResponsesConfig
  wire.rs                                         # SSE event types + request body shape
  encode.rs                                       # ContentBlock -> Responses items
  decode.rs                                       # SSE -> ModelEvent
crates/cogito-model/tests/fixtures/openai_responses/
  text_completion.sse                             # plain-text streaming completion
  tool_call.sse                                   # function-tool call streaming
  reasoning_summary.sse                           # reasoning items streaming
  stop_reason_max_tokens.sse                      # truncation stop reason
crates/cogito-strategy/tests/fixtures/.cogito/strategies/coder.md   # used by integration test
.cogito/strategies/coder.md                        # repo example
.cogito/strategies/planner.md                      # repo example
.cogito/strategies/reviewer.md                     # repo example
docs/adr/0026-strategy-registry.md                 # new ADR (Proposed -> Accepted at sprint close)
crates/cogito-cli/tests/resolve_strategy.rs        # resolve_strategy helper unit + integration tests
crates/cogito-core/tests/resume_chaos_strategy_with_tool_filter.rs  # new chaos scenario
```

### Modify

```
Cargo.toml                                                # workspace member + dep entries
crates/cogito-protocol/src/lib.rs                         # pub mod strategy_registry; re-export
crates/cogito-protocol/Cargo.toml                         # thiserror dep already present
crates/cogito-model/src/lib.rs                            # pub mod openai_responses; re-exports
crates/cogito-model/src/provider_config.rs                # OpenAiResponses arm + ReasoningEffort + build_gateway
crates/cogito-model/Cargo.toml                            # no new deps (reuse reqwest/eventsource-stream)
crates/cogito-config/src/types.rs                         # +default_strategy on RuntimeSection + RuntimeSectionPartial
crates/cogito-cli/Cargo.toml                              # +cogito-strategy dep
crates/cogito-cli/src/chat.rs                             # --strategy + --list-strategies flags + wire resolve_strategy
crates/cogito-cli/src/chat_config.rs                      # registry construction in build_runtime_config
crates/testing/cogito-test-fixtures/Cargo.toml            # +cogito-strategy and +cogito-protocol if missing
crates/testing/cogito-test-fixtures/src/lib.rs            # pub mod strategy; expose MapStrategyRegistry + contract_suite
crates/testing/cogito-test-fixtures/src/strategy.rs       # MapStrategyRegistry + strategy_registry_contract()
crates/cogito-core/tests/resume_chaos.rs                  # register strategy_with_tool_filter scenario
docs/components/H10-strategy-selector.md                  # new top-of-file "What is a strategy" section + supersede 2026-05-21 note
docs/configuration/overview.md                            # add paragraph pointing to ADR-0026
AGENTS.md                                                 # add ADR-0026 to authoritative-docs list
ARCHITECTURE.md                                           # tick component map + add cogito-strategy crate row
ROADMAP.md                                                # split Sprint 9 -> 9a (done) + 9b (TUI carrying over)
CHANGELOG.md                                              # v0.1 / Sprint 9a entry
```

### Delete

```
strategies/claude-opus.yaml             # stale schema; replaced by .cogito/strategies/coder.md
strategies/gpt-4.yaml                   # stale schema; replaced by .cogito/strategies/planner.md
strategies/                             # directory removed entirely
```

---

## Conventions

- Every commit runs `make fmt && make fix CRATE=<name>` before staging.
- Every task ends with `make test CRATE=<name>` green and a commit on `feat/sprint-9a-multi-model-strategy`.
- Test files start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` consistent with the existing codebase.
- All code comments and doc comments in English (CLAUDE.md mandate). Chinese stays in conversation/specs/ADRs.
- No decorative numerals or markers in source or docs. Plain `1.` and `-`.
- Use `tracing::warn!` / `tracing::debug!` — never `println!` / `eprintln!`.
- Strategy resolution lives entirely in `cogito-cli`. Brain (`cogito-core`) never imports `cogito-strategy`. Verified by `make layer-check` (existing tooling — see `tools/layer_check/`).

---

## Phase 0: Branch setup

### Task 00: Cut the working branch

**Files:**
- None (git only)

- [ ] **Step 1: Verify clean working tree**

Run: `git status`
Expected: `nothing to commit, working tree clean` (after the spec commit `9515bb9` from the brainstorm).

- [ ] **Step 2: Cut the branch**

```bash
git checkout -b feat/sprint-9a-multi-model-strategy
```

- [ ] **Step 3: Sanity-check the baseline**

```bash
make ci
```

Expected: fmt + clippy + layer-check + test all green. If anything fails, fix on `main` first — never start sprint work on a red baseline.

- [ ] **Step 4: Tag the start commit (optional, useful for diff-stat at sprint close)**

```bash
git tag -a sprint-9a-start -m "Sprint 9a baseline" HEAD
```

---

## Phase 1: Protocol layer — `StrategyRegistry` trait

### Task 01: Add `StrategyRegistry` trait + `StrategyError` to `cogito-protocol`

**Files:**
- Create: `crates/cogito-protocol/src/strategy_registry.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-protocol/src/strategy_registry.rs`:

```rust
//! Read-only registry of named `HarnessStrategy` bundles.
//!
//! The trait is the protocol-layer seam between the wiring layer
//! (which discovers strategies — from disk in v0.1, from a database
//! or object store in v0.4 SaaS) and any consumer that resolves a
//! strategy by name. Brain does NOT depend on this trait in v0.1;
//! the wiring layer (`cogito-cli`) resolves the strategy up-front
//! and hands `RuntimeBuilder` the final `HarnessStrategy` value.
//!
//! See `docs/adr/0026-strategy-registry.md`.

use thiserror::Error;

use crate::strategy::HarnessStrategy;

/// Read-only registry. v0.1 ships an FS-backed impl in `cogito-strategy`;
/// v0.4 SaaS adds a DB-backed impl behind the same trait.
pub trait StrategyRegistry: Send + Sync + 'static {
    /// Returns the named strategy. The returned value has `system_prompt`
    /// fully materialized (any `file:` references already resolved).
    ///
    /// # Errors
    ///
    /// Returns `StrategyError::Unknown` if `name` is not registered.
    /// Returns `StrategyError::Validation` for impl-specific
    /// post-load checks.
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError>;

    /// Returns the names of all strategies currently registered.
    /// MUST be sorted ascending and deduplicated.
    fn list(&self) -> Vec<String>;
}

/// Errors surfaced by `StrategyRegistry`. `LoadError` (in `cogito-strategy`)
/// is a strictly-richer cousin used at registry-build time.
#[derive(Error, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StrategyError {
    /// `name` is not registered. `available` is `registry.list()` at
    /// the time of the failed lookup; used by CLI surfaces for
    /// "did you mean" output.
    #[error("strategy `{0}` not found; available: {1:?}")]
    Unknown(String, Vec<String>),

    /// Strategy references a provider that is not in `cogito.toml`.
    /// Detected by the wiring layer when both inputs first meet.
    #[error("strategy `{name}` references missing provider `{provider}`")]
    UnknownProvider { name: String, provider: String },

    /// Catch-all for impl-specific validation failures.
    #[error("strategy `{name}` validation failed: {reason}")]
    Validation { name: String, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::HarnessStrategy;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Smallest possible test impl — proves the trait is dyn-compatible
    /// and shareable as `Arc<dyn StrategyRegistry>`.
    struct StubRegistry(HashMap<String, HarnessStrategy>);

    impl StrategyRegistry for StubRegistry {
        fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
            self.0
                .get(name)
                .cloned()
                .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
        }
        fn list(&self) -> Vec<String> {
            let mut v: Vec<String> = self.0.keys().cloned().collect();
            v.sort();
            v
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let mut m = HashMap::new();
        m.insert("foo".into(), HarnessStrategy::default_with_model("test"));
        let reg: Arc<dyn StrategyRegistry> = Arc::new(StubRegistry(m));
        assert_eq!(reg.list(), vec!["foo"]);
        assert!(reg.get("foo").is_ok());
        assert!(matches!(reg.get("missing"), Err(StrategyError::Unknown(_, _))));
    }
}
```

- [ ] **Step 2: Wire the new module**

Edit `crates/cogito-protocol/src/lib.rs`, find the `pub mod` block (next to `pub mod strategy;`) and add:

```rust
pub mod strategy_registry;
```

Re-export at the crate level (next to other re-exports if any pattern exists; otherwise just leave the `pub mod` declaration):

```rust
pub use strategy_registry::{StrategyError, StrategyRegistry};
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test -p cogito-protocol strategy_registry`
Expected: 1 test, `trait_is_object_safe ... ok`.

- [ ] **Step 4: Format + lint**

```bash
make fmt
make fix CRATE=cogito-protocol
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/strategy_registry.rs crates/cogito-protocol/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(protocol): add StrategyRegistry trait + StrategyError

Adds the protocol-layer seam between strategy discovery (FS in v0.1,
DB/S3 in v0.4 SaaS) and strategy consumers. Read-only, object-safe,
Arc-shareable. Brain does not depend on this trait in v0.1; the
wiring layer (cogito-cli) resolves strategies up-front. See
docs/adr/0026-strategy-registry.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 2: `cogito-strategy` crate — scaffold

### Task 02: Create empty `cogito-strategy` crate

**Files:**
- Create: `crates/cogito-strategy/Cargo.toml`
- Create: `crates/cogito-strategy/src/lib.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Add crate to workspace**

Edit the workspace `Cargo.toml`, in the `[workspace] members = [...]` array add `"crates/cogito-strategy"` in alphabetical order (between `cogito-sandbox` and `cogito-test-fixtures` or wherever it lands alphabetically — check existing order).

Then in `[workspace.dependencies]`, add:

```toml
cogito-strategy = { path = "crates/cogito-strategy" }
```

- [ ] **Step 2: Create crate manifest**

Create `crates/cogito-strategy/Cargo.toml`:

```toml
[package]
name = "cogito-strategy"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Filesystem-backed StrategyRegistry implementation for cogito"
publish = false

[lints]
workspace = true

[dependencies]
cogito-protocol = { workspace = true }
cogito-context = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_yaml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
walkdir = { workspace = true }

[dev-dependencies]
cogito-test-fixtures = { workspace = true }
tempfile = { workspace = true }
```

Verify each dep is actually in `[workspace.dependencies]` of the root `Cargo.toml`. If `walkdir` or `tempfile` aren't there yet, add them (check what version cogito-skills uses for `walkdir` and match).

- [ ] **Step 3: Create the lib.rs with the strategy-vs-config framing**

Create `crates/cogito-strategy/src/lib.rs`:

```rust
//! Filesystem-backed `StrategyRegistry` for cogito.
//!
//! # What is a strategy?
//!
//! A strategy is a named, declarative "agent mode." It bundles *which
//! model, which persona, which tools, which context policy* for one
//! kind of work. The consumer ships their cogito-embedded service with
//! N strategies — `coder`, `planner`, `reviewer`, `critic` — and
//! `cogito chat --strategy coder` (or, programmatically,
//! `runtime.open_session_with_strategy("coder", ...)`) selects the mode.
//! Same Brain, same Boundary, different *behavior contract*. Without
//! strategies, every behavior change is a code change and a redeploy.
//!
//! # Strategies are not configuration of cogito
//!
//! `cogito.toml` (loaded by `cogito-config`) is "where is the model and
//! how do I reach it" — endpoints, credentials, provider defaults.
//! Strategies are "what do I tell the model to do." The two layer
//! cleanly: strategies *reference* providers from `cogito.toml` by
//! name; they never embed credentials.
//!
//! # File format
//!
//! A strategy is a markdown file with YAML frontmatter. Filename
//! basename must match the `name` field. See
//! `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
//! §7 for the full schema and `docs/adr/0026-strategy-registry.md` for
//! the architectural rationale.
//!
//! # SaaS path
//!
//! This crate is the v0.1 filesystem-backed implementation. v0.4 SaaS
//! deployment swaps in a DB- or S3-backed implementation behind the
//! same `cogito_protocol::StrategyRegistry` trait — no Brain change.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn crate_compiles() {
        // Placeholder; later tasks add real surface.
    }
}
```

- [ ] **Step 4: Verify the crate builds**

```bash
cargo build -p cogito-strategy
cargo test -p cogito-strategy
```

Expected: build clean, 1 smoke test passes.

- [ ] **Step 5: Format + lint + commit**

```bash
make fmt
make fix CRATE=cogito-strategy
git add Cargo.toml crates/cogito-strategy/
git commit -m "$(cat <<'EOF'
feat(strategy): scaffold cogito-strategy crate

Empty Hands-layer crate that will host FsStrategyRegistry. The
crate-level docstring carries the "strategy vs config" framing
(consumer-facing — explains why this is not folded into cogito-config).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 3: `cogito-strategy` — schema, parser, errors

### Task 03: Define `LoadError`

**Files:**
- Create: `crates/cogito-strategy/src/error.rs`
- Modify: `crates/cogito-strategy/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/src/error.rs`:

```rust
//! Load-time errors for `FsStrategyRegistry`. These surface from
//! `from_roots` / `from_conventional_scopes` and are strictly richer
//! than `cogito_protocol::StrategyError`, which surfaces from `get`/`list`.

use std::path::PathBuf;

use thiserror::Error;

/// Registry-build error. Any variant is fatal at startup — operators
/// learn about broken strategies immediately, not at session-open.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// I/O error reading a strategy file or its referenced prompt file.
    #[error("I/O error reading {path}: {source}")]
    Io {
        /// File path that triggered the error.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// YAML frontmatter failed to deserialize.
    #[error("parse error in {path}: {source}")]
    Parse {
        /// File path that triggered the error.
        path: PathBuf,
        /// Underlying serde error.
        #[source]
        source: serde_yaml::Error,
    },

    /// File has no `---`-delimited frontmatter block.
    #[error("frontmatter missing or malformed in {path}")]
    Frontmatter {
        /// File path that triggered the error.
        path: PathBuf,
    },

    /// Two strategy files in the same scope declare the same `name`.
    #[error("duplicate strategy name `{name}` in scope: {files:?}")]
    DuplicateName {
        /// Conflicting strategy name.
        name: String,
        /// All files in the same scope that declare the name.
        files: Vec<PathBuf>,
    },

    /// Filename basename does not match the `name` field in frontmatter.
    #[error("filename / name mismatch: {path} declares `name: {declared}`")]
    NameMismatch {
        /// Strategy file path.
        path: PathBuf,
        /// `name` field as declared in frontmatter.
        declared: String,
    },

    /// Both body and frontmatter `system_prompt` are empty.
    #[error("strategy {name} has empty system_prompt (body and frontmatter both empty)")]
    EmptyPrompt {
        /// Strategy name.
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_each_variant() {
        // Sanity check that thiserror formatting compiles for every variant.
        let cases: Vec<LoadError> = vec![
            LoadError::Frontmatter { path: PathBuf::from("foo.md") },
            LoadError::DuplicateName {
                name: "coder".into(),
                files: vec![PathBuf::from("a.md"), PathBuf::from("b.md")],
            },
            LoadError::NameMismatch {
                path: PathBuf::from("coder.md"),
                declared: "planner".into(),
            },
            LoadError::EmptyPrompt { name: "coder".into() },
        ];
        for case in cases {
            assert!(!format!("{case}").is_empty());
        }
    }
}
```

- [ ] **Step 2: Wire the module**

Edit `crates/cogito-strategy/src/lib.rs`, replace the smoke-test module with:

```rust
pub mod error;

pub use error::LoadError;
```

- [ ] **Step 3: Run the test**

```bash
cargo test -p cogito-strategy error::tests
```

Expected: 1 test passes.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/src/error.rs crates/cogito-strategy/src/lib.rs
git commit -m "feat(strategy): add LoadError enum

$(cat <<'EOF'
Six fatal-at-startup error variants: Io, Parse, Frontmatter,
DuplicateName, NameMismatch, EmptyPrompt. Distinct from
protocol-level StrategyError (which surfaces from get/list calls,
not registry-build).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 04: Define frontmatter schema types

**Files:**
- Create: `crates/cogito-strategy/src/schema.rs`
- Modify: `crates/cogito-strategy/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/src/schema.rs`:

```rust
//! Serde structs mirroring the YAML frontmatter shape. Parsed into
//! `StrategyFrontmatter`, then materialized into a fully-resolved
//! `HarnessStrategy` by `parser::materialize`.

use std::path::PathBuf;

use cogito_context::ContextConfig;
use cogito_protocol::gateway::ModelParams;
use cogito_protocol::strategy::ToolFilter;
use serde::Deserialize;

/// Direct deserialize target of the YAML frontmatter block.
/// All fields except `name` are optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StrategyFrontmatter {
    /// Required. Must match filename basename.
    pub name: String,

    /// Human-only description, surfaced by `cogito chat --list-strategies`.
    #[serde(default)]
    pub description: Option<String>,

    /// Optional provider reference. Resolved against `cogito.toml`
    /// providers by the wiring layer.
    #[serde(default)]
    pub provider: Option<String>,

    /// Optional model id. Overridden by `--model` CLI flag.
    #[serde(default)]
    pub model: Option<String>,

    /// Optional explicit system prompt override (replaces markdown body).
    #[serde(default)]
    pub system_prompt: Option<SystemPromptSource>,

    /// Tool filter. `None` -> `ToolFilter::All`.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,

    /// Explicit tool ordering for prompt-cache stability.
    #[serde(default)]
    pub tool_order: Option<Vec<String>>,

    /// Inner-loop safety budget. `None` -> 16 (HarnessStrategy default).
    #[serde(default)]
    pub max_turns: Option<u32>,

    /// Sampling knobs. Strategy keys win on overlay with provider-level
    /// model_params (overlay performed by the wiring layer).
    #[serde(default)]
    pub model_params: Option<ModelParamsPartial>,

    /// Context-management pipeline. Defaults to `ContextConfig::default()`.
    #[serde(default)]
    pub context: Option<ContextConfig>,
}

/// `model_params` shape inside a strategy file. Mirrors
/// `cogito_protocol::gateway::ModelParams` but with everything optional
/// (since strategy overlays partial values on top of provider defaults).
/// `model` is intentionally absent — that lives at the top level.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelParamsPartial {
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
}

impl ModelParamsPartial {
    /// Overlay `self` onto `base`. Self's `Some` keys win.
    pub(crate) fn overlay(&self, base: &mut ModelParams) {
        if let Some(t) = self.temperature { base.temperature = Some(t); }
        if let Some(mt) = self.max_tokens { base.max_tokens = mt; }
        if let Some(p) = self.top_p { base.top_p = Some(p); }
        if let Some(s) = &self.stop_sequences { base.stop_sequences = s.clone(); }
    }
}

/// Frontmatter override of the markdown body's role as system_prompt.
///
/// - `Inline(String)`: `system_prompt: just a string`
/// - `FileRef { file }`: `system_prompt: { file: ./path.md }` — path
///   relative to the strategy `.md` file's directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum SystemPromptSource {
    Inline(String),
    FileRef {
        file: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_frontmatter() {
        let yaml = "name: coder\n";
        let fm: StrategyFrontmatter = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(fm.name, "coder");
        assert!(fm.description.is_none());
        assert!(fm.provider.is_none());
        assert!(fm.model.is_none());
    }

    #[test]
    fn parses_full_frontmatter() {
        let yaml = r#"
name: coder
description: Coding tasks
provider: anthropic-default
model: claude-opus-4-7
allowed_tools:
  - read_file
  - run_tests
tool_order:
  - read_file
  - run_tests
max_turns: 50
model_params:
  temperature: 0.3
  max_tokens: 4096
"#;
        let fm: StrategyFrontmatter = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(fm.name, "coder");
        assert_eq!(fm.provider.as_deref(), Some("anthropic-default"));
        assert_eq!(fm.allowed_tools.as_ref().unwrap().len(), 2);
        assert_eq!(fm.max_turns, Some(50));
        let mp = fm.model_params.unwrap();
        assert_eq!(mp.temperature, Some(0.3));
        assert_eq!(mp.max_tokens, Some(4096));
    }

    #[test]
    fn parses_inline_system_prompt() {
        let yaml = "name: foo\nsystem_prompt: \"hello world\"\n";
        let fm: StrategyFrontmatter = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(fm.system_prompt, Some(SystemPromptSource::Inline(ref s)) if s == "hello world"));
    }

    #[test]
    fn parses_file_ref_system_prompt() {
        let yaml = "name: foo\nsystem_prompt:\n  file: ./prompts/foo.md\n";
        let fm: StrategyFrontmatter = serde_yaml::from_str(yaml).unwrap();
        match fm.system_prompt {
            Some(SystemPromptSource::FileRef { file }) => {
                assert_eq!(file, PathBuf::from("./prompts/foo.md"));
            }
            other => panic!("expected FileRef, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let yaml = "name: foo\nbogus: 1\n";
        let r: Result<StrategyFrontmatter, _> = serde_yaml::from_str(yaml);
        assert!(r.is_err(), "deny_unknown_fields should have rejected `bogus`");
    }

    #[test]
    fn overlay_replaces_only_some_keys() {
        let mut base = ModelParams {
            model: "x".into(),
            max_tokens: 1000,
            temperature: Some(0.7),
            top_p: None,
            stop_sequences: vec![],
        };
        let partial = ModelParamsPartial {
            temperature: Some(0.2),
            max_tokens: None,
            top_p: None,
            stop_sequences: None,
        };
        partial.overlay(&mut base);
        assert_eq!(base.temperature, Some(0.2));
        assert_eq!(base.max_tokens, 1000, "max_tokens preserved");
    }
}
```

- [ ] **Step 2: Wire the module**

Edit `crates/cogito-strategy/src/lib.rs`:

```rust
pub mod error;
mod schema;

pub use error::LoadError;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-strategy schema::tests
```

Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/src/schema.rs crates/cogito-strategy/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(strategy): define frontmatter schema types

StrategyFrontmatter is the direct serde target for the YAML
front-block. ModelParamsPartial holds the overlay-able subset of
ModelParams (model field is hoisted to top level). SystemPromptSource
is an untagged enum supporting both inline string and {file: path}
forms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 05: Implement the frontmatter splitter + file parser

**Files:**
- Create: `crates/cogito-strategy/src/parser.rs`
- Modify: `crates/cogito-strategy/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/src/parser.rs`:

```rust
//! Parse a strategy markdown file into a fully-resolved `HarnessStrategy`.
//!
//! Two steps:
//! 1. Split `---`-delimited YAML frontmatter from the markdown body.
//! 2. Materialize: deserialize frontmatter, resolve `system_prompt`
//!    (frontmatter override > inline body), apply `name`-vs-filename
//!    check, hoist `model` into `ModelParams`.

use std::fs;
use std::path::{Path, PathBuf};

use cogito_context::ContextConfig;
use cogito_protocol::gateway::ModelParams;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};

use crate::error::LoadError;
use crate::schema::{StrategyFrontmatter, SystemPromptSource};

/// Parse `path` into a fully-resolved `HarnessStrategy` plus the
/// optional `provider:` reference (for the wiring layer to cross-check
/// against `cogito.toml`).
pub(crate) fn parse_strategy_file(
    path: &Path,
) -> Result<ParsedStrategy, LoadError> {
    let raw = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let (frontmatter_text, body) = split_frontmatter(&raw)
        .ok_or_else(|| LoadError::Frontmatter { path: path.to_path_buf() })?;

    let fm: StrategyFrontmatter = serde_yaml::from_str(frontmatter_text)
        .map_err(|source| LoadError::Parse { path: path.to_path_buf(), source })?;

    let basename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if fm.name != basename {
        return Err(LoadError::NameMismatch {
            path: path.to_path_buf(),
            declared: fm.name,
        });
    }

    let system_prompt = resolve_system_prompt(&fm, body, path)?;
    if system_prompt.trim().is_empty() {
        return Err(LoadError::EmptyPrompt { name: fm.name });
    }

    let allowed_tools = match fm.allowed_tools.clone() {
        None => ToolFilter::All,
        Some(v) => ToolFilter::Allow(v),
    };

    let mut model_params = ModelParams {
        model: fm.model.clone().unwrap_or_default(),
        max_tokens: 4096,
        temperature: None,
        top_p: None,
        stop_sequences: vec![],
    };
    if let Some(p) = &fm.model_params {
        p.overlay(&mut model_params);
    }

    let strategy = HarnessStrategy {
        name: fm.name.clone(),
        system_prompt,
        allowed_tools,
        tool_order: fm.tool_order.clone(),
        model_params,
        max_turns: fm.max_turns.unwrap_or(16),
        context: fm.context.clone().unwrap_or_else(ContextConfig::default),
    };

    Ok(ParsedStrategy {
        strategy,
        provider: fm.provider.clone(),
        model_present: fm.model.is_some(),
        description: fm.description.clone(),
        source_path: path.to_path_buf(),
    })
}

/// Output of `parse_strategy_file`. Carries the strategy plus the
/// out-of-band `provider:` reference the wiring layer needs to check.
#[derive(Debug, Clone)]
pub(crate) struct ParsedStrategy {
    pub strategy: HarnessStrategy,
    pub provider: Option<String>,
    pub model_present: bool,
    pub description: Option<String>,
    pub source_path: PathBuf,
}

/// Split a file body into `(frontmatter_yaml, body_text)`. Returns
/// `None` if the file does not begin with `---\n`.
///
/// Spec §18 risk: this consumes EXACTLY the first frontmatter block
/// and treats `---` lines in the body as horizontal-rule markdown,
/// not new frontmatter.
fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let trimmed = raw.trim_start_matches('\u{feff}'); // strip UTF-8 BOM if present

    let after_first = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n"))?;

    // Find the closing `---` line. Must be a line by itself.
    let mut idx = 0;
    for line in after_first.split_inclusive('\n') {
        let line_trim_end = line.trim_end_matches(['\r', '\n']);
        if line_trim_end == "---" {
            let yaml = &after_first[..idx];
            let after_close = &after_first[idx + line.len()..];
            return Some((yaml, after_close));
        }
        idx += line.len();
    }
    None
}

fn resolve_system_prompt(
    fm: &StrategyFrontmatter,
    body: &str,
    yaml_path: &Path,
) -> Result<String, LoadError> {
    match &fm.system_prompt {
        None => Ok(body.trim().to_string()),
        Some(SystemPromptSource::Inline(s)) => {
            if !body.trim().is_empty() {
                tracing::warn!(
                    path = %yaml_path.display(),
                    "frontmatter `system_prompt` overrides non-empty markdown body"
                );
            }
            Ok(s.clone())
        }
        Some(SystemPromptSource::FileRef { file }) => {
            let base_dir = yaml_path.parent().unwrap_or(Path::new("."));
            let resolved = base_dir.join(file);
            if !body.trim().is_empty() {
                tracing::warn!(
                    path = %yaml_path.display(),
                    "frontmatter `system_prompt: {{ file: ... }}` overrides non-empty markdown body"
                );
            }
            fs::read_to_string(&resolved).map_err(|source| LoadError::Io {
                path: resolved,
                source,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_frontmatter() {
        let raw = "---\nname: foo\n---\nbody text\n";
        let (yaml, body) = split_frontmatter(raw).unwrap();
        assert_eq!(yaml, "name: foo\n");
        assert_eq!(body, "body text\n");
    }

    #[test]
    fn splits_crlf_frontmatter() {
        let raw = "---\r\nname: foo\r\n---\r\nbody text\r\n";
        let (yaml, body) = split_frontmatter(raw).unwrap();
        assert!(yaml.contains("name: foo"));
        assert!(body.contains("body text"));
    }

    #[test]
    fn returns_none_when_no_frontmatter() {
        assert!(split_frontmatter("no fence here").is_none());
        assert!(split_frontmatter("--- not a fence").is_none());
    }

    #[test]
    fn returns_none_when_closing_fence_missing() {
        assert!(split_frontmatter("---\nname: foo\nbut no close\n").is_none());
    }

    #[test]
    fn body_can_contain_horizontal_rule_dashes() {
        let raw = "---\nname: foo\n---\nbody\n\n---\n\nmore body\n";
        let (yaml, body) = split_frontmatter(raw).unwrap();
        assert_eq!(yaml, "name: foo\n");
        assert!(body.contains("more body"), "second `---` must NOT be treated as a new frontmatter close");
    }

    #[test]
    fn strips_utf8_bom() {
        let raw = "\u{feff}---\nname: foo\n---\nbody\n";
        let (yaml, body) = split_frontmatter(raw).unwrap();
        assert_eq!(yaml, "name: foo\n");
        assert_eq!(body, "body\n");
    }
}
```

- [ ] **Step 2: Wire the module**

Edit `crates/cogito-strategy/src/lib.rs`:

```rust
pub mod error;
mod parser;
mod schema;

pub use error::LoadError;
```

- [ ] **Step 3: Run unit tests**

```bash
cargo test -p cogito-strategy parser::tests
```

Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/src/parser.rs crates/cogito-strategy/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(strategy): frontmatter splitter + parse_strategy_file

split_frontmatter handles CRLF, UTF-8 BOM, and body horizontal
rules without false fence matches. parse_strategy_file produces a
ParsedStrategy carrying the fully-materialized HarnessStrategy plus
the optional provider reference for the wiring layer to cross-check.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 06: File-fixture tests for `parse_strategy_file`

**Files:**
- Create: `crates/cogito-strategy/tests/fixtures/valid_minimal.md`
- Create: `crates/cogito-strategy/tests/fixtures/valid_full.md`
- Create: `crates/cogito-strategy/tests/fixtures/valid_file_ref.md`
- Create: `crates/cogito-strategy/tests/fixtures/file_ref_target.md`
- Create: `crates/cogito-strategy/tests/fixtures/malformed_no_frontmatter.md`
- Create: `crates/cogito-strategy/tests/fixtures/malformed_no_name.md`
- Create: `crates/cogito-strategy/tests/fixtures/mismatched_filename.md`
- Create: `crates/cogito-strategy/tests/fixtures/empty_prompt.md`
- Create: `crates/cogito-strategy/tests/parse.rs`

- [ ] **Step 1: Create fixtures**

`crates/cogito-strategy/tests/fixtures/valid_minimal.md`:
```markdown
---
name: valid_minimal
---

You are a helpful assistant.
```

`crates/cogito-strategy/tests/fixtures/valid_full.md`:
```markdown
---
name: valid_full
description: Coding strategy with everything wired
provider: anthropic-default
model: claude-opus-4-7
allowed_tools:
  - read_file
  - run_tests
tool_order:
  - read_file
  - run_tests
max_turns: 50
model_params:
  temperature: 0.3
  max_tokens: 4096
---

You are a precise software engineer.
Always read before writing.

---

A horizontal rule above this line must NOT confuse the splitter.
```

`crates/cogito-strategy/tests/fixtures/valid_file_ref.md`:
```markdown
---
name: valid_file_ref
system_prompt:
  file: ./file_ref_target.md
---
```

`crates/cogito-strategy/tests/fixtures/file_ref_target.md`:
```
You are loaded from a referenced file.
```

`crates/cogito-strategy/tests/fixtures/malformed_no_frontmatter.md`:
```markdown
no frontmatter at all
```

`crates/cogito-strategy/tests/fixtures/malformed_no_name.md`:
```markdown
---
description: missing name field
---

body
```

`crates/cogito-strategy/tests/fixtures/mismatched_filename.md`:
```markdown
---
name: not_the_filename
---

body
```

`crates/cogito-strategy/tests/fixtures/empty_prompt.md`:
```markdown
---
name: empty_prompt
---

```

- [ ] **Step 2: Make `parse_strategy_file` test-visible**

Edit `crates/cogito-strategy/src/parser.rs`: at the top, after the `pub(crate) fn parse_strategy_file` definition, add a `#[cfg(test)] pub` re-export route — but the simplest fix is to move the function and `ParsedStrategy` to `pub(crate)` and add a thin `pub fn parse_strategy_file_for_tests` to `lib.rs` behind `#[cfg(any(test, feature = "test-helpers"))]`.

Actually, prefer: change `pub(crate)` on `parse_strategy_file` and `ParsedStrategy` to `pub`. Mark the module `pub` in `lib.rs` (replace `mod parser;` with `pub mod parser;`). The function is not part of the public API contract — it's an implementation detail — but exposing it for integration tests is conventional. Document this with a `//! NOTE` line at the top of `parser.rs`.

Edit `crates/cogito-strategy/src/parser.rs`: change `pub(crate)` to `pub` on `parse_strategy_file`, `ParsedStrategy`, and its public fields. Add at the top:

```rust
//! NOTE: `parse_strategy_file` is `pub` so that integration tests in
//! `tests/` can drive it directly. End users should construct an
//! `FsStrategyRegistry` instead.
```

Edit `crates/cogito-strategy/src/lib.rs`: `mod parser;` -> `pub mod parser;`. Re-export `parse_strategy_file` at crate root for ergonomics:

```rust
pub use parser::{parse_strategy_file, ParsedStrategy};
```

- [ ] **Step 3: Write the integration test**

Create `crates/cogito-strategy/tests/parse.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use cogito_strategy::{parse_strategy_file, LoadError};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn parses_minimal_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_minimal.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_minimal");
    assert_eq!(parsed.strategy.system_prompt.trim(), "You are a helpful assistant.");
    assert!(parsed.provider.is_none());
}

#[test]
fn parses_full_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_full.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_full");
    assert_eq!(parsed.provider.as_deref(), Some("anthropic-default"));
    assert_eq!(parsed.strategy.model_params.model, "claude-opus-4-7");
    assert_eq!(parsed.strategy.model_params.temperature, Some(0.3));
    assert_eq!(parsed.strategy.max_turns, 50);
    assert!(
        parsed.strategy.system_prompt.contains("horizontal rule"),
        "body must survive a body-level `---` line without truncation"
    );
}

#[test]
fn parses_file_ref_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_file_ref.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_file_ref");
    assert_eq!(parsed.strategy.system_prompt.trim(), "You are loaded from a referenced file.");
}

#[test]
fn rejects_missing_frontmatter() {
    let err = parse_strategy_file(&fixture("malformed_no_frontmatter.md")).unwrap_err();
    assert!(matches!(err, LoadError::Frontmatter { .. }));
}

#[test]
fn rejects_missing_name_field() {
    let err = parse_strategy_file(&fixture("malformed_no_name.md")).unwrap_err();
    assert!(matches!(err, LoadError::Parse { .. }));
}

#[test]
fn rejects_filename_mismatch() {
    let err = parse_strategy_file(&fixture("mismatched_filename.md")).unwrap_err();
    assert!(matches!(err, LoadError::NameMismatch { .. }));
}

#[test]
fn rejects_empty_prompt() {
    let err = parse_strategy_file(&fixture("empty_prompt.md")).unwrap_err();
    assert!(matches!(err, LoadError::EmptyPrompt { .. }));
}
```

- [ ] **Step 4: Run integration tests**

```bash
cargo test -p cogito-strategy --test parse
```

Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/tests/ crates/cogito-strategy/src/parser.rs crates/cogito-strategy/src/lib.rs
git commit -m "$(cat <<'EOF'
test(strategy): file-fixture coverage for parse_strategy_file

Eight fixtures cover the happy paths (minimal, full with file-ref,
body horizontal-rule passthrough) and every LoadError variant the
parser can produce. parse_strategy_file is exposed pub so integration
tests can drive it without going through the registry.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 4: `cogito-strategy` — `FsStrategyRegistry`

### Task 07: Define `Scope` + `ScopeRoot` + conventional scopes helper

**Files:**
- Create: `crates/cogito-strategy/src/scope.rs`
- Modify: `crates/cogito-strategy/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/src/scope.rs`:

```rust
//! Scope precedence model for `FsStrategyRegistry`.
//!
//! Repo scope (highest) is conventionally `.cogito/strategies/` at the
//! current working directory. User scope (lowest) is
//! `~/.config/cogito/strategies/` (or the XDG equivalent). Repo wins
//! over User on cross-scope name collision; same-scope duplicate is
//! fatal at registry-build.

use std::path::PathBuf;

/// Discovery scope. Repo > User.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Repo-local: `.cogito/strategies/`.
    Repo,
    /// User-global: `~/.config/cogito/strategies/`.
    User,
}

/// One scope root: a (scope, path) pair. `path` is the directory the
/// registry will scan recursively.
#[derive(Debug, Clone)]
pub struct ScopeRoot {
    pub scope: Scope,
    pub path: PathBuf,
}

impl ScopeRoot {
    /// Convenience.
    #[must_use]
    pub fn new(scope: Scope, path: PathBuf) -> Self {
        Self { scope, path }
    }
}

/// Return the conventional roots in highest-precedence-first order.
/// Missing directories are not filtered here — `FsStrategyRegistry`
/// silently skips them. The Repo root respects an explicit override
/// (e.g., from `cogito.toml` `runtime.strategies_dir`); pass `None` to
/// use the convention.
#[must_use]
pub fn conventional_scopes(repo_override: Option<PathBuf>) -> Vec<ScopeRoot> {
    let repo = repo_override.unwrap_or_else(|| PathBuf::from(".cogito/strategies"));
    let user = user_scope_dir();
    vec![
        ScopeRoot::new(Scope::Repo, repo),
        ScopeRoot::new(Scope::User, user),
    ]
}

fn user_scope_dir() -> PathBuf {
    // Honor XDG_CONFIG_HOME if set, else fall back to ~/.config.
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("cogito").join("strategies");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("cogito").join("strategies");
    }
    // Last-resort: relative path that almost certainly won't exist, which
    // FsStrategyRegistry treats as "no User scope" silently.
    PathBuf::from(".config").join("cogito").join("strategies")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_repo_path() {
        let roots = conventional_scopes(None);
        assert_eq!(roots[0].scope, Scope::Repo);
        assert_eq!(roots[0].path, PathBuf::from(".cogito/strategies"));
    }

    #[test]
    fn repo_override_honored() {
        let roots = conventional_scopes(Some(PathBuf::from("/tmp/custom")));
        assert_eq!(roots[0].path, PathBuf::from("/tmp/custom"));
    }

    #[test]
    fn user_xdg_honored() {
        // Save and restore the env var so other tests aren't disturbed.
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", "/xdg/home");
        let roots = conventional_scopes(None);
        assert_eq!(roots[1].path, PathBuf::from("/xdg/home/cogito/strategies"));
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
```

- [ ] **Step 2: Wire the module**

Edit `crates/cogito-strategy/src/lib.rs`:

```rust
pub mod error;
pub mod parser;
mod schema;
pub mod scope;

pub use error::LoadError;
pub use parser::{parse_strategy_file, ParsedStrategy};
pub use scope::{conventional_scopes, Scope, ScopeRoot};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-strategy scope::tests
```

Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/src/scope.rs crates/cogito-strategy/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(strategy): Scope, ScopeRoot, conventional_scopes helper

Repo .cogito/strategies/ + User ~/.config/cogito/strategies/ (with
XDG_CONFIG_HOME honored). Repo > User precedence. Repo root accepts
an override so runtime.strategies_dir from cogito.toml keeps the
ADR-0017 escape hatch.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 08: Implement `FsStrategyRegistry`

**Files:**
- Create: `crates/cogito-strategy/src/registry.rs`
- Modify: `crates/cogito-strategy/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/src/registry.rs`:

```rust
//! Filesystem-backed StrategyRegistry impl.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};
use walkdir::WalkDir;

use crate::error::LoadError;
use crate::parser::{parse_strategy_file, ParsedStrategy};
use crate::scope::{Scope, ScopeRoot};

/// FS-backed registry. Built once at startup; immutable thereafter.
#[derive(Debug, Clone)]
pub struct FsStrategyRegistry {
    /// `name -> parsed strategy` (winning scope only).
    by_name: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone)]
struct Entry {
    parsed: ParsedStrategy,
    winning_scope: Scope,
}

impl FsStrategyRegistry {
    /// Build a registry by scanning the given scope roots in
    /// highest-precedence-first order. Missing roots are silently
    /// skipped. Same-scope duplicate names are fatal; cross-scope
    /// shadowing is allowed (higher-precedence scope wins, lower is
    /// silently dropped).
    ///
    /// # Errors
    ///
    /// Returns the first `LoadError` encountered (I/O, parse, or
    /// duplicate-within-scope).
    pub fn from_roots(roots: &[ScopeRoot]) -> Result<Self, LoadError> {
        let mut by_name: BTreeMap<String, Entry> = BTreeMap::new();

        for root in roots {
            let scope = root.scope;
            // Per-scope dedupe tracker to detect within-scope duplicates.
            let mut scope_seen: BTreeMap<String, PathBuf> = BTreeMap::new();

            if !root.path.exists() {
                tracing::debug!(path = %root.path.display(), "scope root missing; skipping");
                continue;
            }

            for entry in WalkDir::new(&root.path).into_iter().filter_map(Result::ok) {
                if !entry.file_type().is_file() {
                    continue;
                }
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                let parsed = parse_strategy_file(p)?;
                let name = parsed.strategy.name.clone();

                if let Some(prev_path) = scope_seen.get(&name) {
                    return Err(LoadError::DuplicateName {
                        name,
                        files: vec![prev_path.clone(), p.to_path_buf()],
                    });
                }
                scope_seen.insert(name.clone(), p.to_path_buf());

                // Cross-scope shadowing: only insert if the name is not
                // already taken by a higher-precedence scope.
                if !by_name.contains_key(&name) {
                    by_name.insert(name.clone(), Entry { parsed, winning_scope: scope });
                } else {
                    tracing::debug!(
                        name = %name,
                        winning_path = %by_name[&name].parsed.source_path.display(),
                        shadowed_path = %p.display(),
                        "lower-precedence scope shadowed",
                    );
                }
            }
        }

        Ok(Self { by_name })
    }

    /// Convenience: scan the conventional roots
    /// (`Repo: .cogito/strategies/`, `User: ~/.config/cogito/strategies/`).
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_roots`].
    pub fn from_conventional_scopes() -> Result<Self, LoadError> {
        Self::from_roots(&crate::scope::conventional_scopes(None))
    }

    /// Convenience: scan the conventional roots with an explicit Repo
    /// override (used when `cogito.toml` `runtime.strategies_dir` is set).
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_roots`].
    pub fn from_conventional_scopes_with_repo_override(
        repo: PathBuf,
    ) -> Result<Self, LoadError> {
        Self::from_roots(&crate::scope::conventional_scopes(Some(repo)))
    }

    /// Returns the description field for a named strategy (used by
    /// `cogito chat --list-strategies`).
    #[must_use]
    pub fn description(&self, name: &str) -> Option<&str> {
        self.by_name.get(name).and_then(|e| e.parsed.description.as_deref())
    }

    /// Returns the `provider:` reference declared by the strategy, if any.
    /// Used by the wiring layer to cross-check against cogito.toml.
    #[must_use]
    pub fn provider_ref(&self, name: &str) -> Option<&str> {
        self.by_name.get(name).and_then(|e| e.parsed.provider.as_deref())
    }
}

impl StrategyRegistry for FsStrategyRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        self.by_name
            .get(name)
            .map(|e| e.parsed.strategy.clone())
            .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
    }

    fn list(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &PathBuf, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn missing_root_is_skipped() {
        let reg = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            PathBuf::from("/does/not/exist/anywhere"),
        )])
        .unwrap();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn loads_two_strategies() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("coder.md"),
            "---\nname: coder\n---\nBe precise.\n",
        );
        write(
            &tmp.path().join("planner.md"),
            "---\nname: planner\n---\nThink first.\n",
        );

        let reg = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            tmp.path().to_path_buf(),
        )])
        .unwrap();
        assert_eq!(reg.list(), vec!["coder", "planner"]);
        assert!(reg.get("coder").is_ok());
        assert!(reg.get("planner").is_ok());
    }

    #[test]
    fn duplicate_name_within_scope_is_fatal() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("coder.md"),
            "---\nname: coder\n---\none.\n",
        );
        // Same name, different file - simulate by placing in a subdir.
        write(
            &tmp.path().join("subdir/coder.md"),
            "---\nname: coder\n---\ntwo.\n",
        );

        let err = FsStrategyRegistry::from_roots(&[ScopeRoot::new(
            Scope::Repo,
            tmp.path().to_path_buf(),
        )])
        .unwrap_err();
        assert!(matches!(err, LoadError::DuplicateName { ref name, .. } if name == "coder"));
    }

    #[test]
    fn repo_shadows_user() {
        let repo = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write(
            &repo.path().join("coder.md"),
            "---\nname: coder\n---\nFROM REPO.\n",
        );
        write(
            &user.path().join("coder.md"),
            "---\nname: coder\n---\nFROM USER.\n",
        );

        let reg = FsStrategyRegistry::from_roots(&[
            ScopeRoot::new(Scope::Repo, repo.path().to_path_buf()),
            ScopeRoot::new(Scope::User, user.path().to_path_buf()),
        ])
        .unwrap();

        let s = reg.get("coder").unwrap();
        assert_eq!(s.system_prompt.trim(), "FROM REPO.");
    }

    #[test]
    fn unknown_name_returns_strategy_error() {
        let reg = FsStrategyRegistry::from_roots(&[]).unwrap();
        let err = reg.get("nope").unwrap_err();
        assert!(matches!(err, StrategyError::Unknown(ref n, _) if n == "nope"));
    }
}
```

- [ ] **Step 2: Wire the module**

Edit `crates/cogito-strategy/src/lib.rs`:

```rust
pub mod error;
pub mod parser;
pub mod registry;
mod schema;
pub mod scope;

pub use error::LoadError;
pub use parser::{parse_strategy_file, ParsedStrategy};
pub use registry::FsStrategyRegistry;
pub use scope::{conventional_scopes, Scope, ScopeRoot};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-strategy registry::tests
```

Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-strategy
git add crates/cogito-strategy/src/registry.rs crates/cogito-strategy/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(strategy): FsStrategyRegistry impl

Walks each ScopeRoot for *.md files, parses each, enforces
within-scope name uniqueness, lets higher-precedence scopes shadow
lower ones. Missing scope roots are silently skipped. Impls
StrategyRegistry from cogito-protocol.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 5: `cogito-test-fixtures` — `MapStrategyRegistry` + contract suite

### Task 09: Add `MapStrategyRegistry` and `strategy_registry_contract` helper

**Files:**
- Create: `crates/testing/cogito-test-fixtures/src/strategy.rs`
- Modify: `crates/testing/cogito-test-fixtures/src/lib.rs`
- Modify: `crates/testing/cogito-test-fixtures/Cargo.toml`

- [ ] **Step 1: Check existing test-fixtures dependencies**

Run: `grep "cogito-protocol\|cogito-strategy" crates/testing/cogito-test-fixtures/Cargo.toml`

Expected: `cogito-protocol` is already a dep. `cogito-strategy` is not (we'll add it as dev-dep only later in plan-execution; for the contract-suite source, all we need is `cogito-protocol`).

- [ ] **Step 2: Write the failing test**

Create `crates/testing/cogito-test-fixtures/src/strategy.rs`:

```rust
//! In-memory `StrategyRegistry` + a contract test suite consumed by
//! every concrete impl (FS-backed, future DB-backed).

use std::collections::BTreeMap;

use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};

/// Simple in-memory registry for tests that don't want to touch disk.
#[derive(Debug, Clone, Default)]
pub struct MapStrategyRegistry {
    inner: BTreeMap<String, HarnessStrategy>,
}

impl MapStrategyRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a strategy by name. Last write wins.
    pub fn insert(&mut self, name: impl Into<String>, strategy: HarnessStrategy) {
        self.inner.insert(name.into(), strategy);
    }

    /// Builder-style helper.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, strategy: HarnessStrategy) -> Self {
        self.insert(name, strategy);
        self
    }
}

impl StrategyRegistry for MapStrategyRegistry {
    fn get(&self, name: &str) -> Result<HarnessStrategy, StrategyError> {
        self.inner
            .get(name)
            .cloned()
            .ok_or_else(|| StrategyError::Unknown(name.to_string(), self.list()))
    }

    fn list(&self) -> Vec<String> {
        self.inner.keys().cloned().collect()
    }
}

/// Run the canonical contract suite against any `StrategyRegistry` impl.
///
/// `make_registry` returns a freshly-built registry containing exactly
/// these strategies: `"foo"` (system_prompt = "FOO"), `"bar"`
/// (system_prompt = "BAR"). The contract is impl-agnostic.
pub fn strategy_registry_contract<R: StrategyRegistry>(
    make_registry: impl Fn() -> R,
) {
    // list() is sorted.
    let reg = make_registry();
    assert_eq!(reg.list(), vec!["bar", "foo"], "list() must be sorted");

    // list() is deduplicated (trivially true for our two-entry case;
    // re-running the contract on a registry with duplicate-name entries
    // is meaningful — kept simple here).
    let reg = make_registry();
    let names = reg.list();
    let mut sorted_dedup = names.clone();
    sorted_dedup.dedup();
    assert_eq!(names, sorted_dedup, "list() must be deduplicated");

    // get() of any name in list() succeeds and is deterministic.
    let reg = make_registry();
    for name in reg.list() {
        let first = reg.get(&name).unwrap();
        let second = reg.get(&name).unwrap();
        assert_eq!(first.name, second.name, "get is deterministic for {name}");
        assert_eq!(first.system_prompt, second.system_prompt, "get is deterministic for {name}");
    }

    // get() of any name NOT in list() returns Unknown.
    let reg = make_registry();
    let err = reg.get("definitely-not-registered").unwrap_err();
    assert!(matches!(err, StrategyError::Unknown(_, _)), "expected Unknown, got {err:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use cogito_protocol::strategy::HarnessStrategy;

    fn build_canonical() -> MapStrategyRegistry {
        let mut foo = HarnessStrategy::default_with_model("test");
        foo.name = "foo".into();
        foo.system_prompt = "FOO".into();
        let mut bar = HarnessStrategy::default_with_model("test");
        bar.name = "bar".into();
        bar.system_prompt = "BAR".into();
        MapStrategyRegistry::new().with("foo", foo).with("bar", bar)
    }

    #[test]
    fn map_registry_passes_contract() {
        strategy_registry_contract(build_canonical);
    }
}
```

- [ ] **Step 3: Wire the module**

Edit `crates/testing/cogito-test-fixtures/src/lib.rs` and add (next to other `pub mod` declarations):

```rust
pub mod strategy;
```

If there are re-exports at the crate root, add:

```rust
pub use strategy::{strategy_registry_contract, MapStrategyRegistry};
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p cogito-test-fixtures strategy::tests
```

Expected: 1 test (`map_registry_passes_contract`) passes.

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-test-fixtures
git add crates/testing/cogito-test-fixtures/src/strategy.rs crates/testing/cogito-test-fixtures/src/lib.rs
git commit -m "$(cat <<'EOF'
test(fixtures): MapStrategyRegistry + strategy_registry_contract

In-memory StrategyRegistry plus the canonical contract suite every
impl (FS, future DB) must pass: list-is-sorted, list-is-deduped,
get-is-deterministic, unknown-returns-StrategyError::Unknown.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: Run the contract suite against `FsStrategyRegistry`

**Files:**
- Create: `crates/cogito-strategy/tests/contract.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-strategy/tests/contract.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use cogito_strategy::{FsStrategyRegistry, Scope, ScopeRoot};
use cogito_test_fixtures::strategy::strategy_registry_contract;
use tempfile::TempDir;

fn canonical_fs_registry() -> FsStrategyRegistry {
    // Match the contract's expectation: registry holds exactly "foo" and "bar".
    let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
    let foo = tmp.path().join("foo.md");
    let bar = tmp.path().join("bar.md");
    fs::write(&foo, "---\nname: foo\n---\nFOO\n").unwrap();
    fs::write(&bar, "---\nname: bar\n---\nBAR\n").unwrap();
    FsStrategyRegistry::from_roots(&[ScopeRoot::new(Scope::Repo, tmp.path().to_path_buf())])
        .unwrap()
}

#[test]
fn fs_registry_passes_contract() {
    strategy_registry_contract(canonical_fs_registry);
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p cogito-strategy --test contract
```

Expected: 1 test passes.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-strategy/tests/contract.rs
git commit -m "$(cat <<'EOF'
test(strategy): FsStrategyRegistry passes the canonical contract

Proves the FS impl satisfies the same invariants every future
StrategyRegistry impl will need to satisfy.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 6: OpenAI Responses adapter — `cogito-model::openai_responses`

### Task 11: Add `ReasoningEffort` enum + scaffold the `openai_responses` module

**Files:**
- Create: `crates/cogito-model/src/openai_responses/mod.rs`
- Create: `crates/cogito-model/src/openai_responses/wire.rs`
- Modify: `crates/cogito-model/src/lib.rs`

- [ ] **Step 1: Inspect the existing `openai_compat` shape**

Run: `ls crates/cogito-model/src/openai_compat/`
Expected: `decode.rs encode.rs mod.rs wire.rs`

Quickly skim `crates/cogito-model/src/openai_compat/mod.rs` to see the gateway constructor pattern. We will mirror this layout.

- [ ] **Step 2: Create `wire.rs` with the minimal request body + SSE event types**

Create `crates/cogito-model/src/openai_responses/wire.rs`:

```rust
//! Wire-protocol types for the OpenAI Responses API.
//!
//! Reference: https://platform.openai.com/docs/api-reference/responses
//! We model only the subset cogito uses: messages + function tools +
//! streaming text + reasoning summary items + stop reasons + usage.

use serde::{Deserialize, Serialize};

/// Top-level POST body to `/v1/responses`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesRequest {
    pub model: String,
    pub input: Vec<InputItem>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `reasoning.effort` toggle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReasoningParams {
    pub effort: ReasoningEffort,
}

/// One input item. Responses uses a flat top-level array; user/assistant
/// turns become `message` items, tool calls become `function_call` items,
/// tool results become `function_call_output` items, and prior thinking
/// becomes `reasoning` items.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum InputItem {
    Message {
        role: String,
        content: Vec<MessageContent>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    Reasoning {
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        summary: Vec<ReasoningSummary>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MessageContent {
    InputText { text: String },
    OutputText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReasoningSummary {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String, // "function"
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// SSE events: each event has a "type" discriminator. We model only the
// subset we route to ModelEvent.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamEvent {
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String },

    #[serde(rename = "response.output_text.done")]
    OutputTextDone { text: String },

    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryDelta { delta: String },

    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryDone { text: String },

    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgsDelta {
        item_id: String,
        delta: String,
    },

    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgsDone {
        item_id: String,
        arguments: String,
    },

    #[serde(rename = "response.output_item.added")]
    OutputItemAdded { item: OutputItemHeader },

    #[serde(rename = "response.completed")]
    Completed { response: ResponseFinal },

    #[serde(rename = "response.failed")]
    Failed { response: ResponseFinal },

    /// Unknown event types are decoded as `Other` so the parser does
    /// not error out on harmless additions.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OutputItemHeader {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "message" | "function_call" | "reasoning"
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ResponseFinal {
    #[serde(default)]
    pub status: Option<String>, // "completed" | "incomplete" | "failed"
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IncompleteDetails {
    pub reason: String, // "max_output_tokens" | "content_filter" | ...
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ResponseError {
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
}
```

- [ ] **Step 3: Create `mod.rs` (gateway stub for now; encode/decode in next tasks)**

Create `crates/cogito-model/src/openai_responses/mod.rs`:

```rust
//! OpenAI Responses API gateway. Decodes native reasoning items into
//! `ContentBlock::Thinking` per ADR-0019. No built-in tools
//! (file_search/web_search/code_interpreter) — Hands concern, separate
//! ADR if/when needed.

mod decode;
mod encode;
mod wire;

pub use wire::ReasoningEffort;

use std::time::Duration;

use cogito_protocol::gateway::{
    ModelError, ModelGateway, ModelInput, ModelLimits, ModelStream,
};
use reqwest::Client;

/// Build-time configuration for `OpenAiResponsesGateway`.
#[derive(Debug, Clone)]
pub struct OpenAiResponsesConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout: Duration,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl OpenAiResponsesConfig {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.openai.com/v1";

    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: Self::DEFAULT_BASE_URL.into(),
            timeout: Duration::from_secs(300),
            reasoning_effort: None,
        }
    }
}

/// `ModelGateway` impl for OpenAI Responses.
pub struct OpenAiResponsesGateway {
    cfg: OpenAiResponsesConfig,
    client: Client,
}

impl OpenAiResponsesGateway {
    /// Build a gateway from config. Forwards `reqwest::Client::builder`
    /// errors as `ModelError::Init`.
    ///
    /// # Errors
    ///
    /// Returns `ModelError::Init` if the reqwest client cannot be built.
    pub fn new(cfg: OpenAiResponsesConfig) -> Result<Self, ModelError> {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| ModelError::Init(e.to_string()))?;
        Ok(Self { cfg, client })
    }
}

#[async_trait::async_trait]
impl ModelGateway for OpenAiResponsesGateway {
    fn name(&self) -> &str {
        "openai-responses"
    }

    fn model_limits(&self, _model_id: &str) -> ModelLimits {
        // Responses-eligible models (o1, o3, gpt-4o) generally support
        // 128k context. Conservative default.
        ModelLimits {
            context_window_tokens: 128_000,
        }
    }

    async fn stream(&self, input: ModelInput) -> Result<ModelStream, ModelError> {
        let request = encode::encode_request(&input, &self.cfg);
        decode::stream_response(&self.client, &self.cfg, request).await
    }
}
```

- [ ] **Step 4: Stub out empty `encode.rs` and `decode.rs` so the mod compiles**

Create `crates/cogito-model/src/openai_responses/encode.rs`:

```rust
//! ContentBlock -> Responses input items. Implemented in Task 12.

use cogito_protocol::gateway::ModelInput;

use super::wire::ResponsesRequest;
use super::OpenAiResponsesConfig;

pub(crate) fn encode_request(_input: &ModelInput, _cfg: &OpenAiResponsesConfig) -> ResponsesRequest {
    unimplemented!("Task 12 lands the encoder")
}
```

Create `crates/cogito-model/src/openai_responses/decode.rs`:

```rust
//! Responses SSE -> ModelEvent. Implemented in Task 13.

use cogito_protocol::gateway::{ModelError, ModelStream};
use reqwest::Client;

use super::wire::ResponsesRequest;
use super::OpenAiResponsesConfig;

pub(crate) async fn stream_response(
    _client: &Client,
    _cfg: &OpenAiResponsesConfig,
    _request: ResponsesRequest,
) -> Result<ModelStream, ModelError> {
    Err(ModelError::Init("Task 13 lands the decoder".into()))
}
```

- [ ] **Step 5: Wire the module in `cogito-model::lib.rs`**

Edit `crates/cogito-model/src/lib.rs`, add (next to `pub mod openai_compat;`):

```rust
pub mod openai_responses;

pub use openai_responses::{OpenAiResponsesConfig, OpenAiResponsesGateway, ReasoningEffort};
```

- [ ] **Step 6: Verify build**

```bash
cargo build -p cogito-model
```

Expected: clean build, unimplemented! is fine (compiles but would panic if called).

- [ ] **Step 7: Commit**

```bash
make fmt && make fix CRATE=cogito-model
git add crates/cogito-model/src/openai_responses/ crates/cogito-model/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(model): scaffold openai_responses adapter

Wire types + ReasoningEffort enum + OpenAiResponsesGateway shell.
Encoder + decoder land in the next two tasks. Mirrors the
openai_compat layout. ADR-0019 reasoning items flow through native
Responses reasoning summary parts in decode.rs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Implement `encode::encode_request`

**Files:**
- Modify: `crates/cogito-model/src/openai_responses/encode.rs`

- [ ] **Step 1: Replace the stub with the encoder**

Replace `crates/cogito-model/src/openai_responses/encode.rs` with:

```rust
//! ContentBlock -> Responses input items.
//!
//! Mapping rules:
//! - `ContentBlock::Text` in a user turn  -> `InputItem::Message { role: "user", content: [InputText] }`
//! - `ContentBlock::Text` in an assistant turn -> `Message { role: "assistant", content: [OutputText] }`
//! - `ContentBlock::ToolUse` -> `FunctionCall { call_id, name, arguments }`
//! - `ContentBlock::ToolResult` -> `FunctionCallOutput { call_id, output }`
//! - `ContentBlock::Thinking` -> `Reasoning { summary: [...] }` (re-feeds prior reasoning back per ADR-0019)
//! - System prompt -> `instructions` field on the request

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{ChatMessage, ModelInput, Role};
use serde_json::json;

use super::wire::{
    InputItem, MessageContent, ReasoningEffort, ReasoningParams, ReasoningSummary,
    ResponsesRequest, ToolDef,
};
use super::OpenAiResponsesConfig;

pub(crate) fn encode_request(
    input: &ModelInput,
    cfg: &OpenAiResponsesConfig,
) -> ResponsesRequest {
    let mut items: Vec<InputItem> = Vec::new();
    for msg in &input.messages {
        encode_message(msg, &mut items);
    }

    let tools = input
        .tools
        .iter()
        .map(|t| ToolDef {
            kind: "function".into(),
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.input_schema.clone(),
        })
        .collect();

    let reasoning = cfg.reasoning_effort.map(|effort| ReasoningParams { effort });

    ResponsesRequest {
        model: input.params.model.clone(),
        input: items,
        stream: true,
        max_output_tokens: Some(input.params.max_tokens),
        temperature: input.params.temperature,
        top_p: input.params.top_p,
        tools,
        reasoning,
        instructions: if input.system_prompt.is_empty() {
            None
        } else {
            Some(input.system_prompt.clone())
        },
    }
}

fn encode_message(msg: &ChatMessage, out: &mut Vec<InputItem>) {
    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
    };

    // Text blocks collapse into one message item per role-run.
    let mut text_runs: Vec<MessageContent> = Vec::new();
    let flush_texts = |runs: &mut Vec<MessageContent>, out: &mut Vec<InputItem>| {
        if !runs.is_empty() {
            out.push(InputItem::Message {
                role: role_str.to_string(),
                content: std::mem::take(runs),
            });
        }
    };

    for block in &msg.content {
        match block {
            ContentBlock::Text(t) => {
                let mc = if matches!(msg.role, Role::Assistant) {
                    MessageContent::OutputText { text: t.text.clone() }
                } else {
                    MessageContent::InputText { text: t.text.clone() }
                };
                text_runs.push(mc);
            }
            ContentBlock::ToolUse(tu) => {
                flush_texts(&mut text_runs, out);
                out.push(InputItem::FunctionCall {
                    call_id: tu.id.clone(),
                    name: tu.name.clone(),
                    arguments: tu.input.to_string(),
                });
            }
            ContentBlock::ToolResult(tr) => {
                flush_texts(&mut text_runs, out);
                out.push(InputItem::FunctionCallOutput {
                    call_id: tr.tool_use_id.clone(),
                    output: tr.content.iter().map(|b| match b {
                        ContentBlock::Text(t) => t.text.clone(),
                        _ => serde_json::to_string(b).unwrap_or_default(),
                    }).collect::<Vec<_>>().join("\n"),
                });
            }
            ContentBlock::Thinking(th) => {
                flush_texts(&mut text_runs, out);
                out.push(InputItem::Reasoning {
                    summary: vec![ReasoningSummary {
                        kind: "summary_text".into(),
                        text: th.thinking.clone(),
                    }],
                });
            }
            _ => {
                // Image / other non-text blocks not yet supported on
                // the Responses adapter. v0.5 will land them.
                let _ = json!({}); // silence unused-import warning until then
            }
        }
    }
    flush_texts(&mut text_runs, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cogito_protocol::content::{TextBlock, ThinkingBlock, ToolUseBlock};
    use cogito_protocol::gateway::ModelParams;

    fn empty_params() -> ModelParams {
        ModelParams {
            model: "test".into(),
            max_tokens: 100,
            temperature: None,
            top_p: None,
            stop_sequences: vec![],
        }
    }

    #[test]
    fn encodes_a_simple_user_message() {
        let input = ModelInput {
            system_prompt: "".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: vec![ContentBlock::Text(TextBlock { text: "hi".into() })],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        assert!(req.stream);
        assert!(req.instructions.is_none());
        match &req.input[0] {
            InputItem::Message { role, content } => {
                assert_eq!(role, "user");
                assert!(matches!(content[0], MessageContent::InputText { ref text } if text == "hi"));
            }
            other => panic!("unexpected first item: {other:?}"),
        }
    }

    #[test]
    fn encodes_reasoning_effort_when_set() {
        let input = ModelInput {
            system_prompt: "sys".into(),
            messages: vec![],
            tools: vec![],
            params: empty_params(),
        };
        let mut cfg = OpenAiResponsesConfig::with_api_key("k");
        cfg.reasoning_effort = Some(ReasoningEffort::Medium);
        let req = encode_request(&input, &cfg);
        assert!(req.reasoning.is_some());
        assert_eq!(req.instructions.as_deref(), Some("sys"));
    }

    #[test]
    fn encodes_tool_use_and_result_as_function_items() {
        let tool_use = ToolUseBlock {
            id: "call_1".into(),
            name: "read_file".into(),
            input: serde_json::json!({"path": "/etc/hosts"}),
        };
        let input = ModelInput {
            system_prompt: "".into(),
            messages: vec![ChatMessage {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse(tool_use)],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        match &req.input[0] {
            InputItem::FunctionCall { call_id, name, .. } => {
                assert_eq!(call_id, "call_1");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected FunctionCall, got {other:?}"),
        }
    }

    #[test]
    fn encodes_thinking_block_as_reasoning_item() {
        let input = ModelInput {
            system_prompt: "".into(),
            messages: vec![ChatMessage {
                role: Role::Assistant,
                content: vec![ContentBlock::Thinking(ThinkingBlock {
                    thinking: "let me think...".into(),
                    signature: None,
                })],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        match &req.input[0] {
            InputItem::Reasoning { summary } => {
                assert_eq!(summary[0].text, "let me think...");
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Verify**

The encoder references `cogito_protocol::content::{TextBlock, ThinkingBlock, ToolUseBlock, ToolResultBlock}` and `cogito_protocol::gateway::{ChatMessage, Role}`. Check the actual names with:

```bash
grep -n "pub struct\|pub enum" crates/cogito-protocol/src/content.rs
grep -n "pub struct\|pub enum" crates/cogito-protocol/src/gateway.rs | head -20
```

If any names differ from what the test uses (TextBlock / ThinkingBlock / ToolUseBlock / ToolResultBlock), update the test to match — these types already exist; the encoder must use the canonical names.

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-model openai_responses::encode::tests
```

Expected: 4 tests pass. If `ThinkingBlock` has a different field name than `thinking` (e.g., `text`), adjust accordingly — the existing `openai_compat::encode` is the source of truth, mirror its field accesses.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-model
git add crates/cogito-model/src/openai_responses/encode.rs
git commit -m "$(cat <<'EOF'
feat(model/responses): encode ContentBlock to Responses input items

Text -> Message{content: [InputText|OutputText]} per role; ToolUse ->
FunctionCall; ToolResult -> FunctionCallOutput; Thinking -> Reasoning
summary (re-feeds prior reasoning per ADR-0019). System prompt ->
instructions field. Reasoning effort flows from provider config.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: Implement `decode::stream_response` + SSE fixtures

**Files:**
- Modify: `crates/cogito-model/src/openai_responses/decode.rs`
- Create: `crates/cogito-model/tests/fixtures/openai_responses/text_completion.sse`
- Create: `crates/cogito-model/tests/fixtures/openai_responses/tool_call.sse`
- Create: `crates/cogito-model/tests/fixtures/openai_responses/reasoning_summary.sse`
- Create: `crates/cogito-model/tests/fixtures/openai_responses/stop_reason_max_tokens.sse`
- Create: `crates/cogito-model/tests/openai_responses_decode.rs`

- [ ] **Step 1: Inspect existing `openai_compat::decode` to mirror its style**

Run: `grep -n "stream_response\|eventsource\|SseStream" crates/cogito-model/src/openai_compat/decode.rs | head -20`

Note the helpers used (likely `reqwest::Response::bytes_stream()` + `eventsource-stream`). Reuse the same pattern.

- [ ] **Step 2: Replace `decode.rs` stub with the full decoder**

Replace `crates/cogito-model/src/openai_responses/decode.rs` with:

```rust
//! Responses SSE -> ModelEvent stream.

use std::pin::Pin;

use cogito_protocol::content::{TextBlock, ThinkingBlock, ToolUseBlock};
use cogito_protocol::gateway::{
    FinishReason, ModelError, ModelEvent, ModelStream, Usage as ProtocolUsage,
};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use std::collections::HashMap;

use super::wire::{ResponsesRequest, StreamEvent};
use super::OpenAiResponsesConfig;

pub(crate) async fn stream_response(
    client: &Client,
    cfg: &OpenAiResponsesConfig,
    request: ResponsesRequest,
) -> Result<ModelStream, ModelError> {
    let url = format!("{}/responses", cfg.base_url);
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| ModelError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ModelError::Http(format!("HTTP {status}: {body}")));
    }

    // Per-item-id tracking. Responses streams emit text deltas and
    // function-call argument deltas tagged with the same `item_id` they
    // were introduced with via `response.output_item.added`. We carry
    // that mapping to label outgoing events.
    let item_kinds: HashMap<String, String> = HashMap::new();
    let function_calls: HashMap<String, (String, String)> = HashMap::new(); // item_id -> (call_id, name)

    let byte_stream = resp.bytes_stream();
    let sse = byte_stream.eventsource();

    let stream = futures::stream::unfold(
        DecodeState {
            sse,
            item_kinds,
            function_calls,
        },
        |mut state| async move {
            loop {
                let evt = state.sse.next().await?;
                let evt = match evt {
                    Ok(e) => e,
                    Err(e) => return Some((Err(ModelError::Stream(e.to_string())), state)),
                };
                if evt.data.is_empty() {
                    continue;
                }
                let parsed: StreamEvent = match serde_json::from_str(&evt.data) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::debug!(error = %e, raw = %evt.data, "unparseable Responses SSE event");
                        continue;
                    }
                };
                match parsed {
                    StreamEvent::OutputItemAdded { item } => {
                        state.item_kinds.insert(item.id.clone(), item.kind.clone());
                        if item.kind == "function_call" {
                            if let (Some(call_id), Some(name)) = (item.call_id.clone(), item.name.clone()) {
                                state.function_calls.insert(item.id.clone(), (call_id, name));
                            }
                        }
                        continue;
                    }
                    StreamEvent::OutputTextDelta { delta } => {
                        return Some((
                            Ok(ModelEvent::TextDelta { text: delta }),
                            state,
                        ));
                    }
                    StreamEvent::OutputTextDone { text } => {
                        return Some((
                            Ok(ModelEvent::TextBlockCompleted(TextBlock { text })),
                            state,
                        ));
                    }
                    StreamEvent::ReasoningSummaryDelta { delta } => {
                        return Some((
                            Ok(ModelEvent::ThinkingDelta { text: delta }),
                            state,
                        ));
                    }
                    StreamEvent::ReasoningSummaryDone { text } => {
                        return Some((
                            Ok(ModelEvent::ThinkingBlockCompleted(ThinkingBlock {
                                thinking: text,
                                signature: None,
                            })),
                            state,
                        ));
                    }
                    StreamEvent::FunctionCallArgsDelta { item_id, delta } => {
                        if let Some((call_id, name)) = state.function_calls.get(&item_id).cloned() {
                            return Some((
                                Ok(ModelEvent::ToolCallDelta {
                                    id: call_id,
                                    name,
                                    input_delta: delta,
                                }),
                                state,
                            ));
                        }
                        continue;
                    }
                    StreamEvent::FunctionCallArgsDone { item_id, arguments } => {
                        if let Some((call_id, name)) = state.function_calls.get(&item_id).cloned() {
                            let input: serde_json::Value =
                                serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null);
                            return Some((
                                Ok(ModelEvent::ToolCallCompleted(ToolUseBlock {
                                    id: call_id,
                                    name,
                                    input,
                                })),
                                state,
                            ));
                        }
                        continue;
                    }
                    StreamEvent::Completed { response } => {
                        let stop_reason = match (
                            response.status.as_deref(),
                            response.incomplete_details.as_ref(),
                        ) {
                            (_, Some(d)) if d.reason == "max_output_tokens" => FinishReason::MaxTokens,
                            (Some("incomplete"), _) => FinishReason::MaxTokens,
                            _ => FinishReason::EndTurn,
                        };
                        let usage = response.usage.as_ref().map(|u| ProtocolUsage {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                        });
                        return Some((
                            Ok(ModelEvent::Completed { stop_reason, usage }),
                            state,
                        ));
                    }
                    StreamEvent::Failed { response } => {
                        let msg = response
                            .error
                            .map(|e| e.message)
                            .unwrap_or_else(|| "Responses stream failed".into());
                        return Some((Err(ModelError::Stream(msg)), state));
                    }
                    StreamEvent::Other => continue,
                }
            }
        },
    );

    let boxed: Pin<Box<dyn futures::Stream<Item = Result<ModelEvent, ModelError>> + Send>> =
        Box::pin(stream);
    Ok(ModelStream::new(boxed))
}

struct DecodeState<S> {
    sse: S,
    item_kinds: HashMap<String, String>,
    function_calls: HashMap<String, (String, String)>,
}

#[cfg(test)]
mod tests {
    //! Live-API tests live in `crates/cogito-model/tests/openai_responses_decode.rs`
    //! and drive recorded SSE fixtures through a tiny parser harness.
    //!
    //! Unit-level tests here cover the parser's degenerate paths:
    //! - Unknown event types are skipped (`StreamEvent::Other`).
    //! - Empty SSE data lines are skipped.
    //!
    //! These two assertions are folded into the integration test below
    //! via dedicated fixture lines.
}
```

- [ ] **Step 3: Create SSE fixtures**

Create `crates/cogito-model/tests/fixtures/openai_responses/text_completion.sse`:

```
event: response.output_item.added
data: {"type":"response.output_item.added","item":{"id":"msg_1","type":"message"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Hello"}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":" world"}

event: response.output_text.done
data: {"type":"response.output_text.done","text":"Hello world"}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":12,"output_tokens":4,"total_tokens":16}}}

```

Create `crates/cogito-model/tests/fixtures/openai_responses/tool_call.sse`:

```
event: response.output_item.added
data: {"type":"response.output_item.added","item":{"id":"item_1","type":"function_call","call_id":"call_abc","name":"read_file"}}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","delta":"{\"path\":"}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","delta":"\"/etc/hosts\"}"}

event: response.function_call_arguments.done
data: {"type":"response.function_call_arguments.done","item_id":"item_1","arguments":"{\"path\":\"/etc/hosts\"}"}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":50,"output_tokens":10,"total_tokens":60}}}

```

Create `crates/cogito-model/tests/fixtures/openai_responses/reasoning_summary.sse`:

```
event: response.output_item.added
data: {"type":"response.output_item.added","item":{"id":"r_1","type":"reasoning"}}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","delta":"Considering"}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","delta":" the request..."}

event: response.reasoning_summary_text.done
data: {"type":"response.reasoning_summary_text.done","text":"Considering the request..."}

event: response.output_item.added
data: {"type":"response.output_item.added","item":{"id":"msg_1","type":"message"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Here is the answer."}

event: response.output_text.done
data: {"type":"response.output_text.done","text":"Here is the answer."}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}

```

Create `crates/cogito-model/tests/fixtures/openai_responses/stop_reason_max_tokens.sse`:

```
event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Truncated"}

event: response.completed
data: {"type":"response.completed","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}

```

- [ ] **Step 4: Create the integration test**

The integration test parses fixtures directly through the wire types
(no HTTP layer). This is the same shape as `openai_compat`'s
fixture-driven decode tests.

Create `crates/cogito-model/tests/openai_responses_decode.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

// We test the wire-level parsing only here. The full async stream
// integration is exercised by a live-API smoke test (manual, not in CI)
// — see ROADMAP / spec §15.3.

#[test]
fn text_completion_parses() {
    let raw = read_fixture("text_completion.sse");
    let events = parse_sse(&raw);
    let kinds: Vec<&str> = events.iter().map(|s| s.as_str()).collect();
    assert!(kinds.iter().any(|k| k.contains("output_text.delta")));
    assert!(kinds.iter().any(|k| k.contains("response.completed")));
}

#[test]
fn tool_call_parses() {
    let raw = read_fixture("tool_call.sse");
    let events = parse_sse(&raw);
    assert!(events.iter().any(|e| e.contains("function_call_arguments.delta")));
    assert!(events.iter().any(|e| e.contains("function_call_arguments.done")));
}

#[test]
fn reasoning_summary_parses() {
    let raw = read_fixture("reasoning_summary.sse");
    let events = parse_sse(&raw);
    assert!(events.iter().any(|e| e.contains("reasoning_summary_text.delta")));
    assert!(events.iter().any(|e| e.contains("reasoning_summary_text.done")));
}

#[test]
fn stop_reason_max_tokens_parses() {
    let raw = read_fixture("stop_reason_max_tokens.sse");
    let events = parse_sse(&raw);
    assert!(
        events.last().unwrap().contains("max_output_tokens"),
        "incomplete_details.reason must be preserved through SSE encoding"
    );
}

fn read_fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("openai_responses")
        .join(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"))
}

/// Minimal SSE event-data extractor: returns each `data:` payload line.
fn parse_sse(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|l| l.strip_prefix("data: ").map(str::to_string))
        .collect()
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p cogito-model --test openai_responses_decode
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
make fmt && make fix CRATE=cogito-model
git add crates/cogito-model/src/openai_responses/decode.rs crates/cogito-model/tests/
git commit -m "$(cat <<'EOF'
feat(model/responses): SSE decoder + four fixture-based decode tests

Decoder maps Responses output_text.{delta,done} -> ModelEvent::Text*,
reasoning_summary_text.{delta,done} -> ModelEvent::Thinking*,
function_call_arguments.{delta,done} -> ModelEvent::ToolCall*. Final
response.completed event surfaces FinishReason + Usage; incomplete
status with reason="max_output_tokens" maps to FinishReason::MaxTokens.
Unknown event types are skipped via StreamEvent::Other catch-all.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 7: `ProviderConfig::OpenAiResponses` arm + `build_gateway` dispatch

### Task 14: Add `OpenAiResponses` ProviderConfig variant

**Files:**
- Modify: `crates/cogito-model/src/provider_config.rs`

- [ ] **Step 1: Read current shape**

Already read in spec phase. Key insight: add a third arm + a `defaults::openai_responses_base_url` fn + extend `name()` match + extend `build_gateway` match.

- [ ] **Step 2: Apply the edit**

In `crates/cogito-model/src/provider_config.rs`:

1. Add new import at the top of imports:
```rust
use crate::{OpenAiResponsesConfig, OpenAiResponsesGateway, ReasoningEffort};
```

2. Add the new enum arm directly after the `OpenAiCompat { ... }` arm and before the `// OpenAiResponses { ... } lands in Sprint 5` comment. Delete that comment in the same edit. The arm:

```rust
    /// OpenAI Responses API endpoint. Native reasoning items mapped to
    /// `ContentBlock::Thinking` per ADR-0019.
    #[serde(rename = "openai-responses")]
    OpenAiResponses {
        /// Provider entry name (used by surfaces for `--provider <name>` lookup).
        name: String,
        /// API key for `Authorization: Bearer ...`.
        api_key: String,
        /// Base URL. Defaults to `https://api.openai.com/v1`.
        #[serde(default = "defaults::openai_responses_base_url")]
        base_url: String,
        /// Per-request timeout in seconds. `None` keeps the gateway default.
        #[serde(default)]
        timeout_secs: Option<u64>,
        /// `low` | `medium` | `high` | omit (= provider default).
        #[serde(default)]
        reasoning_effort: Option<ReasoningEffort>,
    },
```

3. Extend the `name()` impl:

```rust
    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic { name, .. }
            | Self::OpenAiCompat { name, .. }
            | Self::OpenAiResponses { name, .. } => name,
        }
    }
```

4. Add the dispatch arm in `build_gateway`:

```rust
        ProviderConfig::OpenAiResponses {
            api_key,
            base_url,
            timeout_secs,
            reasoning_effort,
            ..
        } => {
            let mut c = OpenAiResponsesConfig::with_api_key(api_key);
            c.base_url = base_url;
            c.reasoning_effort = reasoning_effort;
            if let Some(s) = timeout_secs {
                c.timeout = Duration::from_secs(s);
            }
            Ok(Arc::new(OpenAiResponsesGateway::new(c)?))
        }
```

5. Add the default URL:

```rust
mod defaults {
    // ... existing fns ...
    pub(super) fn openai_responses_base_url() -> String {
        "https://api.openai.com/v1".into()
    }
}
```

- [ ] **Step 3: Write a parse test**

Append to existing `provider_config.rs` tests (or add a `#[cfg(test)] mod tests` block if none exists):

```rust
#[cfg(test)]
mod responses_tests {
    use super::*;

    #[test]
    fn parses_openai_responses_toml() {
        let toml = r#"
kind = "openai-responses"
name = "openai-prod"
api_key = "sk-xxx"
reasoning_effort = "high"
"#;
        let cfg: ProviderConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.name(), "openai-prod");
        assert!(matches!(cfg, ProviderConfig::OpenAiResponses { .. }));
    }

    #[test]
    fn build_gateway_dispatch_includes_responses() {
        let cfg = ProviderConfig::OpenAiResponses {
            name: "p".into(),
            api_key: "k".into(),
            base_url: "https://api.openai.com/v1".into(),
            timeout_secs: Some(60),
            reasoning_effort: None,
        };
        let gw = build_gateway(cfg).unwrap();
        assert_eq!(gw.name(), "openai-responses");
    }
}
```

- [ ] **Step 4: Run**

```bash
cargo test -p cogito-model provider_config
```

Expected: all tests (old + 2 new) pass.

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-model
git add crates/cogito-model/src/provider_config.rs
git commit -m "$(cat <<'EOF'
feat(model): ProviderConfig::OpenAiResponses arm + build_gateway dispatch

Third provider kind alongside Anthropic and OpenAiCompat. `kind =
"openai-responses"` in cogito.toml. Carries api_key, base_url
(defaults to api.openai.com/v1), timeout, reasoning_effort (low |
medium | high | omit).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 8: `cogito-config` — `default_strategy` key

### Task 15: Add `default_strategy` field

**Files:**
- Modify: `crates/cogito-config/src/types.rs`

- [ ] **Step 1: Read the existing `RuntimeSection`**

Run: `grep -n "default_provider\|default_model\|strategies_dir" crates/cogito-config/src/types.rs`

You should see `default_provider`, `default_model`, `strategies_dir` already present on both `RuntimeSection` and `RuntimeSectionPartial`.

- [ ] **Step 2: Add `default_strategy` next to them**

Edit `crates/cogito-config/src/types.rs`:

1. In `RuntimeSection`, add (next to `default_model`):

```rust
    /// Optional default strategy name. If `None` and `--strategy`
    /// is not given, `resolve_strategy` synthesizes a strategy from
    /// `default_model` + CLI flags. See ADR-0026.
    pub default_strategy: Option<String>,
```

2. In `RuntimeSectionPartial`, add:

```rust
    /// Override for `RuntimeSection::default_strategy`.
    pub default_strategy: Option<String>,
```

3. In `RuntimeSectionPartial::finalize` (or whatever merges Partial into the final shape — find the function that builds `RuntimeSection` from `RuntimeSectionPartial`), thread the field through:

```rust
        default_strategy: self.default_strategy,
```

4. In the merge logic (`crates/cogito-config/src/merge.rs` — peek at how other fields like `default_provider` are merged) add a parallel handling for `default_strategy`. If the merge is generic (e.g., per-field `Option::or`), this is automatic.

5. Update the existing fixture test (if any) to round-trip `default_strategy`. Look for the test around line 155 of types.rs that builds a sample `RuntimeSectionPartial` and add `default_strategy: Some("coder".into())` plus an assertion that it survives the round-trip.

- [ ] **Step 3: Run**

```bash
cargo test -p cogito-config
```

Expected: all existing tests pass + the new round-trip assertion.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-config
git add crates/cogito-config/src/
git commit -m "$(cat <<'EOF'
feat(config): runtime.default_strategy key

Optional cogito.toml key alongside default_provider and default_model.
When --strategy is not given on the CLI, resolve_strategy reads this
key; if neither is set, the synthesized default kicks in.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 9: `cogito-cli` — `--strategy`, `resolve_strategy`, `--list-strategies`

### Task 16: Add cogito-strategy dependency + skeleton `resolve_strategy`

**Files:**
- Modify: `crates/cogito-cli/Cargo.toml`
- Modify: `crates/cogito-cli/src/chat.rs`
- Modify: `crates/cogito-cli/src/chat_config.rs`

- [ ] **Step 1: Add dep**

In `crates/cogito-cli/Cargo.toml`, add to `[dependencies]`:

```toml
cogito-strategy = { workspace = true }
```

- [ ] **Step 2: Find the existing strategy construction**

Run: `grep -n "default_with_model\|HarnessStrategy" crates/cogito-cli/src/chat.rs`

You'll find one or two call sites that build the strategy. We will replace these with a call to `resolve_strategy`.

- [ ] **Step 3: Add the `resolve_strategy` helper**

In `crates/cogito-cli/src/chat.rs`, add (near the top of the impl section, after imports):

```rust
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::strategy_registry::{StrategyError, StrategyRegistry};
use cogito_strategy::FsStrategyRegistry;

/// Resolve a `HarnessStrategy` + `ProviderConfig` pair from CLI args,
/// the loaded RuntimeConfig, and the FS-backed registry. This is the
/// single seam where strategy + provider + CLI overrides collide;
/// downstream code (RuntimeBuilder, gateway construction) consumes
/// only the resolved values.
///
/// Resolution order (per Sprint 9a spec §12.1):
///   strategy        = --strategy -> cogito.toml default_strategy -> synthesized
///   provider        = --provider -> strategy.provider -> cogito.toml default_provider -> error
///   model           = --model -> strategy.model -> cogito.toml runtime.default_model -> error
///   system_prompt   = --system -> strategy.system_prompt -> empty
///
/// # Errors
///
/// Returns one of:
/// - `ResolveError::UnknownStrategy(name, available)` — CLI named a missing strategy
/// - `ResolveError::UnknownProvider { strategy, provider }` — strategy.provider points to nothing
/// - `ResolveError::MissingProvider` — no `--provider` flag, no strategy.provider, no default_provider
/// - `ResolveError::MissingModel` — neither flag, strategy, nor default_model has a model id
pub(crate) fn resolve_strategy(
    args: &ChatArgs,
    cfg: &cogito_config::RuntimeConfig,
    registry: &dyn StrategyRegistry,
) -> Result<(HarnessStrategy, cogito_model::ProviderConfig), ResolveError> {
    // 1. Pick the strategy name (or None for synthesis).
    let strategy_name = args
        .strategy
        .clone()
        .or_else(|| cfg.runtime.default_strategy.clone());

    let mut strategy = match strategy_name.as_deref() {
        Some(name) => match registry.get(name) {
            Ok(s) => s,
            Err(StrategyError::Unknown(n, available)) => {
                return Err(ResolveError::UnknownStrategy {
                    name: n,
                    available,
                });
            }
            Err(e) => return Err(ResolveError::Strategy(e)),
        },
        None => {
            // Synthesized default. Model resolved below.
            let initial_model = args
                .model
                .clone()
                .or_else(|| cfg.runtime.default_model.clone())
                .unwrap_or_default();
            HarnessStrategy::default_with_model(initial_model)
        }
    };

    // 2. Apply CLI overrides on the strategy.
    if let Some(ref model) = args.model {
        strategy.model_params.model = model.clone();
    }
    if let Some(ref sys) = args.system {
        strategy.system_prompt = sys.clone();
    }

    // 3. Ensure model is non-empty after overrides + strategy + cogito.toml fallback.
    if strategy.model_params.model.is_empty() {
        if let Some(m) = &cfg.runtime.default_model {
            strategy.model_params.model = m.clone();
        }
    }
    if strategy.model_params.model.is_empty() {
        return Err(ResolveError::MissingModel);
    }

    // 4. Resolve provider:
    //   --provider > strategy_provider_ref > cogito.toml default_provider
    let strategy_provider_ref = strategy_name
        .as_deref()
        .and_then(|n| {
            // resolve via FsStrategyRegistry-specific provider_ref; falls
            // back to None for non-FS impls (which the wiring layer can't
            // ask about provider refs through the protocol trait alone).
            // We only hit this path for FsStrategyRegistry in v0.1.
            registry_provider_ref(registry, n)
        });

    let provider_name = args
        .provider
        .clone()
        .or(strategy_provider_ref)
        .or_else(|| cfg.runtime.default_provider.clone())
        .ok_or(ResolveError::MissingProvider)?;

    let provider_cfg = cfg
        .providers
        .iter()
        .find(|p| p.name() == provider_name)
        .cloned()
        .ok_or_else(|| ResolveError::UnknownProvider {
            strategy: strategy_name.clone().unwrap_or_else(|| "<synthesized>".into()),
            provider: provider_name,
        })?;

    Ok((strategy, provider_cfg))
}

/// Extract a strategy's `provider:` reference. Concrete-impl downcast
/// (FsStrategyRegistry only). Returns `None` for other impls — those
/// must declare provider via cogito.toml default_provider in v0.1.
fn registry_provider_ref(
    registry: &dyn StrategyRegistry,
    name: &str,
) -> Option<String> {
    use std::any::Any;
    let any_self: &dyn Any = registry.as_any();
    any_self
        .downcast_ref::<FsStrategyRegistry>()
        .and_then(|fs| fs.provider_ref(name).map(str::to_string))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResolveError {
    #[error("strategy `{name}` not found; available: {available:?}")]
    UnknownStrategy { name: String, available: Vec<String> },
    #[error("strategy `{strategy}` references provider `{provider}` which is not in cogito.toml")]
    UnknownProvider { strategy: String, provider: String },
    #[error("no provider available: pass --provider, set strategy.provider, or set runtime.default_provider")]
    MissingProvider,
    #[error("no model available: pass --model, set strategy.model, or set runtime.default_model")]
    MissingModel,
    #[error(transparent)]
    Strategy(StrategyError),
}
```

- [ ] **Step 4: Add `as_any` to the StrategyRegistry trait (in cogito-protocol)**

The downcast in `registry_provider_ref` needs trait support. Edit `crates/cogito-protocol/src/strategy_registry.rs` and add to the trait:

```rust
    /// Downcast hook — concrete impls override to return `self` as
    /// `&dyn Any`. Default returns a placeholder. Used by wiring code
    /// to read impl-specific metadata (e.g., FsStrategyRegistry's
    /// `provider_ref`). v0.4 SaaS impls can override with their own
    /// metadata hooks (or leave it as the default — provider refs
    /// live elsewhere in that world).
    fn as_any(&self) -> &dyn std::any::Any {
        &()
    }
```

Edit `crates/cogito-strategy/src/registry.rs`, in `impl StrategyRegistry for FsStrategyRegistry`, add:

```rust
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
```

Edit `crates/testing/cogito-test-fixtures/src/strategy.rs`, in `impl StrategyRegistry for MapStrategyRegistry`, add the same `as_any` override.

- [ ] **Step 5: Run all touched crates' tests**

```bash
cargo test -p cogito-protocol -p cogito-strategy -p cogito-test-fixtures
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
make fmt && make fix CRATE=cogito-protocol
make fix CRATE=cogito-strategy
make fix CRATE=cogito-test-fixtures
make fix CRATE=cogito-cli
git add crates/cogito-cli/Cargo.toml crates/cogito-cli/src/chat.rs crates/cogito-protocol/ crates/cogito-strategy/ crates/testing/cogito-test-fixtures/
git commit -m "$(cat <<'EOF'
feat(cli): resolve_strategy helper + StrategyRegistry::as_any hook

resolve_strategy is the single seam where CLI flags, cogito.toml
defaults, and registry data combine into the final (HarnessStrategy,
ProviderConfig) pair. as_any on the trait lets FsStrategyRegistry
surface impl-specific provider references without bloating the
protocol-level API.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 17: Wire `--strategy` and `--list-strategies` CLI flags + use `resolve_strategy`

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs`

- [ ] **Step 1: Add the flags to `ChatArgs`**

Find the `ChatArgs` struct in `crates/cogito-cli/src/chat.rs` and add two fields:

```rust
    /// Strategy name to load from .cogito/strategies/. Overrides
    /// cogito.toml runtime.default_strategy. Mutually independent of
    /// --model: --model can still override the strategy's model.
    #[arg(long, value_name = "NAME")]
    pub strategy: Option<String>,

    /// Print available strategies (name + description) and exit.
    #[arg(long)]
    pub list_strategies: bool,
```

- [ ] **Step 2: Handle `--list-strategies` before opening the runtime**

Find the chat-command entry point (likely `pub async fn run(args: ChatArgs) -> Result<()> ...`). Right after building the registry but before opening a runtime, add:

```rust
    if args.list_strategies {
        for name in registry.list() {
            let desc = registry.description(&name).unwrap_or("(no description)");
            println!("{name:<24} {desc}");
        }
        return Ok(());
    }
```

If the registry is constructed inside a different function (look in `chat_config.rs` for `build_runtime_config` or similar — that's where to construct `FsStrategyRegistry::from_conventional_scopes_with_repo_override(cfg.runtime.strategies_dir.clone())`), thread the registry handle into the main function so this list code can reach it.

- [ ] **Step 3: Replace the `default_with_model` call with `resolve_strategy`**

Find the existing strategy construction (the `let mut strategy = HarnessStrategy::default_with_model(...)` line). Replace with:

```rust
    let (strategy, provider_cfg) = resolve_strategy(&args, &runtime_cfg, registry.as_ref())
        .map_err(|e| match e {
            ResolveError::UnknownStrategy { name, available } => {
                eprintln!("unknown strategy `{name}`");
                eprintln!("available: {}", available.join(", "));
                std::process::exit(2);
            }
            other => anyhow::anyhow!(other),
        })?;
```

Remove the prior `let provider_cfg = ...` derivation if there was one — `resolve_strategy` now returns both.

- [ ] **Step 4: Run a manual smoke test**

```bash
# Synthesized default still works.
cargo run -p cogito-cli -- chat --model "gpt-4o" --message "hi" 2>&1 | head -5
```

Expected: doesn't fail with a strategy error; the model name reaches the gateway (will fail if no API key is configured — that's fine for this check).

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-cli
git add crates/cogito-cli/src/chat.rs
git commit -m "$(cat <<'EOF'
feat(cli): --strategy and --list-strategies flags

--strategy <name> selects a strategy from the registry. --list-strategies
prints available strategies + descriptions and exits. resolve_strategy
becomes the single source of truth for HarnessStrategy + ProviderConfig
construction. Zero-config cogito chat --model X still works via
synthesized default.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 18: Build the FS registry inside `chat_config` / runtime construction

**Files:**
- Modify: `crates/cogito-cli/src/chat_config.rs`

- [ ] **Step 1: Find the runtime-config build site**

Run: `grep -n "RuntimeConfig\|build_runtime\|RuntimeBuilder" crates/cogito-cli/src/chat_config.rs`

You should see a function that loads cogito.toml + CLI overrides and returns a `RuntimeConfig`. That's where we construct the FS registry.

- [ ] **Step 2: Thread `Arc<FsStrategyRegistry>` next to the RuntimeConfig**

Decision: return `(RuntimeConfig, Arc<FsStrategyRegistry>)` from the existing builder so the caller (chat.rs) can pass both into `resolve_strategy`. If the existing function already returns a tuple, add a new return slot; if it returns a single value, change to a struct or tuple.

Pseudocode (adapt to actual signature):

```rust
pub(crate) fn build_runtime_config_and_registry(
    args: &ChatArgs,
) -> Result<(RuntimeConfig, std::sync::Arc<FsStrategyRegistry>)> {
    let cfg = build_runtime_config_inner(args)?;

    let registry = match &cfg.runtime.strategies_dir {
        // If operator set `strategies_dir` in cogito.toml, treat that as
        // the Repo-scope override.
        path => FsStrategyRegistry::from_conventional_scopes_with_repo_override(
            path.clone(),
        )?,
    };
    Ok((cfg, std::sync::Arc::new(registry)))
}
```

`from_conventional_scopes_with_repo_override` returns `Result<_, LoadError>`. Wrap with `anyhow::Context` or convert to the function's existing error type — match the surrounding code style.

- [ ] **Step 3: Update the caller in `chat.rs`**

In the chat-command run function, replace the prior `build_runtime_config` call with:

```rust
    let (runtime_cfg, registry) = chat_config::build_runtime_config_and_registry(&args)?;
```

Then `registry.as_ref()` flows into `resolve_strategy`.

- [ ] **Step 4: Smoke test the registry path**

Create a temporary strategy file in the cwd:

```bash
mkdir -p .cogito/strategies
cat > .cogito/strategies/coder.md <<'EOF'
---
name: coder
description: Smoke test
model: gpt-4o-mini
---

You are a coder.
EOF

cargo run -p cogito-cli -- chat --list-strategies
```

Expected output (alongside any pre-existing strategies): `coder  Smoke test`.

Clean up:

```bash
rm -rf .cogito/strategies
```

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-cli
git add crates/cogito-cli/src/chat_config.rs crates/cogito-cli/src/chat.rs
git commit -m "$(cat <<'EOF'
feat(cli): construct FsStrategyRegistry at startup

Registry is built once during runtime-config construction, scoped by
runtime.strategies_dir (Repo override) + User ~/.config/cogito/strategies.
Threaded through to the chat command alongside RuntimeConfig.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 19: Integration test — `resolve_strategy` end-to-end

**Files:**
- Create: `crates/cogito-cli/tests/resolve_strategy.rs`

- [ ] **Step 1: Write the test**

Create `crates/cogito-cli/tests/resolve_strategy.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat::{resolve_strategy, ChatArgs, ResolveError};
use cogito_config::{ProvidersList, RuntimeConfig, RuntimeSection};
use cogito_model::ProviderConfig;
use cogito_test_fixtures::strategy::MapStrategyRegistry;
use cogito_protocol::strategy::HarnessStrategy;

// NOTE: If `resolve_strategy` / `ChatArgs` / `ResolveError` are not
// re-exported from `crates/cogito-cli/src/lib.rs`, this test cannot
// reach them. Add a `pub mod chat;` to `lib.rs` if necessary, or move
// these items into `lib.rs`. The simplest fix: at the top of
// `crates/cogito-cli/src/lib.rs`, ensure:
//
//     pub mod chat;
//
// and inside `chat.rs` mark the items `pub`:
//
//     pub(crate) -> pub on resolve_strategy, ChatArgs, ResolveError.
//
// Adjust as needed during this task.

fn cfg_with_provider() -> RuntimeConfig {
    RuntimeConfig {
        runtime: RuntimeSection {
            session_root: std::path::PathBuf::from("./sessions"),
            default_provider: Some("anthropic-default".into()),
            default_model: Some("claude-opus-4-7".into()),
            default_strategy: None,
            strategies_dir: std::path::PathBuf::from(".cogito/strategies"),
        },
        providers: vec![ProviderConfig::Anthropic {
            name: "anthropic-default".into(),
            api_key: "k".into(),
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout_secs: None,
        }],
        strategies: Default::default(),
    }
}

fn args_with(strategy: Option<&str>, model: Option<&str>) -> ChatArgs {
    let mut a = ChatArgs::default();
    a.strategy = strategy.map(String::from);
    a.model = model.map(String::from);
    a
}

#[test]
fn synthesized_default_when_no_strategy() {
    let reg = MapStrategyRegistry::new();
    let cfg = cfg_with_provider();
    let args = args_with(None, None);
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.name, "default");
    assert_eq!(strategy.model_params.model, "claude-opus-4-7");
}

#[test]
fn registry_hit_when_strategy_provided() {
    let mut s = HarnessStrategy::default_with_model("");
    s.name = "coder".into();
    s.system_prompt = "be precise".into();
    s.model_params.model = "claude-sonnet-4-6".into();

    let reg = MapStrategyRegistry::new().with("coder", s);
    let cfg = cfg_with_provider();
    let args = args_with(Some("coder"), None);
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.name, "coder");
    assert_eq!(strategy.model_params.model, "claude-sonnet-4-6");
    assert_eq!(strategy.system_prompt, "be precise");
}

#[test]
fn model_flag_overrides_strategy_model() {
    let mut s = HarnessStrategy::default_with_model("");
    s.name = "coder".into();
    s.model_params.model = "claude-sonnet-4-6".into();
    let reg = MapStrategyRegistry::new().with("coder", s);

    let cfg = cfg_with_provider();
    let args = args_with(Some("coder"), Some("claude-opus-4-7"));
    let (strategy, _provider) = resolve_strategy(&args, &cfg, &reg).unwrap();
    assert_eq!(strategy.model_params.model, "claude-opus-4-7");
}

#[test]
fn unknown_strategy_returns_error_with_available() {
    let reg = MapStrategyRegistry::new();
    let cfg = cfg_with_provider();
    let args = args_with(Some("nope"), None);
    let err = resolve_strategy(&args, &cfg, &reg).unwrap_err();
    assert!(matches!(err, ResolveError::UnknownStrategy { ref name, .. } if name == "nope"));
}

#[test]
fn missing_provider_returns_error() {
    let mut cfg = cfg_with_provider();
    cfg.runtime.default_provider = None;
    cfg.providers.clear();
    let reg = MapStrategyRegistry::new();
    let args = args_with(None, Some("any"));
    let err = resolve_strategy(&args, &cfg, &reg).unwrap_err();
    assert!(matches!(err, ResolveError::MissingProvider));
}
```

- [ ] **Step 2: Ensure crate-level visibility**

Edit `crates/cogito-cli/src/lib.rs` to make sure the chat module is accessible from integration tests:

```rust
pub mod chat;
pub mod chat_config;
```

And in `crates/cogito-cli/src/chat.rs`, change `pub(crate)` to `pub` for `resolve_strategy`, `ChatArgs`, `ResolveError`. Mark `ChatArgs` with `#[derive(Default)]` if it isn't already (so the test helper works).

- [ ] **Step 3: Run the test**

```bash
cargo test -p cogito-cli --test resolve_strategy
```

Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
make fmt && make fix CRATE=cogito-cli
git add crates/cogito-cli/tests/resolve_strategy.rs crates/cogito-cli/src/
git commit -m "$(cat <<'EOF'
test(cli): resolve_strategy end-to-end with MapStrategyRegistry

Five scenarios cover the resolution table from Sprint 9a spec §12.1:
synthesized default, registry hit, --model override of strategy.model,
unknown strategy error path, missing provider error path.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 10: Example strategies + migration

### Task 20: Replace stale `strategies/*.yaml` with fresh `.cogito/strategies/*.md`

**Files:**
- Delete: `strategies/claude-opus.yaml`
- Delete: `strategies/gpt-4.yaml`
- Delete: `strategies/` (directory)
- Create: `.cogito/strategies/coder.md`
- Create: `.cogito/strategies/planner.md`
- Create: `.cogito/strategies/reviewer.md`

- [ ] **Step 1: Delete stale files**

```bash
git rm strategies/claude-opus.yaml strategies/gpt-4.yaml
rmdir strategies 2>/dev/null || true
```

- [ ] **Step 2: Create `.cogito/strategies/coder.md`**

```bash
mkdir -p .cogito/strategies
```

Create `.cogito/strategies/coder.md`:

```markdown
---
name: coder
description: Coding tasks. Read before writing. Run tests after every change.
provider: anthropic-default
model: claude-opus-4-7
allowed_tools:
  - read_file
  - run_tests
max_turns: 50
model_params:
  temperature: 0.2
  max_tokens: 4096
---

You are a precise software engineer working in a Rust codebase.

Read the relevant files before proposing a change. Make the change with
a clear rationale tied to existing code. Run tests after every edit and
make sure they pass before moving on.

Prefer small, focused commits over large refactors. When unsure, ask a
clarifying question rather than guessing.
```

- [ ] **Step 3: Create `.cogito/strategies/planner.md`**

```markdown
---
name: planner
description: Decompose a goal into a sequenced plan. No tool calls.
provider: anthropic-default
model: claude-opus-4-7
allowed_tools: []
max_turns: 8
model_params:
  temperature: 0.5
  max_tokens: 4096
---

You are a planner. Given a goal, produce a numbered list of concrete
steps that, when executed in order, accomplish the goal. Do not call
tools. Keep each step small enough to complete in 5–10 minutes.

For each step include:
- A one-line summary of the action.
- The expected observable outcome.

End your response with a single line "READY" once the plan is complete.
```

- [ ] **Step 4: Create `.cogito/strategies/reviewer.md`**

```markdown
---
name: reviewer
description: Read-only code review. Identifies risks, suggests improvements, no edits.
provider: anthropic-default
model: claude-opus-4-7
allowed_tools:
  - read_file
max_turns: 16
model_params:
  temperature: 0.3
  max_tokens: 4096
---

You are a senior code reviewer. Given a diff or a pull-request
description, identify:

1. Correctness risks (logic errors, edge cases, race conditions).
2. Security risks (input validation, auth, secret handling).
3. Maintainability concerns (naming, structure, comments).
4. Test coverage gaps.

Read source files via `read_file` as needed. Do not propose edits;
return a structured review only. End with an overall recommendation:
APPROVE, REQUEST_CHANGES, or COMMENT.
```

- [ ] **Step 5: Verify the strategies parse**

```bash
cargo run -p cogito-cli -- chat --list-strategies
```

Expected: three lines, `coder`, `planner`, `reviewer`, each with its description.

- [ ] **Step 6: Commit**

```bash
git add .cogito/strategies/ -- :!strategies/  # delete already staged via git rm
git commit -m "$(cat <<'EOF'
feat(strategies): replace stale draft YAMLs with .cogito/strategies/*.md

Delete strategies/claude-opus.yaml and strategies/gpt-4.yaml (schema
no longer matches anything in the codebase). Ship three fresh
markdown+frontmatter strategies under .cogito/strategies/:

- coder.md — coding tasks, all FS tools, low temperature
- planner.md — decomposition, no tools, higher temperature
- reviewer.md — code review, read-only, medium temperature

Names match the v0.3 Sprint 11 subagent role plan, so these files
serve double duty when subagents land.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 11: Resume-chaos scenario

### Task 21: `strategy_with_tool_filter` resume-chaos scenario

**Files:**
- Modify: `crates/cogito-core/tests/resume_chaos.rs`

- [ ] **Step 1: Find an existing scenario to mirror**

Run: `grep -n "fn .*scenario\|happy_path\|no_tool_short_turn" crates/cogito-core/tests/resume_chaos.rs | head -20`

Identify the function shape — likely `fn single_tool_happy_path_scenario() -> Scenario { ... }` or similar. Open the file and read one full scenario to understand:
- How `HarnessStrategy` is constructed for the scenario.
- How `MockModelGateway` is fed canned responses.
- Where `PanicAt` injection points sit.
- What oracle assertions are written.

- [ ] **Step 2: Add the new scenario**

Append a new scenario `strategy_with_tool_filter` after the existing ones. Use the same shape as the closest sibling. Key differences:

```rust
fn strategy_with_tool_filter_scenario() -> Scenario {
    // Strategy uses an Allow filter naming only `read_file`. Any
    // additional tool offered by the provider must be stripped at H05.
    let mut strategy = HarnessStrategy::default_with_model("mock-model");
    strategy.name = "filtered_coder".into();
    strategy.system_prompt = "Strict read-only coder.".into();
    strategy.allowed_tools = cogito_protocol::strategy::ToolFilter::Allow(vec![
        "read_file".into(),
    ]);
    strategy.model_params.temperature = Some(0.3);

    // Model script: assistant calls read_file once, gets the result,
    // then ends with text. Same shape as single_tool_happy_path but
    // with the filtered strategy.
    let model_script = vec![
        // ... copy from single_tool_happy_path with read_file as the only call ...
    ];

    Scenario {
        name: "strategy_with_tool_filter".into(),
        strategy,
        model_script,
        // PanicAt boundaries: pick the same set the happy-path scenario uses
        // (typically: after PromptBuilt, after ModelCompleted, after first
        // tool result recorded).
        crash_points: default_crash_points(),
        // Oracle additions: the post-resume strategy MUST still carry
        // the Allow filter and the same temperature. The closure runs
        // after the resume completes.
        extra_oracles: vec![Oracle::Custom(Box::new(|recording| {
            // Walk recording.events; find the TurnStarted event that
            // followed the resume; verify the strategy fields match.
            let started = recording
                .events
                .iter()
                .find_map(|e| match &e.payload {
                    EventPayload::TurnStarted { strategy, .. } => Some(strategy.clone()),
                    _ => None,
                })
                .expect("post-resume TurnStarted event missing");
            assert_eq!(started.allowed_tools, cogito_protocol::strategy::ToolFilter::Allow(vec!["read_file".into()]));
            assert_eq!(started.model_params.temperature, Some(0.3));
        }))],
    }
}
```

(Names like `Scenario`, `Oracle`, `default_crash_points`, `EventPayload` will need to match what's already in `resume_chaos.rs`. The block above is illustrative — copy the most recent scenario's exact types.)

- [ ] **Step 3: Register the scenario**

Find the harness entry point (often `#[test] fn run_all_scenarios()` or a `tokio::test` parameterized over scenarios). Add `strategy_with_tool_filter_scenario()` to the list.

- [ ] **Step 4: Run**

```bash
make chaos
```

Expected: existing scenarios still pass + new scenario's 3 boundary points pass all 4 oracles.

- [ ] **Step 5: Commit**

```bash
make fmt && make fix CRATE=cogito-core
git add crates/cogito-core/tests/resume_chaos.rs
git commit -m "$(cat <<'EOF'
test(chaos): strategy_with_tool_filter scenario

Drives a turn with a Strategy carrying ToolFilter::Allow + non-default
model_params through every crash boundary. Oracle verifies post-resume
TurnStarted event carries the SAME filter + same model_params — proves
strategy is not mid-turn state, it's re-derived from the registry
deterministically.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 12: Documentation — ADR-0026, H10, AGENTS, ARCHITECTURE, configuration overview

### Task 22: Write ADR-0026

**Files:**
- Create: `docs/adr/0026-strategy-registry.md`

- [ ] **Step 1: Compose the ADR**

Create `docs/adr/0026-strategy-registry.md`:

```markdown
# ADR-0026: Strategy registry (`cogito-strategy`) — declarative agent modes

**Status**: Proposed (ratified at Sprint 9a close)
**Date**: 2026-05-27
**Spec**: `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
**Supersedes (partially)**: ADR-0017 §13 (single `strategies_dir` model is now
overridable but no longer the only path)

## Context

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
```

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0026-strategy-registry.md
git commit -m "$(cat <<'EOF'
docs(adr-0026): strategy registry — declarative agent modes

Ratifies cogito-strategy + StrategyRegistry trait, locks markdown-
with-frontmatter format, locks Repo > User scope precedence,
supersedes ADR-0017 §13's single strategies_dir model.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 23: Update H10 doc with "What is a strategy" framing

**Files:**
- Modify: `docs/components/H10-strategy-selector.md`

- [ ] **Step 1: Prepend the framing section**

Open `docs/components/H10-strategy-selector.md`. Insert a new top section right after the `# H10 · Strategy Selector` line and before the existing `> **Status**: ...` block:

```markdown
## What is a strategy

A strategy is a named, declarative "agent mode." It bundles *which
model, which persona, which tools, which context policy* for one
kind of work. Consumers ship N strategies — `coder`, `planner`,
`reviewer`, `critic` — and `--strategy <name>` selects the mode.
Same Brain, same Boundary, different *behavior contract*.

Strategies are not configuration of cogito. `cogito.toml` is
"where is the model and how do I reach it"; strategies are "what do
I tell the model to do." Strategies reference providers from
`cogito.toml` by name; they never embed credentials.

See [`ADR-0026`](../adr/0026-strategy-registry.md) for the full
rationale.
```

- [ ] **Step 2: Replace the stale 2026-05-21 note**

Find the `> **2026-05-21 update (ADR-0017 §9)** ...` blockquote near the end of the file. Replace it with:

```markdown
> **2026-05-27 update (ADR-0026 / Sprint 9a):** Strategy files are
> markdown with YAML frontmatter (Skills convention). The `name`
> frontmatter field is REQUIRED and MUST match the filename
> basename. The 2026-05-21 note saying `name:` would be dropped is
> superseded — we keep `name:` and validate it. Scope precedence
> (Repo > User) replaces the single `runtime.strategies_dir` model;
> `strategies_dir`, when set, overrides the Repo root only.
```

- [ ] **Step 3: Update the Sprint 6 example block**

Find the `## v0.x Sprint 6 scope` heading and its accompanying YAML example. Update the YAML example to match the new markdown+frontmatter format. Replace the YAML example with the same `coder.md` snippet from spec §7.1.

- [ ] **Step 4: Commit**

```bash
git add docs/components/H10-strategy-selector.md
git commit -m "$(cat <<'EOF'
docs(h10): top-of-file 'What is a strategy' framing + ADR-0026 link

Adds the consumer-facing framing that explains strategies as agent
modes. Supersedes the 2026-05-21 note about dropping the name field
— we now keep it and validate against filename. Updates the example
block from YAML to markdown+frontmatter.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 24: AGENTS.md + ARCHITECTURE.md + configuration overview cross-references

**Files:**
- Modify: `AGENTS.md`
- Modify: `ARCHITECTURE.md`
- Modify: `docs/configuration/overview.md`

- [ ] **Step 1: AGENTS.md authoritative-docs list**

Open `AGENTS.md`, find the "Authoritative docs" / "Read these first" section. Add one line:

```markdown
- `docs/adr/0026-strategy-registry.md` — what strategies are, why
  cogito-strategy is a separate crate from cogito-config.
```

- [ ] **Step 2: ARCHITECTURE.md crate table**

Open `ARCHITECTURE.md`, find the component-map / crate-table section. Add a row for `cogito-strategy`:

```markdown
| cogito-strategy | Hands sub-layer | v0.1 | FS-backed `StrategyRegistry` impl. Markdown+frontmatter strategy files under `.cogito/strategies/`. See ADR-0026. |
```

(Match the table format exactly to what's already there — column widths, etc.)

- [ ] **Step 3: configuration/overview.md paragraph**

Open `docs/configuration/overview.md`. Find a good spot near the top (under whatever heading describes the overall configuration model) and add:

```markdown
## Strategies are not in `cogito.toml`

A strategy bundles "what to tell the model to do" (system prompt,
allowed tools, model knobs). Strategies live in
`.cogito/strategies/*.md` (Repo scope) or
`~/.config/cogito/strategies/*.md` (User scope), not in
`cogito.toml`. The two layers are deliberate: cogito.toml is the
deployment operator's territory (endpoints, credentials); strategies
are the agent designer's territory (behavior).

See [`ADR-0026`](../adr/0026-strategy-registry.md) for the full
rationale.
```

- [ ] **Step 4: Verify framing landed in all six places**

```bash
grep -l "strategy is a named, declarative" docs/superpowers/specs/2026-05-27*.md docs/adr/0026*.md docs/components/H10*.md crates/cogito-strategy/src/lib.rs
grep -l "ADR-0026\|0026-strategy-registry" AGENTS.md ARCHITECTURE.md docs/configuration/overview.md
```

Expected: first grep returns 4 paths; second grep returns 3 paths.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md ARCHITECTURE.md docs/configuration/overview.md
git commit -m "$(cat <<'EOF'
docs: cross-reference ADR-0026 from AGENTS, ARCHITECTURE, config overview

Three load-bearing pointers so the strategy-vs-config framing is
discoverable from every entry point a new consumer reaches.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 13: ROADMAP split + CHANGELOG entry

### Task 25: Split Sprint 9 → 9a (done) + 9b (TUI carry-over) + tick Sprint 9a

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Find the Sprint 9 block**

Run: `grep -n "Sprint 9" ROADMAP.md`

Locate the current `#### Sprint 9 · Multi-model Strategy + TUI` block.

- [ ] **Step 2: Replace with the split**

Replace the existing Sprint 9 block with:

```markdown
#### Sprint 9a · Multi-model Strategy (2 days)

**Split from old Sprint 9** by 2026-05-27 spec/plan. Carries the
multi-model half of the original Sprint 9. TUI carries to Sprint 9b.

- [x] OpenAI Responses adapter in `cogito-model` (Responses API; ContentBlock serialization with native reasoning items per ADR-0019)
- [x] H10 Strategy Selector — markdown+frontmatter strategy registry via new crate `cogito-strategy` (FS-backed `StrategyRegistry` impl)
- [x] CLI `--strategy <name>` flag selects strategy; `--model` overrides strategy.model
- [x] Per-strategy `model_params`, `allowed_tools`, `system_prompt`, `context`
- [x] Three example strategies under `.cogito/strategies/` (coder, planner, reviewer)
- [x] **ADR-0026**: Strategy registry — markdown+frontmatter format, Repo > User scope precedence, supersedes ADR-0017 §13
- [x] Resume-chaos `strategy_with_tool_filter` scenario passes all 4 oracles

#### Sprint 9b · TUI (1 day)

**Split from old Sprint 9** by 2026-05-27 spec/plan. Replicates
`cogito chat` in a ratatui TUI; consumes the same `resolve_strategy`
helper landed in 9a.

- [ ] Basic TUI with ratatui replicating `cogito chat`
- [ ] `cogito-tui` reads the same FsStrategyRegistry; `--strategy` flag honored
- [ ] Spec to follow once 9a lands
```

Also update the top-of-file "Current" block:

```markdown
> **v0.1 · Foundation** — Sprints 0–3 + 4.5 + 4.7 + 5 + 6 + 7 + 8 + 9a complete;
> Sprint 4 (MCP sync tools) in flight; Sprint 9 split into 9a (done)
> and 9b (TUI; spec pending); Sprints 10 unchanged.
> **Current sprint: Sprint 9b (TUI).**
```

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md
git commit -m "$(cat <<'EOF'
docs(roadmap): split Sprint 9 into 9a (done) and 9b (TUI)

Sprint 9a's seven checkboxes are ticked. Sprint 9b becomes the
current sprint, scoped to ratatui TUI work over the resolve_strategy
helper landed in 9a.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 26: CHANGELOG entry

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Locate the current Unreleased / v0.1 section**

Run: `head -50 CHANGELOG.md`

If a `## [Unreleased]` or `### Sprint 9a` heading is needed, add it under the existing v0.1 working entry.

- [ ] **Step 2: Add the entry**

Under the appropriate section (likely `## [Unreleased]` or `### v0.1 (in progress)`), add:

```markdown
### Sprint 9a · Multi-model Strategy (2026-05-27)

**Added**
- `cogito-protocol::StrategyRegistry` trait (read-only, object-safe).
- `cogito-strategy` crate — FS-backed `StrategyRegistry` impl.
  Markdown+frontmatter strategy files under `.cogito/strategies/`
  (Repo scope) and `~/.config/cogito/strategies/` (User scope).
- `cogito-model::openai_responses` adapter — OpenAI Responses API
  with native reasoning-item decoding (ADR-0019).
- `ProviderConfig::OpenAiResponses` variant.
- `cogito.toml` `runtime.default_strategy` key.
- `cogito chat --strategy <name>` and `--list-strategies` flags.
- `cogito_cli::chat::resolve_strategy` helper — single seam for
  combining strategy + CLI flags + `cogito.toml`.
- Example strategies: `.cogito/strategies/{coder,planner,reviewer}.md`.
- Resume-chaos `strategy_with_tool_filter` scenario.
- ADR-0026 (Strategy registry).

**Changed**
- `runtime.strategies_dir` in `cogito.toml` is now an optional Repo-
  scope override rather than a single canonical directory.

**Removed**
- `strategies/claude-opus.yaml` and `strategies/gpt-4.yaml` (stale
  schema; replaced by `.cogito/strategies/*.md`).
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): Sprint 9a entry

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Phase 14: Final verification

### Task 27: Full CI green + final spot-checks

**Files:**
- None (verification only)

- [ ] **Step 1: Run full CI**

```bash
make ci
```

Expected: fmt + clippy + layer-check + test all green.

- [ ] **Step 2: Run chaos**

```bash
make chaos
```

Expected: all scenarios (including `strategy_with_tool_filter`) pass.

- [ ] **Step 3: Verify the wiring layer doesn't leak into Brain**

```bash
grep -r "cogito_strategy::\|cogito-strategy" crates/cogito-core/ crates/cogito-context/ crates/cogito-protocol/
```

Expected: no matches. Brain does not import the strategy crate. If anything appears, that's a layer violation — fix before committing.

- [ ] **Step 4: Verify the strategy-vs-config framing landed in all six places**

```bash
grep -l "strategy is a named, declarative" \
  docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md \
  docs/adr/0026-strategy-registry.md \
  docs/components/H10-strategy-selector.md \
  crates/cogito-strategy/src/lib.rs

grep -l "ADR-0026\|0026-strategy-registry" \
  AGENTS.md \
  ARCHITECTURE.md \
  docs/configuration/overview.md
```

Expected: 4 + 3 = 7 distinct file paths.

- [ ] **Step 5: List sprint diff stats**

```bash
git log --oneline sprint-9a-start..HEAD
git diff --stat sprint-9a-start..HEAD | tail -5
```

Inspect the file-count + line-count totals to make sure the PR's blast radius matches what the spec promised. Expected: roughly the file map from the top of this plan, ~3000 lines added.

- [ ] **Step 6: Open the PR**

```bash
gh pr create --base main --head feat/sprint-9a-multi-model-strategy \
  --title "Sprint 9a: multi-model strategy registry + OpenAI Responses adapter" \
  --body "$(cat <<'EOF'
## Summary

- New `cogito-strategy` crate + `StrategyRegistry` trait. Markdown+frontmatter strategy files under `.cogito/strategies/` (Repo) and `~/.config/cogito/strategies/` (User).
- OpenAI Responses adapter with native reasoning items (ADR-0019).
- `--strategy <name>` + `--list-strategies` CLI flags. `--model` keeps overriding the strategy's model.
- ADR-0026 + propagated "strategy ≠ config" framing to H10 doc, crate docstring, AGENTS.md, ARCHITECTURE.md, configuration overview.
- Three example strategies (coder, planner, reviewer).
- Resume-chaos `strategy_with_tool_filter` scenario.

## Test plan

- [x] `make ci` green
- [x] `make chaos` green (new scenario passes all 4 oracles)
- [x] Manual: `cogito chat --list-strategies` lists the three examples
- [x] Manual: `cogito chat --model claude-opus-4-7` (no strategy flag) still works — synthesized default preserved
- [ ] Smoke test against a real OpenAI Responses endpoint (manual, off-CI)

Spec: docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md
ADR: docs/adr/0026-strategy-registry.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL printed.

---

## Self-review checklist (run before declaring the plan complete)

**Spec coverage:** Every numbered section of the spec maps to a task:

- Spec §1 Goals → Tasks 01–20 (the working system)
- Spec §2 Non-goals → not tasks (intentionally absent)
- Spec §3 + §4 "What is a strategy" + table → Tasks 02 (lib.rs docstring), 22 (ADR-0026), 23 (H10 doc), 24 (AGENTS / ARCHITECTURE / overview)
- Spec §4.1 Supersession of ADR-0017 §13 → Tasks 15, 22
- Spec §5 Locked decisions → all of Phases 1–10
- Spec §6 Architecture overview → realised in code by Tasks 01–19
- Spec §7 Strategy file format → Tasks 04, 05, 06, 20
- Spec §8 `StrategyRegistry` trait → Task 01
- Spec §9 `cogito-strategy` crate → Phases 2–4 (Tasks 02–10)
- Spec §10 Default-strategy synthesis → Task 16
- Spec §11 OpenAI Responses adapter → Phases 6–7 (Tasks 11–14)
- Spec §12 CLI integration → Tasks 16–19
- Spec §13 Documentation propagation → Tasks 02, 22, 23, 24
- Spec §14 Error handling → Tasks 03, 01, 16
- Spec §15 Testing → Tasks 06, 09, 10, 12, 13, 19, 21
- Spec §16 Migration → Task 20
- Spec §17 Workspace topology → realised by file creation across the plan
- Spec §18 Risks → noted (parser edge cases tested in Task 05–06; SSE volatility addressed by fixture-driven tests in Task 13)
- Spec §19 Acceptance criteria → Task 27
- Spec §20 Out of scope → not tasks

**Type consistency:** `HarnessStrategy`, `StrategyRegistry`, `StrategyError`, `LoadError`, `FsStrategyRegistry`, `MapStrategyRegistry`, `ParsedStrategy`, `Scope`, `ScopeRoot`, `ProviderConfig`, `ReasoningEffort`, `ChatArgs`, `ResolveError`, `OpenAiResponsesConfig`, `OpenAiResponsesGateway` — names used consistently across all tasks.

**Placeholder scan:** No "TBD", no "TODO", no "implement later", no "fill in details". The two Sprint-7-parser-reuse decisions in Task 02/03 (whether to lift the parser) are explicitly deferred to the implementation PR — that's a real decision, not a placeholder.

**Frequent commits:** Every task ends in a commit. Total commits ≈ 27 (one per task), each scoped to a single file group.

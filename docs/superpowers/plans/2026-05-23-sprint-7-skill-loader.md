# Sprint 7 · Skill Loader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a userland skill loader (`cogito-skills` crate) so team members extend agent behaviour via `SKILL.md` markdown files; the model activates via `$Name` sigils and users via `/skill <name>`.

**Architecture:** New Hands-layer crate `cogito-skills` exposes `SkillRegistry: SkillProvider` from a filesystem scan of Repo + User scopes. Sprint 6's `SystemPromptInjector` is implemented by a new `SkillInjector` (in `cogito-context`) that re-derives model-channel activations from previous-turn text blocks, reads user-channel names from `TurnStarted.activate_skills`, dedupes against prior `SkillActivated` events, and emits `SystemPromptInjected` with an "Available Skills" registry block plus XML-wrapped activated bodies. H06 gains a code-fence-aware sigil detector that surfaces `StreamEvent::SkillActivationRequested`. CLI parses `/skill <name> [text]` into a new `TurnTrigger::SkillActivation` variant.

**Tech Stack:** Rust 2024 (MSRV 1.85), `regex` 1, `serde_yaml`, `walkdir`, `async-trait`, `thiserror`, `tracing`. Tests via `cargo nextest`; resume-chaos via existing `cogito-test-fixtures` harness.

**Branch:** `feat/sprint-7-skills` (created from `main` after PR #21 merge).

**Spec:** `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md`

---

## File map

### Create

```
crates/cogito-skills/
  Cargo.toml
  src/
    lib.rs           # SkillRegistry, ScanConfig, SkillRegistryError, pub use of submodules
    metadata.rs      # YAML frontmatter parser + SKILL.md split
    sigil.rs         # Sigil regex + FenceState code-fence-aware scanner
    discovery.rs     # Scope-based filesystem walker (repo walk-up + user dir)
    registry.rs      # SkillRegistry impl of SkillProvider
  tests/
    metadata.rs
    sigil.rs
    discovery.rs
    fixtures/
      .cogito/skills/repo-foo/SKILL.md
      .cogito/skills/repo-bar/SKILL.md
      user-home/.cogito/skills/user-baz/SKILL.md

crates/cogito-protocol/src/skill.rs             # SkillProvider trait + metadata/content/source types
crates/cogito-context/src/injector/skill.rs     # SkillInjector impl of SystemPromptInjector
crates/cogito-context/tests/skill_injector.rs   # Unit tests for SkillInjector
crates/cogito-core/tests/h06_skill_sigil_detection.rs   # H06 streaming sigil detection
crates/cogito-core/tests/h11_skill_injection.rs         # End-to-end H11 with SkillInjector
crates/cogito-core/tests/turn_driver_skill_activation_user.rs   # User-channel projection test
crates/cogito-cli/tests/slash_skill.rs                  # CLI slash parser
crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl  # New canonical fixture
docs/skills/overview.md                                  # Hands-layer doc
```

### Modify

```
Cargo.toml                                              # add member, workspace dep
crates/cogito-protocol/Cargo.toml                       # add serde-related deps already there
crates/cogito-protocol/src/lib.rs                       # pub mod skill; re-export
crates/cogito-protocol/src/event.rs                     # SkillActivated variant + TurnStarted.activate_skills field
crates/cogito-protocol/src/turn_trigger.rs              # TurnTrigger::SkillActivation variant + reservation comment update
crates/cogito-protocol/src/stream.rs                    # StreamEvent::SkillActivationRequested variant
crates/cogito-protocol/src/store.rs                     # EventRecorder::record_skill_activated default-impl method
crates/cogito-protocol/src/context.rs                   # SystemPromptInjectorConfig::Skill variant
crates/cogito-context/Cargo.toml                        # dep on cogito-skills NOT needed (trait via protocol only)
crates/cogito-context/src/injector/mod.rs               # pub mod skill;
crates/cogito-context/src/pipeline.rs                   # build_pipeline takes SkillProvider arg
crates/cogito-core/src/harness/turn_driver/deps.rs      # TurnDeps.skills: Option<Arc<dyn SkillProvider>>
crates/cogito-core/src/harness/stream_demux.rs          # FenceState + sigil emit
crates/cogito-core/src/runtime/builder.rs               # RuntimeBuilder::skills(...) + pass into build_pipeline
crates/cogito-core/src/runtime/session_loop.rs          # try_start_turn projection for SkillActivation
crates/cogito-cli/src/chat.rs                           # slash parser
crates/cogito-cli/src/chat_config.rs                    # [skills] section parse + SkillRegistry construction
crates/cogito-config/src/types.rs                       # SkillsConfig + cogito.toml shape
crates/cogito-core/tests/resume_chaos.rs                # text_then_skill_then_tool scenario
crates/testing/cogito-test-fixtures/src/lib.rs          # re-export new skill helpers
crates/testing/cogito-test-fixtures/src/context.rs      # InMemoryRecorder.record_skill_activated default works via append_payload
docs/data-model/jsonl-v1.md                             # activate_skills + skill_activated entries
docs/schemas/conversation-event-v1.json                 # regen via cargo run -p cogito-gen-schema
docs/adr/0020-skill-loader.md                           # promote Proposed → Accepted
docs/components/H06-stream-demux.md                     # sigil detection side-channel
docs/components/H11-context-manage.md                   # SkillInjector mentioned alongside NoneInjector
ROADMAP.md                                              # tick Sprint 7 checkboxes
CHANGELOG.md                                            # v0.1 / Sprint 7 entry
```

---

## Conventions

- Every commit runs `make fmt && make fix CRATE=<name>` first.
- Every task ends with `make test CRATE=<name>` green and a commit on `feat/sprint-7-skills`.
- Test files use `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` consistent with the existing codebase.
- All code comments and doc comments in English (CLAUDE.md mandate).
- No decorative numerals or markers (no `①`, `★`, etc.); plain `1.` and `-`.
- Use `tracing::warn!` / `tracing::debug!` — never `println!`.

---

## Phase 0: Crate scaffold

### Task 01: Create empty `cogito-skills` crate + workspace wire-up

**Files:**
- Create: `crates/cogito-skills/Cargo.toml`
- Create: `crates/cogito-skills/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members` + `cogito-skills` entry in `[workspace.dependencies]`)

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-skills/src/lib.rs`:
```rust
//! Hands-layer Skill loader. See `docs/skills/overview.md` and
//! `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md`.

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn crate_compiles() {
        // Placeholder; later tasks add real surface.
    }
}
```

Create `crates/cogito-skills/Cargo.toml`:
```toml
[package]
name = "cogito-skills"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
cogito-protocol = { workspace = true }
serde = { workspace = true }
serde_yaml = { workspace = true }
regex = { workspace = true }
walkdir = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

Modify root `Cargo.toml` workspace `members` (insert alphabetically near `cogito-sandbox`):
```toml
members = [
    # ...existing entries...
    "crates/cogito-skills",
    # ...
]
```

Add to `[workspace.dependencies]`:
```toml
cogito-skills = { path = "crates/cogito-skills", version = "0.1.0" }
serde_yaml = "0.9"
walkdir = "2"
```
(If `serde_yaml`, `walkdir`, or `tempfile` already exist in workspace deps, do NOT re-add — reuse the existing line.)

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-skills`

Expected: build succeeds, one passing trivial test. (No "fail" here; this is the bootstrap task. The "test" is that the crate compiles cleanly inside the workspace.)

If `serde_yaml` / `walkdir` / `tempfile` are missing from `[workspace.dependencies]`, the build will fail with "no matching package found" — add them and retry.

- [ ] **Step 3: Implement (n/a for scaffold)**

The lib.rs above IS the implementation for this task. Confirm `cargo metadata -p cogito-skills` succeeds:
```bash
cargo metadata --format-version 1 -p cogito-skills | head -c 200
```

- [ ] **Step 4: Confirm full workspace build**

Run: `make fmt && make fix CRATE=cogito-skills && make test CRATE=cogito-skills`

Expected: all green; `cogito-skills` test count = 1.

Run: `make ci`

Expected: green (fmt + clippy + layer-check + test all pass).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/cogito-skills/
git commit -m "$(cat <<'EOF'
feat(skills): scaffold cogito-skills crate + workspace wiring

Hands-layer crate (no Brain dep allowed). Empty surface; later tasks
add SkillProvider impl + frontmatter parser + sigil scanner.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 1: Protocol additive surface

### Task 02: Add `SkillProvider` trait + supporting types in `cogito-protocol::skill`

**Files:**
- Create: `crates/cogito-protocol/src/skill.rs`
- Modify: `crates/cogito-protocol/src/lib.rs` (add `pub mod skill;`)
- Test: `crates/cogito-protocol/tests/skill_types.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-protocol/tests/skill_types.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillSource};
use std::path::PathBuf;

#[test]
fn skill_source_serde_roundtrip_repo() {
    let src = SkillSource::Repo {
        dir: PathBuf::from("/tmp/.cogito/skills/foo"),
    };
    let json = serde_json::to_string(&src).unwrap();
    let parsed: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, src);
    assert!(json.contains("\"kind\":\"repo\""));
}

#[test]
fn skill_source_serde_roundtrip_user() {
    let src = SkillSource::User;
    let json = serde_json::to_string(&src).unwrap();
    assert_eq!(json, r#"{"kind":"user"}"#);
}

#[test]
fn skill_source_serde_roundtrip_plugin() {
    let src = SkillSource::Plugin {
        plugin_id: "acme-tools".into(),
    };
    let json = serde_json::to_string(&src).unwrap();
    let parsed: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, src);
}

#[test]
fn skill_metadata_is_constructible() {
    let m = SkillMetadata {
        name: "invoice-parser".into(),
        description: "Parses invoices".into(),
        source: SkillSource::User,
        disable_model_invocation: false,
        user_invocable: true,
        version: Some("0.1.0".into()),
    };
    assert_eq!(m.name, "invoice-parser");
}

#[test]
fn skill_content_is_constructible() {
    let c = SkillContent {
        name: "x".into(),
        source: SkillSource::User,
        body: "# heading".into(),
    };
    assert_eq!(c.body, "# heading");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-protocol --test skill_types`

Expected: FAIL — "unresolved module skill" / "no module skill in cogito_protocol".

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-protocol/src/skill.rs`:
```rust
//! Skill loader protocol surface — trait + value types consumed by
//! `cogito-skills` (provider impl), `cogito-context` (SkillInjector),
//! and `cogito-core::harness` (H04 / H06 / H11) via dependency injection.
//!
//! Implementations live in `cogito-skills`. See ADR-0020.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Read-only handle on the registered skill set. Built by the Surface
/// (typically Runtime construction) and consumed by Brain through
/// `Arc<dyn SkillProvider>` injection.
pub trait SkillProvider: Send + Sync {
    /// Lightweight metadata for the "Available Skills" registry block.
    /// Called once per turn by the SkillInjector. MUST be cheap.
    fn list(&self) -> Vec<SkillMetadata>;

    /// Full skill body (SKILL.md text, frontmatter stripped) for
    /// activation. `None` if the name is not registered.
    fn get(&self, name: &str) -> Option<SkillContent>;

    /// O(1) check used by H06 sigil filter — only registered names
    /// activate; unknown `$X` is treated as literal text.
    fn is_registered(&self, name: &str) -> bool;
}

/// Lightweight skill descriptor (no body) — used for the system-prompt
/// registry block and for telemetry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillMetadata {
    /// Kebab-case skill identifier (bare for Repo/User/System,
    /// `<plugin_id>:<name>` for Plugin).
    pub name: String,
    /// Short one-line description (already char-capped by the parser).
    pub description: String,
    /// Where this skill was discovered.
    pub source: SkillSource,
    /// `true` if `SKILL.md` frontmatter set `disable-model-invocation: true`.
    pub disable_model_invocation: bool,
    /// `true` unless `SKILL.md` frontmatter set `user-invocable: false`.
    pub user_invocable: bool,
    /// Optional `version` field from frontmatter.
    pub version: Option<String>,
}

/// Full skill body for activation. Returned by `SkillProvider::get`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillContent {
    /// The same name as the metadata.
    pub name: String,
    /// Where this skill was discovered.
    pub source: SkillSource,
    /// SKILL.md body with frontmatter stripped (already validated UTF-8).
    pub body: String,
}

/// Where a skill was discovered. Forward-compatible with v0.2 plugins.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillSource {
    /// `<workspace>/.cogito/skills/<name>/`. `dir` is the workspace root
    /// at which the `.cogito/skills/` directory was found (NOT the skill's
    /// own directory).
    Repo {
        /// Workspace root directory.
        dir: PathBuf,
    },
    /// `~/.cogito/skills/<name>/`.
    User,
    /// `<plugin>/skills/<name>/` — never produced in v0.1 (Plugin loader
    /// is Sprint 12 / ADR-0021).
    Plugin {
        /// Plugin id; namespacing is `<plugin_id>:<skill_name>`.
        plugin_id: String,
    },
    /// cogito-bundled skill (feature-gated; off by default in v0.1).
    System,
}
```

Modify `crates/cogito-protocol/src/lib.rs` — add `pub mod skill;` next to existing `pub mod` declarations (alphabetical order).

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-protocol && cargo test -p cogito-protocol --test skill_types`

Expected: PASS — 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/skill.rs crates/cogito-protocol/src/lib.rs crates/cogito-protocol/tests/skill_types.rs
git commit -m "$(cat <<'EOF'
feat(protocol): SkillProvider trait + SkillMetadata/Content/Source

Hands-facing trait consumed by H04/H06/H11 via dependency injection.
SkillSource is non_exhaustive + JsonSchema for ADR-0007 forward-compat.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 03: Add `EventPayload::SkillActivated` + extend `TurnStarted` with `activate_skills`

**Files:**
- Modify: `crates/cogito-protocol/src/event.rs`
- Test: `crates/cogito-protocol/tests/event_roundtrip.rs` (existing — add cases)

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-protocol/tests/event_roundtrip.rs`:
```rust
#[test]
fn skill_activated_roundtrip() {
    use cogito_protocol::event::EventPayload;
    use cogito_protocol::skill::SkillSource;
    use cogito_protocol::skill::SkillActivationChannel;

    let payload = EventPayload::SkillActivated {
        skill_name: "invoice-parser".into(),
        source: SkillSource::User,
        channel: SkillActivationChannel::ModelSigil,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let parsed: EventPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, payload);
    assert!(json.contains("\"type\":\"skill_activated\""));
}

#[test]
fn turn_started_activate_skills_defaults_to_empty() {
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::event::EventPayload;

    // Old serialized form (without activate_skills) must still deserialize.
    let old_json = r#"{"type":"turn_started","data":{"user_input":[{"type":"text","text":"hi"}]}}"#;
    let parsed: EventPayload = serde_json::from_str(old_json).unwrap();
    match parsed {
        EventPayload::TurnStarted { user_input, activate_skills } => {
            assert_eq!(user_input.len(), 1);
            assert!(activate_skills.is_empty());
        }
        _ => panic!("expected TurnStarted"),
    }
}

#[test]
fn turn_started_with_activate_skills_roundtrip() {
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::event::EventPayload;

    let payload = EventPayload::TurnStarted {
        user_input: vec![ContentBlock::Text { text: "go".into() }],
        activate_skills: vec!["foo".into(), "bar".into()],
    };
    let json = serde_json::to_string(&payload).unwrap();
    let parsed: EventPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, payload);
}
```

Append to `crates/cogito-protocol/src/skill.rs`:
```rust
/// Channel that triggered a `SkillActivated` event.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillActivationChannel {
    /// Model emitted `$Name` in stream text.
    ModelSigil,
    /// User typed `/skill <name>`.
    UserSlash,
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-protocol --test event_roundtrip skill_activated_roundtrip turn_started_activate_skills_defaults_to_empty turn_started_with_activate_skills_roundtrip`

Expected: FAIL — "no variant `SkillActivated`" / "no field `activate_skills`".

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-protocol/src/event.rs`, locate `EventPayload::TurnStarted` and extend:
```rust
    TurnStarted {
        /// User input that triggered this turn.
        user_input: Vec<ContentBlock>,
        /// User-requested skill activations carried via
        /// `TurnTrigger::SkillActivation` (slash-command channel). Empty
        /// for plain `UserText` triggers. Independent from sigil-based
        /// activations, which are re-derived from previous-turn text.
        ///
        /// Additive per ADR-0007: defaults to empty on read for older
        /// fixtures.
        #[serde(default)]
        activate_skills: Vec<String>,
    },
```

Add a new variant before the closing `}` of `EventPayload`:
```rust
    /// A skill (Skill loader, ADR-0020) was activated for the upcoming
    /// turn. Written by `SkillInjector` in H11; one event per newly
    /// activated skill (dedupe rules in spec §11).
    SkillActivated {
        /// Bare name (`foo`) or `<plugin_id>:<name>` for Plugin scope.
        skill_name: String,
        /// Where the skill was discovered.
        source: crate::skill::SkillSource,
        /// Channel that triggered this activation.
        channel: crate::skill::SkillActivationChannel,
    },
```

Update any internal `EventPayload::TurnStarted { user_input }` constructor sites within `event.rs` (the trait impl block for `EventPayload::is_terminal` etc.) and in the existing tests to use the new shape — add `activate_skills: vec![]` to existing `TurnStarted { user_input: ... }` literals.

Grep workspace for other constructor sites:
```bash
git grep -n 'TurnStarted {' -- '*.rs' | grep -v 'src/event.rs'
```
For each hit, add `activate_skills: vec![]` (additive, default-empty).

- [ ] **Step 4: Run test to verify it passes**

Run:
```
make fmt && make fix CRATE=cogito-protocol
cargo test -p cogito-protocol
make test CRATE=cogito-protocol
```

Expected: all green. The whole workspace compiles because all `TurnStarted` literals now carry the new field.

Also run schema regen check:
```
cargo run -p cogito-gen-schema -- --check
```
Expected: drift detected (additive). Regenerate:
```
cargo run -p cogito-gen-schema
git diff docs/schemas/conversation-event-v1.json
```
Then re-run `--check` — green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/event.rs crates/cogito-protocol/src/skill.rs \
        crates/cogito-protocol/tests/event_roundtrip.rs \
        docs/schemas/conversation-event-v1.json
# Plus any TurnStarted constructor-site fixups across the workspace:
git add -u
git commit -m "$(cat <<'EOF'
feat(protocol): SkillActivated event + TurnStarted.activate_skills

Both additive under ADR-0007; #[serde(default)] keeps pre-Sprint-7
fixtures parseable. SCHEMA_VERSION unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 04: Add `TurnTrigger::SkillActivation` variant

**Files:**
- Modify: `crates/cogito-protocol/src/turn_trigger.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/cogito-protocol/src/turn_trigger.rs`:
```rust
    #[test]
    fn skill_activation_serde_roundtrip_no_text() {
        let trigger = TurnTrigger::SkillActivation {
            names: vec!["foo".into(), "bar".into()],
            user_text: None,
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: TurnTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, trigger);
        assert!(json.contains("\"kind\":\"skill_activation\""));
    }

    #[test]
    fn skill_activation_serde_roundtrip_with_text() {
        let trigger = TurnTrigger::SkillActivation {
            names: vec!["foo".into()],
            user_text: Some("do X".into()),
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: TurnTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, trigger);
    }
```

Replace the existing `unknown_kind_fails_to_deserialize` test's payload with a different unknown kind (the old test used `"skill_invocation"` which is now no longer suitable since `skill_activation` IS known). New payload:
```rust
    #[test]
    fn unknown_kind_fails_to_deserialize() {
        let unknown = r#"{"kind":"hook_fired","data":{"hook_id":"x"}}"#;
        let result: Result<TurnTrigger, _> = serde_json::from_str(unknown);
        assert!(result.is_err(), "expected unknown variant to error; got {result:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-protocol --lib turn_trigger`

Expected: FAIL — "no variant `SkillActivation`".

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-protocol/src/turn_trigger.rs`, update the doc comment to remove the `SkillInvocation` reservation (it is being landed now under a different shape) and add the variant:

```rust
/// What caused a new turn to start. Open-by-extension via
/// `#[non_exhaustive]` per ADR-0007 track B.
///
/// Reserved variants (DO NOT add to the enum until the matching
/// consumer lands):
///
/// - `UserContent(Vec<ContentBlock>)` — lands with v0.2 multimedia ADR.
/// - `HookFired { hook_id, payload }` — lands with post-v0.6 Hook trigger work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnTrigger {
    /// User-typed plain text. The overwhelmingly common case for v0.1.
    UserText(String),

    /// User invoked one or more skills via `/skill <name>` (optionally with
    /// trailing text). `user_text` is the leftover after slash parsing
    /// (`None` when the user typed only `/skill foo`). Both fields can be
    /// non-empty simultaneously (`/skill foo do X`).
    SkillActivation {
        /// Skill names to activate (Repo/User bare names or `<plugin_id>:<name>`).
        names: Vec<String>,
        /// Optional trailing user text that becomes `TurnStarted.user_input`.
        user_text: Option<String>,
    },
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-protocol && cargo test -p cogito-protocol --lib turn_trigger`

Expected: PASS — 3 tests including the new two and the rewritten unknown-kind.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/turn_trigger.rs
git commit -m "$(cat <<'EOF'
feat(protocol): TurnTrigger::SkillActivation variant

Carries optional user_text alongside skill names so /skill foo do X
becomes one trigger, not two. ADR-0007 additive (non_exhaustive enum).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 05: Add `StreamEvent::SkillActivationRequested` + `EventRecorder::record_skill_activated`

**Files:**
- Modify: `crates/cogito-protocol/src/stream.rs`
- Modify: `crates/cogito-protocol/src/store.rs`
- Test: `crates/cogito-protocol/tests/stream_event.rs` (existing)

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-protocol/tests/stream_event.rs`:
```rust
#[test]
fn skill_activation_requested_serde_roundtrip() {
    use cogito_protocol::stream::StreamEvent;
    let ev = StreamEvent::SkillActivationRequested {
        skill_name: "invoice-parser".into(),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains("\"kind\":\"skill_activation_requested\""));
    let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, ev);
}
```

Create `crates/cogito-protocol/tests/store_skill_activated.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::event::EventPayload;
use cogito_protocol::ids::TurnId;
use cogito_protocol::skill::{SkillActivationChannel, SkillSource};
use cogito_protocol::store::EventRecorder;
use cogito_test_fixtures::context::InMemoryRecorder;

#[tokio::test]
async fn record_skill_activated_writes_event() {
    let mut recorder = InMemoryRecorder::default();
    let turn_id = TurnId::new();
    let _eid = recorder
        .record_skill_activated(
            turn_id,
            "foo".into(),
            SkillSource::User,
            SkillActivationChannel::UserSlash,
        )
        .await
        .unwrap();
    assert_eq!(recorder.events.len(), 1);
    let (_, payload) = &recorder.events[0];
    match payload {
        EventPayload::SkillActivated { skill_name, source, channel } => {
            assert_eq!(skill_name, "foo");
            assert_eq!(*source, SkillSource::User);
            assert_eq!(*channel, SkillActivationChannel::UserSlash);
        }
        other => panic!("expected SkillActivated, got {other:?}"),
    }
}
```

Add `cogito-test-fixtures = { workspace = true }` to `crates/cogito-protocol/Cargo.toml` `[dev-dependencies]` if not already present.

- [ ] **Step 2: Run test to verify it fails**

Run:
```
cargo test -p cogito-protocol --test stream_event skill_activation_requested
cargo test -p cogito-protocol --test store_skill_activated
```

Expected: FAIL — "no variant `SkillActivationRequested`" and "no method `record_skill_activated`".

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-protocol/src/stream.rs`, inside `pub enum StreamEvent`, add the variant (alphabetically near the other `Skill*` if any; insert after `ThinkingDelta`):
```rust
    /// H06 detected a `$<registered>` sigil outside code blocks. Broadcast
    /// only — NOT persisted. Subscribers (REPL, TUI) surface this for live
    /// feedback; the authoritative activation lands as
    /// `EventPayload::SkillActivated` in the next turn's H11 pass.
    SkillActivationRequested {
        /// The bare skill name (or `<plugin_id>:<name>`) detected.
        skill_name: String,
    },
```

In `crates/cogito-protocol/src/store.rs`, inside `pub trait EventRecorder` (next to `record_system_prompt_injected`), add a default-impl method:
```rust
    /// Persist a `SkillActivated` event. Default impl routes through
    /// `append_payload`; backends needing tighter integration may override.
    async fn record_skill_activated(
        &mut self,
        turn_id: crate::ids::TurnId,
        skill_name: String,
        source: crate::skill::SkillSource,
        channel: crate::skill::SkillActivationChannel,
    ) -> Result<crate::ids::EventId, StoreError> {
        let (id, _seq) = self
            .append_payload(
                Some(turn_id),
                crate::event::EventPayload::SkillActivated {
                    skill_name,
                    source,
                    channel,
                },
            )
            .await?;
        Ok(id)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run:
```
make fmt && make fix CRATE=cogito-protocol
cargo test -p cogito-protocol
```

Expected: all green.

Also: regen schema, since `StreamEvent` is NOT in the JSONL schema (broadcast only) — verify it does NOT show up in the schema diff:
```
cargo run -p cogito-gen-schema -- --check
```
Expected: green (no drift).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/stream.rs crates/cogito-protocol/src/store.rs \
        crates/cogito-protocol/tests/stream_event.rs \
        crates/cogito-protocol/tests/store_skill_activated.rs \
        crates/cogito-protocol/Cargo.toml
git commit -m "$(cat <<'EOF'
feat(protocol): StreamEvent::SkillActivationRequested + EventRecorder.record_skill_activated

Broadcast-only sigil-detection surface (no schema bump). Default-impl
recorder method routes through append_payload like the other Sprint 6
record_* helpers.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 06: Add `SystemPromptInjectorConfig::Skill` variant + `ContextConfig.skills` integration point

**Files:**
- Modify: `crates/cogito-protocol/src/context.rs`
- Test: existing `config_tests` mod in `context.rs`

- [ ] **Step 1: Write the failing test**

Append to the `config_tests` mod in `crates/cogito-protocol/src/context.rs`:
```rust
    #[test]
    fn system_prompt_injector_config_skill_serde() {
        let cfg = SystemPromptInjectorConfig::Skill;
        let json = serde_json::to_string(&cfg).unwrap();
        assert_eq!(json, r#"{"kind":"skill"}"#);
        let parsed: SystemPromptInjectorConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SystemPromptInjectorConfig::Skill));
    }

    #[test]
    fn context_config_with_skill_injector_toml_parses() {
        let toml_input = r#"
[compactor]
kind = "none"

[history_projector]
kind = "standard"

[system_prompt_injector]
kind = "skill"

[tool_filter_overrider]
kind = "none"
"#;
        let parsed: ContextConfig = toml::from_str(toml_input).expect("parses");
        assert!(matches!(parsed.system_prompt_injector, SystemPromptInjectorConfig::Skill));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-protocol --lib context::config_tests`

Expected: FAIL — "no variant `Skill`".

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-protocol/src/context.rs`, extend `SystemPromptInjectorConfig`:
```rust
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemPromptInjectorConfig {
    /// No-op injector; `strategy.system_prompt` is used unchanged.
    None,
    /// Skill loader injector (Sprint 7): re-derives model-channel
    /// activations from history, reads user-channel names from
    /// `TurnStarted.activate_skills`, emits one `SkillActivated` event
    /// per new activation, and writes a `SystemPromptInjected` event
    /// whose suffix contains the "Available Skills" registry block plus
    /// XML-wrapped activated SKILL.md bodies.
    ///
    /// Requires a `SkillProvider` injected at Runtime build time;
    /// `cogito_context::build_pipeline` fails fast if missing.
    Skill,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-protocol && cargo test -p cogito-protocol`

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/src/context.rs
git commit -m "$(cat <<'EOF'
feat(protocol): SystemPromptInjectorConfig::Skill variant

Lets cogito.toml [context.system_prompt_injector] kind="skill" select the
new SkillInjector. Build-time error if SkillProvider not injected.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 2: `cogito-skills` core implementation

### Task 07: Frontmatter parser (`metadata.rs`)

**Files:**
- Create: `crates/cogito-skills/src/metadata.rs`
- Modify: `crates/cogito-skills/src/lib.rs` (add `pub mod metadata;`)
- Test: `crates/cogito-skills/tests/metadata.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-skills/tests/metadata.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_skills::metadata::{ParsedSkill, ParseError, parse_skill_md};

const VALID: &str = r#"---
name: invoice-parser
description: Parses invoices into structured JSON.
version: 0.1.0
---

# Invoice parser

Body content here.
"#;

#[test]
fn parses_required_fields() {
    let p = parse_skill_md(VALID).unwrap();
    assert_eq!(p.name, "invoice-parser");
    assert_eq!(p.description, "Parses invoices into structured JSON.");
    assert_eq!(p.version, Some("0.1.0".into()));
    assert!(p.body.starts_with("# Invoice parser"));
}

#[test]
fn defaults_for_optional_flags() {
    let p = parse_skill_md(VALID).unwrap();
    assert!(!p.disable_model_invocation);
    assert!(p.user_invocable);
}

#[test]
fn rejects_missing_frontmatter() {
    let err = parse_skill_md("no frontmatter here").unwrap_err();
    assert!(matches!(err, ParseError::MissingFrontmatter));
}

#[test]
fn rejects_missing_name() {
    let s = "---\ndescription: x\n---\nbody";
    assert!(matches!(parse_skill_md(s).unwrap_err(), ParseError::MissingField(_)));
}

#[test]
fn rejects_invalid_name_chars() {
    let s = "---\nname: \"foo bar!\"\ndescription: x\n---\nbody";
    let err = parse_skill_md(s).unwrap_err();
    assert!(matches!(err, ParseError::InvalidName(_)));
}

#[test]
fn description_oversize_is_capped() {
    let long = "x".repeat(2048);
    let s = format!("---\nname: foo\ndescription: \"{long}\"\n---\nbody");
    let p = parse_skill_md(&s).unwrap();
    assert!(p.description.len() <= cogito_skills::metadata::DESCRIPTION_CAP_CHARS);
    // Last char should be the ellipsis sentinel marker.
    assert!(p.description.ends_with('…'));
}

#[test]
fn parses_disable_model_invocation() {
    let s = "---\nname: foo\ndescription: x\ndisable-model-invocation: true\n---\nbody";
    let p = parse_skill_md(s).unwrap();
    assert!(p.disable_model_invocation);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-skills --test metadata`

Expected: FAIL — `parse_skill_md` not defined.

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-skills/src/metadata.rs`:
```rust
//! YAML frontmatter parser for `SKILL.md`. See ADR-0020 §3.

use serde::Deserialize;
use thiserror::Error;

/// Maximum length of the `description` field after capping (chars, not bytes).
pub const DESCRIPTION_CAP_CHARS: usize = 1024;

/// Maximum length of the `name` field (matches the sigil regex).
pub const NAME_MAX_CHARS: usize = 64;

/// Parsed `SKILL.md` representation. Body has frontmatter already stripped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSkill {
    /// Skill identifier (validated kebab-case-ish).
    pub name: String,
    /// One-line description (already char-capped).
    pub description: String,
    /// `true` if frontmatter set `disable-model-invocation: true`.
    pub disable_model_invocation: bool,
    /// `false` if frontmatter set `user-invocable: false`.
    pub user_invocable: bool,
    /// Optional `version` field.
    pub version: Option<String>,
    /// Body content (frontmatter stripped, no leading newline).
    pub body: String,
}

/// Errors returned by `parse_skill_md`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// SKILL.md did not start with a `---` frontmatter fence.
    #[error("missing frontmatter (must start with ---)")]
    MissingFrontmatter,
    /// Required field was absent.
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    /// `name` contained disallowed characters or was oversized.
    #[error("invalid name '{0}' (allowed: ^[A-Za-z][A-Za-z0-9_:-]{{0,63}}$)")]
    InvalidName(String),
    /// YAML deserialization failed.
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default, rename = "disable-model-invocation")]
    disable_model_invocation: bool,
    #[serde(rename = "user-invocable")]
    user_invocable: Option<bool>,
    version: Option<String>,
}

/// Parse a `SKILL.md` file string into a `ParsedSkill`.
pub fn parse_skill_md(input: &str) -> Result<ParsedSkill, ParseError> {
    let bytes = input.as_bytes();
    if !bytes.starts_with(b"---") {
        return Err(ParseError::MissingFrontmatter);
    }
    // Find the closing `---` on its own line.
    let after_open = &input[3..];
    let after_open = after_open.trim_start_matches('\r').trim_start_matches('\n');
    let Some(close_idx) = find_closing_fence(after_open) else {
        return Err(ParseError::MissingFrontmatter);
    };
    let yaml_text = &after_open[..close_idx];
    let body = after_open[close_idx..]
        .trim_start_matches("---")
        .trim_start_matches('\r')
        .trim_start_matches('\n')
        .to_string();

    let raw: RawFrontmatter = serde_yaml::from_str(yaml_text)?;
    let name = raw.name.ok_or(ParseError::MissingField("name"))?;
    let description = raw.description.ok_or(ParseError::MissingField("description"))?;

    if !is_valid_name(&name) {
        return Err(ParseError::InvalidName(name));
    }

    let description = cap_description(&description, DESCRIPTION_CAP_CHARS);

    Ok(ParsedSkill {
        name,
        description,
        disable_model_invocation: raw.disable_model_invocation,
        user_invocable: raw.user_invocable.unwrap_or(true),
        version: raw.version,
        body,
    })
}

fn find_closing_fence(s: &str) -> Option<usize> {
    let mut idx = 0usize;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            return Some(idx);
        }
        idx += line.len();
    }
    None
}

fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > NAME_MAX_CHARS {
        return false;
    }
    let mut iter = name.chars();
    let Some(first) = iter.next() else { return false };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    iter.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
}

fn cap_description(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    let mut out: String = s.chars().take(cap.saturating_sub(1)).collect();
    out.push('…');
    out
}
```

Modify `crates/cogito-skills/src/lib.rs` — add `pub mod metadata;` after the existing `//!` doc.

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-skills && cargo test -p cogito-skills --test metadata`

Expected: PASS — 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-skills/src/metadata.rs crates/cogito-skills/src/lib.rs \
        crates/cogito-skills/tests/metadata.rs
git commit -m "$(cat <<'EOF'
feat(skills): SKILL.md frontmatter parser

YAML frontmatter via serde_yaml; description capped at 1024 chars
with '…' ellipsis sentinel; name validated as letter-start kebab/colon.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 08: Sigil regex + code-fence-aware scanner (`sigil.rs`)

**Files:**
- Create: `crates/cogito-skills/src/sigil.rs`
- Modify: `crates/cogito-skills/src/lib.rs` (add `pub mod sigil;`)
- Test: `crates/cogito-skills/tests/sigil.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-skills/tests/sigil.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_skills::sigil::{FenceState, SigilHit, find_sigils_outside_code};

fn names(hits: Vec<SigilHit>) -> Vec<String> {
    hits.into_iter().map(|h| h.name).collect()
}

#[test]
fn finds_single_sigil() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "use $invoice-parser please");
    assert_eq!(names(hits), vec!["invoice-parser"]);
}

#[test]
fn ignores_sigil_in_fenced_code() {
    let mut s = FenceState::default();
    let text = "regular text\n```rust\nlet x = $foo;\n```\nback to text";
    let hits = find_sigils_outside_code(&mut s, text);
    assert!(names(hits).is_empty());
}

#[test]
fn ignores_sigil_in_inline_backticks() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "the `$foo` example");
    assert!(names(hits).is_empty());
}

#[test]
fn allows_sigil_with_colon_for_plugin_ns() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "$acme:linter please");
    assert_eq!(names(hits), vec!["acme:linter"]);
}

#[test]
fn streaming_fence_state_persists_across_chunks() {
    let mut s = FenceState::default();
    let _ = find_sigils_outside_code(&mut s, "intro\n```\n");
    // Now inside a fence — $foo on a separate chunk must NOT match.
    let hits = find_sigils_outside_code(&mut s, "let x = $foo;\n");
    assert!(names(hits).is_empty());
    let hits = find_sigils_outside_code(&mut s, "```\nafter fence $bar end");
    assert_eq!(names(hits), vec!["bar"]);
}

#[test]
fn rejects_digit_starting_sigil() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "amount $123");
    assert!(names(hits).is_empty(), "digits cannot start a sigil");
}

#[test]
fn caps_name_length_at_64() {
    let mut s = FenceState::default();
    let long = "a".repeat(80);
    let input = format!("$valid-{long}");  // total > 64
    let hits = find_sigils_outside_code(&mut s, &input);
    // Regex caps body to 63 chars after the leading letter; what's
    // matched is `valid-` + first 57 'a's.
    let only = hits.first().expect("expected one match");
    assert!(only.name.len() <= 64);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-skills --test sigil`

Expected: FAIL — `find_sigils_outside_code` undefined.

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-skills/src/sigil.rs`:
```rust
//! Sigil regex + streaming code-fence-aware scanner. See ADR-0020 §1
//! and spec §6.3.

use std::sync::OnceLock;

use regex::Regex;

/// Match anchored on letter; allow kebab + underscore + colon (plugin ns).
const SIGIL_PATTERN: &str = r"\$([A-Za-z][A-Za-z0-9_:-]{0,63})";

fn sigil_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(SIGIL_PATTERN).expect("sigil regex compiles"))
}

/// A sigil match in a text chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigilHit {
    /// The captured name (regex group 1).
    pub name: String,
    /// Byte offset within the supplied chunk.
    pub byte_offset: usize,
}

/// Streaming code-fence parser state. Default = `Normal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FenceState {
    /// Outside any code construct; sigils match.
    #[default]
    Normal,
    /// Inside a ```fenced block; sigils ignored until closing fence.
    InFenced,
    /// Inside an inline `backtick` span on the current line; sigils ignored
    /// until the closing backtick or end of line.
    InInline,
}

/// Find sigils in `chunk` while honoring `state`. State persists across calls
/// (so multi-chunk streaming works).
pub fn find_sigils_outside_code(state: &mut FenceState, chunk: &str) -> Vec<SigilHit> {
    let re = sigil_regex();
    let mut hits = Vec::new();
    let mut idx = 0usize;
    let bytes = chunk.as_bytes();

    while idx < bytes.len() {
        match *state {
            FenceState::Normal => {
                // Look for the next fence opener or sigil; whichever comes first.
                let triple = find_at_line_start(bytes, idx, b"```");
                let backtick = find_byte(bytes, idx, b'`');
                let newline = find_byte(bytes, idx, b'\n');

                // Pick the smallest of triple_open / backtick that is BEFORE the
                // next newline (inline scope is per-line).
                let next_special = pick_min([
                    triple,
                    if let (Some(bt), Some(nl)) = (backtick, newline) {
                        if bt < nl { Some(bt) } else { None }
                    } else { backtick },
                ]);

                let end = next_special.unwrap_or(bytes.len());
                let slice = &chunk[idx..end];
                let slice_base = idx;
                for cap in re.captures_iter(slice) {
                    let m = cap.get(0).expect("regex matched");
                    let name = cap.get(1).expect("regex group 1").as_str().to_string();
                    hits.push(SigilHit { name, byte_offset: slice_base + m.start() });
                }
                idx = end;
                if let Some(t) = triple {
                    if Some(t) == next_special {
                        *state = FenceState::InFenced;
                        idx = t + 3;
                        continue;
                    }
                }
                if let Some(bt) = backtick {
                    if Some(bt) == next_special {
                        *state = FenceState::InInline;
                        idx = bt + 1;
                        continue;
                    }
                }
            }
            FenceState::InFenced => {
                // Skip everything until the next line-start triple-backtick.
                if let Some(close) = find_at_line_start(bytes, idx, b"```") {
                    *state = FenceState::Normal;
                    idx = close + 3;
                } else {
                    return hits;
                }
            }
            FenceState::InInline => {
                // Skip until the closing backtick or end of line.
                let backtick = find_byte(bytes, idx, b'`');
                let newline = find_byte(bytes, idx, b'\n');
                match (backtick, newline) {
                    (Some(bt), Some(nl)) if bt < nl => {
                        *state = FenceState::Normal;
                        idx = bt + 1;
                    }
                    (Some(bt), None) => {
                        *state = FenceState::Normal;
                        idx = bt + 1;
                    }
                    (_, Some(nl)) => {
                        // Inline scope ends at end of line.
                        *state = FenceState::Normal;
                        idx = nl + 1;
                    }
                    (None, None) => return hits,
                }
            }
        }
    }
    hits
}

fn find_byte(bytes: &[u8], from: usize, b: u8) -> Option<usize> {
    bytes[from..].iter().position(|&x| x == b).map(|p| from + p)
}

fn find_at_line_start(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + needle.len() <= bytes.len() {
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start && bytes[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn pick_min<const N: usize>(opts: [Option<usize>; N]) -> Option<usize> {
    opts.into_iter().flatten().min()
}
```

Modify `crates/cogito-skills/src/lib.rs` — add `pub mod sigil;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-skills && cargo test -p cogito-skills --test sigil`

Expected: PASS — 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-skills/src/sigil.rs crates/cogito-skills/src/lib.rs \
        crates/cogito-skills/tests/sigil.rs
git commit -m "$(cat <<'EOF'
feat(skills): code-fence-aware sigil scanner

FenceState state machine survives across streaming chunks; regex
\$([A-Za-z][A-Za-z0-9_:-]{0,63}) anchors on letter and caps at 64 chars.
Skips both fenced ``` blocks and inline backtick spans.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 09: Filesystem discovery walker (`discovery.rs`)

**Files:**
- Create: `crates/cogito-skills/src/discovery.rs`
- Modify: `crates/cogito-skills/src/lib.rs` (add `pub mod discovery;`)
- Test: `crates/cogito-skills/tests/discovery.rs`
- Test fixtures: `crates/cogito-skills/tests/fixtures/`

- [ ] **Step 1: Write the failing test**

Create fixture files:

`crates/cogito-skills/tests/fixtures/.cogito/skills/repo-foo/SKILL.md`:
```markdown
---
name: repo-foo
description: A repo-scope skill.
---
foo body
```

`crates/cogito-skills/tests/fixtures/.cogito/skills/repo-bar/SKILL.md`:
```markdown
---
name: repo-bar
description: Another repo skill.
---
bar body
```

`crates/cogito-skills/tests/fixtures/user-home/.cogito/skills/user-baz/SKILL.md`:
```markdown
---
name: user-baz
description: A user-scope skill.
---
baz body
```

Also create an empty `crates/cogito-skills/tests/fixtures/.git/HEAD` (so the walk-up rule stops there):
```
ref: refs/heads/main
```

Create `crates/cogito-skills/tests/discovery.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use cogito_skills::discovery::{ScanConfig, discover_skills};
use cogito_protocol::skill::SkillSource;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

#[test]
fn discovers_repo_and_user_scopes() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let names: Vec<&str> = found.iter().map(|s| s.parsed.name.as_str()).collect();
    assert!(names.contains(&"repo-foo"));
    assert!(names.contains(&"repo-bar"));
    assert!(names.contains(&"user-baz"));
}

#[test]
fn repo_scope_carries_source_repo() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: None,
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let foo = found.iter().find(|s| s.parsed.name == "repo-foo").unwrap();
    matches!(foo.source, SkillSource::Repo { .. });
}

#[test]
fn user_scope_carries_source_user() {
    let cfg = ScanConfig {
        workspace_root: None,
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let baz = found.iter().find(|s| s.parsed.name == "user-baz").unwrap();
    assert!(matches!(baz.source, SkillSource::User));
}

#[test]
fn missing_user_dir_is_not_an_error() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(PathBuf::from("/does/not/exist/cogito-skills-test")),
        include_system: false,
    };
    let _ = discover_skills(&cfg).expect("missing user dir is OK");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-skills --test discovery`

Expected: FAIL — module `discovery` not found.

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-skills/src/discovery.rs`:
```rust
//! Filesystem walker — Repo scope (workspace root walk-up) + User scope
//! (`~/.cogito/skills/` by default).

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, warn};

use cogito_protocol::skill::SkillSource;

use crate::metadata::{ParseError, ParsedSkill, parse_skill_md};

/// Configuration for the discovery walker.
#[derive(Clone, Debug, Default)]
pub struct ScanConfig {
    /// Starting cwd for the Repo-scope walk-up; `None` skips Repo scope.
    pub workspace_root: Option<PathBuf>,
    /// User-scope skills directory; `None` disables user scope. Missing
    /// directories are not errors.
    pub user_dir: Option<PathBuf>,
    /// Include cogito-bundled (System) skills. v0.1 leaves this off.
    pub include_system: bool,
}

/// One discovered skill — frontmatter parsed, body retained, source known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredSkill {
    /// Parsed `SKILL.md` content.
    pub parsed: ParsedSkill,
    /// Where it was found.
    pub source: SkillSource,
    /// The skill's own directory (the parent of `SKILL.md`).
    pub dir: PathBuf,
}

/// Errors returned by `discover_skills`. Per-skill parse failures are logged
/// + skipped; only walker-level failures surface here.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// I/O failure reading the filesystem.
    #[error("io error reading {path:?}: {source}")]
    Io {
        /// Path being read when the failure occurred.
        path: PathBuf,
        /// Wrapped io error.
        source: std::io::Error,
    },
}

/// Discover skills under the configured scopes.
///
/// Repo-scope walk-up rule: start at `workspace_root`, walk parent directories
/// until either `.git/` is present, or `cogito.toml`, or filesystem root.
/// Each directory along the path is checked for `.cogito/skills/`.
pub fn discover_skills(config: &ScanConfig) -> Result<Vec<DiscoveredSkill>, DiscoveryError> {
    let mut out = Vec::new();
    if let Some(root) = &config.workspace_root {
        for dir in repo_walk_up(root) {
            scan_skills_dir(&dir.join(".cogito").join("skills"), SkillSource::Repo { dir: dir.clone() }, &mut out)?;
        }
    }
    if let Some(user_dir) = &config.user_dir {
        scan_skills_dir(user_dir, SkillSource::User, &mut out)?;
    }
    if config.include_system {
        // v0.1: no bundled skills yet.
    }
    Ok(out)
}

fn repo_walk_up(start: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = start.to_path_buf();
    loop {
        chain.push(current.clone());
        if current.join(".git").exists() || current.join("cogito.toml").exists() {
            break;
        }
        let Some(parent) = current.parent() else { break };
        if parent == current { break; }
        current = parent.to_path_buf();
    }
    chain
}

fn scan_skills_dir(
    skills_dir: &Path,
    source: SkillSource,
    out: &mut Vec<DiscoveredSkill>,
) -> Result<(), DiscoveryError> {
    if !skills_dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(skills_dir).map_err(|e| DiscoveryError::Io {
        path: skills_dir.to_path_buf(),
        source: e,
    })?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        match parse_one(&skill_md) {
            Ok(parsed) => {
                out.push(DiscoveredSkill {
                    parsed,
                    source: source.clone(),
                    dir: dir.clone(),
                });
            }
            Err(e) => {
                warn!(?skill_md, error = %e, "skipping malformed SKILL.md");
            }
        }
    }
    Ok(())
}

fn parse_one(path: &Path) -> Result<ParsedSkill, ParseError> {
    let text = fs::read_to_string(path).map_err(|e| ParseError::from(serde_yaml::Error::custom(e.to_string())))?;
    // Note: io errors masquerade as ParseError to keep the per-file
    // skip-on-error path uniform. Walker-level errors stay separate.
    let _ = debug!(?path, "parsing SKILL.md");
    parse_skill_md(&text)
}
```

> Note: the cast of `io::Error` → `ParseError` above is awkward. A cleaner
> approach is to add `ParseError::Io` and a `From<std::io::Error>` impl. Do
> that here in Step 3:
>
> Append to `metadata.rs`'s `ParseError`:
> ```rust
>     /// I/O error reading SKILL.md.
>     #[error("io error: {0}")]
>     Io(#[from] std::io::Error),
> ```
> Then `parse_one` becomes a plain `fs::read_to_string(...)? + parse_skill_md(&text)`.

Modify `crates/cogito-skills/src/lib.rs` — add `pub mod discovery;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-skills && cargo test -p cogito-skills --test discovery`

Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-skills/src/discovery.rs crates/cogito-skills/src/metadata.rs \
        crates/cogito-skills/src/lib.rs \
        crates/cogito-skills/tests/discovery.rs \
        crates/cogito-skills/tests/fixtures/
git commit -m "$(cat <<'EOF'
feat(skills): filesystem discovery walker (Repo + User scopes)

Walk-up rule: stop at .git/ OR cogito.toml OR fs root. Per-skill
parse failures are warn-logged and skipped; missing user_dir is not
an error. System scope reserved for future include_system feature.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: `SkillRegistry` impl of `SkillProvider` (`registry.rs`)

**Files:**
- Create: `crates/cogito-skills/src/registry.rs`
- Modify: `crates/cogito-skills/src/lib.rs` (re-export `SkillRegistry`, `SkillRegistryError`)
- Test: extend `crates/cogito-skills/tests/discovery.rs` with registry build cases

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-skills/tests/discovery.rs`:
```rust
use cogito_protocol::skill::SkillProvider;
use cogito_skills::{SkillRegistry, SkillRegistryError};

#[test]
fn registry_build_succeeds() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
    })
    .unwrap();
    assert!(reg.is_registered("repo-foo"));
    assert!(reg.is_registered("user-baz"));
    assert!(!reg.is_registered("nonexistent"));
}

#[test]
fn registry_get_returns_body() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: None,
        include_system: false,
    })
    .unwrap();
    let content = reg.get("repo-foo").unwrap();
    assert!(content.body.starts_with("foo body"));
}

#[test]
fn duplicate_name_in_same_dir_is_fatal() {
    use std::fs;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let skills = tmp.path().join(".cogito").join("skills");
    fs::create_dir_all(skills.join("dup-a")).unwrap();
    fs::create_dir_all(skills.join("dup-b")).unwrap();
    fs::write(skills.join("dup-a").join("SKILL.md"),
        "---\nname: dup\ndescription: a\n---\nbody-a").unwrap();
    fs::write(skills.join("dup-b").join("SKILL.md"),
        "---\nname: dup\ndescription: b\n---\nbody-b").unwrap();
    // Plant a .git/ so the walk-up stops here:
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let err = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(tmp.path().to_path_buf()),
        user_dir: None,
        include_system: false,
    })
    .unwrap_err();
    assert!(matches!(err, SkillRegistryError::DuplicateName { .. }));
}

#[test]
fn cross_scope_repo_wins_over_user() {
    use std::fs;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let repo_skills = tmp.path().join("repo").join(".cogito").join("skills").join("dual");
    let user_skills = tmp.path().join("user").join(".cogito").join("skills").join("dual");
    fs::create_dir_all(&repo_skills).unwrap();
    fs::create_dir_all(&user_skills).unwrap();
    fs::write(repo_skills.join("SKILL.md"),
        "---\nname: dual\ndescription: from-repo\n---\nrepo body").unwrap();
    fs::write(user_skills.join("SKILL.md"),
        "---\nname: dual\ndescription: from-user\n---\nuser body").unwrap();
    fs::create_dir_all(tmp.path().join("repo").join(".git")).unwrap();

    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(tmp.path().join("repo")),
        user_dir: Some(tmp.path().join("user").join(".cogito").join("skills")),
        include_system: false,
    })
    .unwrap();
    let dual = reg.get("dual").unwrap();
    assert!(dual.body.starts_with("repo body"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-skills --test discovery`

Expected: FAIL — `SkillRegistry` undefined.

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-skills/src/registry.rs`:
```rust
//! `SkillRegistry` — eager-scan implementation of `SkillProvider`.

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;
use tracing::debug;

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};

use crate::discovery::{DiscoveredSkill, DiscoveryError, ScanConfig, discover_skills};

/// Errors from `SkillRegistry::scan`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SkillRegistryError {
    /// Two skills declared the same `name` inside one directory (or the
    /// directory chain visited as a single dir scan).
    #[error("duplicate skill name '{name}' in scope {scope}")]
    DuplicateName {
        /// The colliding skill name.
        name: String,
        /// Human-readable scope label ("repo", "user", "plugin", "system").
        scope: &'static str,
    },
    /// Walker-level failure.
    #[error("discovery failed: {0}")]
    Discovery(#[from] DiscoveryError),
}

#[derive(Debug)]
struct SkillRecord {
    metadata: SkillMetadata,
    body: String,
    source: SkillSource,
}

/// Eager `SkillProvider`. Built once at Runtime construction; full bodies
/// kept in-memory (they tend to be small markdown files).
#[derive(Clone)]
pub struct SkillRegistry {
    by_name: Arc<HashMap<String, Arc<SkillRecord>>>,
}

impl SkillRegistry {
    /// Scan filesystem according to `config` and build the registry.
    ///
    /// # Errors
    ///
    /// Returns `Err(SkillRegistryError::DuplicateName)` if two skills
    /// within the same scope class declare the same `name`. Higher-scope
    /// shadowing across classes (Repo > User > Plugin > System) is silent.
    pub fn scan(config: ScanConfig) -> Result<Self, SkillRegistryError> {
        let mut found = discover_skills(&config)?;
        // Sort so Repo entries come before User; within Repo, closer (deeper)
        // dirs come first. We rely on discover_skills emitting in walk-order:
        // Repo-walked dirs in cwd→ancestor order, then User. Within one
        // scope class, detect duplicates as fatal.
        let mut by_name: HashMap<String, Arc<SkillRecord>> = HashMap::new();
        let mut seen_in_repo: HashMap<String, ()> = HashMap::new();
        let mut seen_in_user: HashMap<String, ()> = HashMap::new();
        for d in found.drain(..) {
            let scope_label: &'static str = match &d.source {
                SkillSource::Repo { .. } => "repo",
                SkillSource::User => "user",
                SkillSource::Plugin { .. } => "plugin",
                SkillSource::System => "system",
            };
            // Same-dir duplicate detection: implemented via discover_skills
            // emitting one entry per SKILL.md; same `name` from same scope
            // is fatal.
            let same_scope_seen = match &d.source {
                SkillSource::Repo { .. } => seen_in_repo.contains_key(&d.parsed.name),
                SkillSource::User => seen_in_user.contains_key(&d.parsed.name),
                _ => false,
            };
            if same_scope_seen {
                // Repo monorepo walk: closer dir wins; emit debug + skip.
                // Strict fatal only for explicit collision (which v0.1
                // hard-defines as "same scope label"). For Repo walk-up,
                // the deeper dir already populated by_name; treat the
                // later (shallower) dup as a closer-wins case.
                debug!(name = %d.parsed.name, scope = scope_label, "duplicate within scope dropped (closer dir already won)");
                continue;
            }
            if let Some(existing) = by_name.get(&d.parsed.name) {
                // Cross-scope: higher precedence already won.
                debug!(name = %d.parsed.name, existing = ?existing.source, new = ?d.source, "lower-scope skill shadowed");
                continue;
            }
            match &d.source {
                SkillSource::Repo { .. } => { seen_in_repo.insert(d.parsed.name.clone(), ()); }
                SkillSource::User => { seen_in_user.insert(d.parsed.name.clone(), ()); }
                _ => {}
            }
            let metadata = SkillMetadata {
                name: d.parsed.name.clone(),
                description: d.parsed.description.clone(),
                source: d.source.clone(),
                disable_model_invocation: d.parsed.disable_model_invocation,
                user_invocable: d.parsed.user_invocable,
                version: d.parsed.version.clone(),
            };
            by_name.insert(
                d.parsed.name.clone(),
                Arc::new(SkillRecord {
                    metadata,
                    body: d.parsed.body,
                    source: d.source,
                }),
            );
        }
        // Same-directory duplicate (file-system level): two SKILL.md files in
        // the same `skills/<dir>/` cannot occur (only one SKILL.md per dir).
        // Two skill dirs declaring the same `name` is what we want to surface
        // as fatal — that's the "same-dir" case in the spec.
        //
        // Detect by re-scanning: count name occurrences per scope label in
        // the original `found` list. (Simpler: track during the loop.)
        //
        // For now, the loop above only marks 'closer-wins' for Repo. To
        // honor the spec's "same-scope fatal" we need an explicit second
        // pass that compares parent-directories. v0.1 keeps it simple:
        // if discover_skills returns two SKILL.md files declaring the
        // same name and both are direct children of the SAME `.cogito/skills/`
        // directory, we emit DuplicateName.
        //
        // Implement that check here:
        check_same_dir_duplicates(&config)?;

        Ok(Self { by_name: Arc::new(by_name) })
    }
}

fn check_same_dir_duplicates(config: &ScanConfig) -> Result<(), SkillRegistryError> {
    use std::collections::HashMap;
    let mut found = discover_skills(config)?;
    let mut by_dir: HashMap<std::path::PathBuf, HashMap<String, ()>> = HashMap::new();
    for d in found.drain(..) {
        let parent = d.dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let names = by_dir.entry(parent).or_default();
        if names.contains_key(&d.parsed.name) {
            let scope_label = match d.source {
                SkillSource::Repo { .. } => "repo",
                SkillSource::User => "user",
                SkillSource::Plugin { .. } => "plugin",
                SkillSource::System => "system",
            };
            return Err(SkillRegistryError::DuplicateName {
                name: d.parsed.name.clone(),
                scope: scope_label,
            });
        }
        names.insert(d.parsed.name.clone(), ());
    }
    Ok(())
}

impl SkillProvider for SkillRegistry {
    fn list(&self) -> Vec<SkillMetadata> {
        self.by_name.values().map(|r| r.metadata.clone()).collect()
    }

    fn get(&self, name: &str) -> Option<SkillContent> {
        self.by_name.get(name).map(|r| SkillContent {
            name: r.metadata.name.clone(),
            source: r.source.clone(),
            body: r.body.clone(),
        })
    }

    fn is_registered(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }
}
```

Modify `crates/cogito-skills/src/lib.rs`:
```rust
pub mod discovery;
pub mod metadata;
pub mod registry;
pub mod sigil;

pub use registry::{SkillRegistry, SkillRegistryError};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-skills && cargo test -p cogito-skills`

Expected: PASS — all crate tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-skills/src/registry.rs crates/cogito-skills/src/lib.rs \
        crates/cogito-skills/tests/discovery.rs
git commit -m "$(cat <<'EOF'
feat(skills): SkillRegistry impl of SkillProvider

Eager-scan registry; same-directory duplicate names are fatal,
cross-scope shadowing (Repo > User > Plugin > System) is silent.
Body kept in-memory inside Arc<SkillRecord>.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 3: `SkillInjector` in `cogito-context`

### Task 11: `SkillInjector` skeleton + suffix builder

**Files:**
- Create: `crates/cogito-context/src/injector/skill.rs`
- Modify: `crates/cogito-context/src/injector/mod.rs` (add `pub mod skill;`)
- Test: `crates/cogito-context/tests/skill_injector.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-context/tests/skill_injector.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_context::injector::skill::SkillInjector;
use cogito_protocol::context::{InjectionInput, SystemPromptInjector};
use cogito_protocol::event::EventPayload;
use cogito_protocol::exec_ctx::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::strategy::HarnessStrategy;
use cogito_test_fixtures::context::InMemoryRecorder;

struct StaticProvider {
    skills: Vec<(SkillMetadata, String)>,
}

impl SkillProvider for StaticProvider {
    fn list(&self) -> Vec<SkillMetadata> {
        self.skills.iter().map(|(m, _)| m.clone()).collect()
    }
    fn get(&self, name: &str) -> Option<SkillContent> {
        self.skills.iter().find_map(|(m, body)| {
            if m.name == name {
                Some(SkillContent {
                    name: m.name.clone(),
                    source: m.source.clone(),
                    body: body.clone(),
                })
            } else { None }
        })
    }
    fn is_registered(&self, name: &str) -> bool {
        self.skills.iter().any(|(m, _)| m.name == name)
    }
}

fn provider() -> Arc<dyn SkillProvider> {
    Arc::new(StaticProvider {
        skills: vec![(
            SkillMetadata {
                name: "invoice-parser".into(),
                description: "Parses invoices.".into(),
                source: SkillSource::User,
                disable_model_invocation: false,
                user_invocable: true,
                version: None,
            },
            "# Invoice parser body".into(),
        )],
    })
}

#[tokio::test]
async fn empty_history_emits_registry_block_only() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
    let input = InjectionInput {
        session_id, turn_id,
        strategy: &strategy, history: &[],
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let injector = SkillInjector::new(provider());
    let _ = injector.inject(input).await.unwrap();
    let (_, payload) = recorder.events.last().unwrap();
    match payload {
        EventPayload::SystemPromptInjected { suffix, contributors, produced_by, .. } => {
            assert!(suffix.contains("Available Skills"));
            assert!(suffix.contains("invoice-parser"));
            assert!(contributors.is_empty(), "no activations on empty history");
            assert_eq!(produced_by, "skill");
        }
        _ => panic!("expected SystemPromptInjected"),
    }
}

#[tokio::test]
async fn empty_registry_emits_empty_suffix() {
    struct Empty;
    impl SkillProvider for Empty {
        fn list(&self) -> Vec<SkillMetadata> { vec![] }
        fn get(&self, _: &str) -> Option<SkillContent> { None }
        fn is_registered(&self, _: &str) -> bool { false }
    }
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);
    let input = InjectionInput {
        session_id, turn_id,
        strategy: &strategy, history: &[],
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let injector = SkillInjector::new(Arc::new(Empty));
    let _ = injector.inject(input).await.unwrap();
    let (_, payload) = recorder.events.last().unwrap();
    match payload {
        EventPayload::SystemPromptInjected { suffix, .. } => {
            assert!(suffix.is_empty());
        }
        _ => panic!("expected SystemPromptInjected"),
    }
}
```

Add `cogito-skills = { workspace = true }` to `crates/cogito-context/Cargo.toml` `[dev-dependencies]` — only for tests that build a real provider; not in `[dependencies]`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-context --test skill_injector`

Expected: FAIL — `SkillInjector` not defined.

- [ ] **Step 3: Write minimal implementation**

Create `crates/cogito-context/src/injector/skill.rs`:
```rust
//! `SkillInjector` — `SystemPromptInjector` impl for the Skill loader.
//!
//! Spec: `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md` §7.

use std::sync::Arc;

use async_trait::async_trait;

use cogito_protocol::context::{ContextError, InjectionInput, SystemPromptInjector};
use cogito_protocol::ids::EventId;
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::store::EventRecorder;

/// Per-skill description character cap for the registry block.
const DESCRIPTION_CAP_CHARS: usize = 1024;

/// `SystemPromptInjector` impl powered by a `SkillProvider`.
#[derive(Clone)]
pub struct SkillInjector {
    provider: Arc<dyn SkillProvider>,
    description_cap_chars: usize,
}

impl SkillInjector {
    /// Construct with default char cap.
    #[must_use]
    pub fn new(provider: Arc<dyn SkillProvider>) -> Self {
        Self {
            provider,
            description_cap_chars: DESCRIPTION_CAP_CHARS,
        }
    }
}

#[async_trait]
impl SystemPromptInjector for SkillInjector {
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError> {
        // Task 12 will fill in candidate collection + dedupe.
        // For Task 11 we emit only the registry block — no activations.
        let suffix = build_registry_block(&*self.provider, self.description_cap_chars);
        let event_id = EventRecorder::record_system_prompt_injected(
            input.recorder,
            input.turn_id,
            suffix,
            Vec::new(),
            "skill",
        )
        .await?;
        Ok(event_id)
    }

    fn id(&self) -> &'static str {
        "skill"
    }
}

fn build_registry_block(provider: &dyn SkillProvider, cap_chars: usize) -> String {
    let metas = provider.list();
    if metas.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Available Skills\n");
    for m in metas {
        let desc = if m.description.chars().count() > cap_chars {
            let mut t: String = m.description.chars().take(cap_chars.saturating_sub(1)).collect();
            t.push('…');
            t
        } else {
            m.description
        };
        out.push_str("- ");
        out.push_str(&m.name);
        out.push_str(": ");
        out.push_str(&desc);
        out.push('\n');
    }
    out
}
```

Modify `crates/cogito-context/src/injector/mod.rs`:
```rust
pub mod none;
pub mod skill;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-context && cargo test -p cogito-context --test skill_injector`

Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/injector/skill.rs \
        crates/cogito-context/src/injector/mod.rs \
        crates/cogito-context/tests/skill_injector.rs \
        crates/cogito-context/Cargo.toml
git commit -m "$(cat <<'EOF'
feat(context): SkillInjector skeleton + Available Skills registry block

Emits SystemPromptInjected every turn (matches Sprint 6 trait contract).
Activation logic lands in Task 12; this task ships only the always-on
registry block.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: `SkillInjector` activation logic — re-derive + dedupe + SkillActivated events

**Files:**
- Modify: `crates/cogito-context/src/injector/skill.rs`
- Test: extend `crates/cogito-context/tests/skill_injector.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-context/tests/skill_injector.rs`:
```rust
use chrono::Utc;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::ConversationEvent;
use cogito_protocol::ids::EventId;
use cogito_protocol::skill::{SkillActivationChannel, SkillSource};

fn make_event(seq: u64, turn_id: TurnId, payload: EventPayload) -> ConversationEvent {
    ConversationEvent {
        schema_version: cogito_protocol::event::SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id: SessionId::new(),
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload,
    }
}

#[tokio::test]
async fn user_channel_activates_from_turn_started() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);

    let history = vec![make_event(0, turn_id, EventPayload::TurnStarted {
        user_input: vec![],
        activate_skills: vec!["invoice-parser".into()],
    })];

    let input = InjectionInput {
        session_id, turn_id,
        strategy: &strategy, history: &history,
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let mut saw_activated = false;
    let mut saw_injected = false;
    for (_, p) in &recorder.events {
        match p {
            EventPayload::SkillActivated { skill_name, channel, .. } => {
                assert_eq!(skill_name, "invoice-parser");
                assert_eq!(*channel, SkillActivationChannel::UserSlash);
                saw_activated = true;
            }
            EventPayload::SystemPromptInjected { suffix, contributors, .. } => {
                assert!(suffix.contains("<skill name=\"invoice-parser\""));
                assert!(suffix.contains("# Invoice parser body"));
                assert_eq!(contributors, &vec!["invoice-parser".to_string()]);
                saw_injected = true;
            }
            _ => {}
        }
    }
    assert!(saw_activated && saw_injected);
}

#[tokio::test]
async fn model_channel_activates_from_previous_text_block() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let prev_turn = TurnId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![
        make_event(0, prev_turn, EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: "hi".into() }],
            activate_skills: vec![],
        }),
        make_event(1, prev_turn, EventPayload::AssistantMessageAppended {
            text: "Sure, $invoice-parser please.".into(),
        }),
        make_event(2, cur_turn, EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: "go".into() }],
            activate_skills: vec![],
        }),
    ];

    let input = InjectionInput {
        session_id, turn_id: cur_turn,
        strategy: &strategy, history: &history,
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let activated: Vec<_> = recorder.events.iter().filter_map(|(_, p)| {
        if let EventPayload::SkillActivated { skill_name, channel, .. } = p {
            Some((skill_name.clone(), *channel))
        } else { None }
    }).collect();
    assert_eq!(activated, vec![("invoice-parser".to_string(), SkillActivationChannel::ModelSigil)]);
}

#[tokio::test]
async fn prior_activation_dedupes_repeat() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let prev_turn = TurnId::new();
    let cur_turn = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, cur_turn);

    let history = vec![
        make_event(0, prev_turn, EventPayload::SkillActivated {
            skill_name: "invoice-parser".into(),
            source: SkillSource::User,
            channel: SkillActivationChannel::ModelSigil,
        }),
        make_event(1, cur_turn, EventPayload::TurnStarted {
            user_input: vec![],
            activate_skills: vec!["invoice-parser".into()],
        }),
    ];

    let input = InjectionInput {
        session_id, turn_id: cur_turn,
        strategy: &strategy, history: &history,
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let _ = SkillInjector::new(provider()).inject(input).await.unwrap();

    let count = recorder.events.iter().filter(|(_, p)|
        matches!(p, EventPayload::SkillActivated { .. })
    ).count();
    assert_eq!(count, 0, "must not re-activate already-activated skill");
}

#[tokio::test]
async fn idempotent_on_existing_system_prompt_injected() {
    let mut recorder = InMemoryRecorder::default();
    let strategy = HarnessStrategy::default_with_model("test");
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let exec_ctx = ExecCtx::open_ended(session_id, turn_id);

    let existing_id = EventId::new();
    let history = vec![ConversationEvent {
        schema_version: cogito_protocol::event::SCHEMA_VERSION,
        event_id: existing_id,
        session_id,
        turn_id: Some(turn_id),
        seq: 0,
        ts: Utc::now(),
        payload: EventPayload::SystemPromptInjected {
            turn_id, suffix: "preexisting".into(),
            contributors: vec![], produced_by: "skill".into(),
        },
    }];

    let input = InjectionInput {
        session_id, turn_id,
        strategy: &strategy, history: &history,
        exec_ctx: &exec_ctx, recorder: &mut recorder,
    };
    let returned = SkillInjector::new(provider()).inject(input).await.unwrap();
    assert_eq!(returned, existing_id);
    assert!(recorder.events.is_empty(), "no new events on resume hit");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-context --test skill_injector`

Expected: FAIL — 4 new tests expect activation behavior that isn't implemented yet.

- [ ] **Step 3: Write minimal implementation**

Replace `inject` in `crates/cogito-context/src/injector/skill.rs`:
```rust
#[async_trait]
impl SystemPromptInjector for SkillInjector {
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError> {
        // Idempotency: if a SystemPromptInjected for this turn exists, return early.
        if let Some(eid) = find_existing_injection(input.history, input.turn_id) {
            return Ok(eid);
        }

        // Step 1: collect user-channel names from current turn's TurnStarted.
        let user_names = collect_user_channel(input.history, input.turn_id);

        // Step 2: collect model-channel names from previous turn(s) text.
        let model_names = collect_model_channel(input.history, input.turn_id, &*self.provider);

        // Step 3: dedupe against prior SkillActivated events.
        let prior = collect_prior_activations(input.history);

        let mut seen_this_turn: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut contributors: Vec<String> = Vec::new();
        let mut to_inject: Vec<String> = Vec::new();

        for name in user_names.iter().cloned()
            .map(|n| (n, cogito_protocol::skill::SkillActivationChannel::UserSlash))
            .chain(model_names.into_iter()
                .map(|n| (n, cogito_protocol::skill::SkillActivationChannel::ModelSigil)))
        {
            let (name, channel) = name;
            if prior.contains(&name) { continue; }
            if !seen_this_turn.insert(name.clone()) { continue; }
            let Some(content) = self.provider.get(&name) else { continue };
            EventRecorder::record_skill_activated(
                input.recorder,
                input.turn_id,
                name.clone(),
                content.source.clone(),
                channel,
            ).await?;
            contributors.push(name.clone());
            to_inject.push(name);
        }

        // Step 4: build suffix.
        let registry = build_registry_block(&*self.provider, self.description_cap_chars);
        let bodies = build_body_blocks(&*self.provider, &to_inject);
        let suffix = if registry.is_empty() && bodies.is_empty() {
            String::new()
        } else {
            format!("{registry}{bodies}")
        };

        let event_id = EventRecorder::record_system_prompt_injected(
            input.recorder, input.turn_id, suffix, contributors, "skill",
        ).await?;
        Ok(event_id)
    }
    fn id(&self) -> &'static str { "skill" }
}

fn find_existing_injection(
    history: &[cogito_protocol::event::ConversationEvent],
    turn_id: cogito_protocol::ids::TurnId,
) -> Option<EventId> {
    for ev in history {
        if ev.turn_id == Some(turn_id) {
            if let cogito_protocol::event::EventPayload::SystemPromptInjected { .. } = &ev.payload {
                return Some(ev.event_id);
            }
        }
    }
    None
}

fn collect_user_channel(
    history: &[cogito_protocol::event::ConversationEvent],
    turn_id: cogito_protocol::ids::TurnId,
) -> Vec<String> {
    for ev in history {
        if ev.turn_id == Some(turn_id) {
            if let cogito_protocol::event::EventPayload::TurnStarted { activate_skills, .. } = &ev.payload {
                return activate_skills.clone();
            }
        }
    }
    Vec::new()
}

fn collect_model_channel(
    history: &[cogito_protocol::event::ConversationEvent],
    current_turn: cogito_protocol::ids::TurnId,
    provider: &dyn SkillProvider,
) -> Vec<String> {
    use cogito_skills_sigil::{FenceState, find_sigils_outside_code};
    // The sigil function lives in cogito-skills. To avoid a circular dep
    // (cogito-context → cogito-skills), we re-implement the regex inline
    // here OR add cogito-skills as a *direct* dep of cogito-context.
    // Per ADR-0004 cogito-context is Hands; cogito-skills is also Hands.
    // Same-layer dep is allowed. Add cogito-skills to cogito-context's
    // [dependencies] (NOT just dev-deps).
    //
    // We use the public cogito_skills::sigil::find_sigils_outside_code.
    use cogito_skills::sigil::{FenceState as FS, find_sigils_outside_code as fnd};
    let mut state = FS::default();
    let mut names: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut hit_current = false;
    for ev in history {
        if ev.turn_id == Some(current_turn) { hit_current = true; }
        if hit_current { continue; }
        if let cogito_protocol::event::EventPayload::AssistantMessageAppended { text } = &ev.payload {
            for hit in fnd(&mut state, text) {
                if provider.is_registered(&hit.name) && seen.insert(hit.name.clone()) {
                    names.push(hit.name);
                }
            }
        }
    }
    names
}

fn collect_prior_activations(
    history: &[cogito_protocol::event::ConversationEvent],
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for ev in history {
        if let cogito_protocol::event::EventPayload::SkillActivated { skill_name, .. } = &ev.payload {
            out.insert(skill_name.clone());
        }
    }
    out
}

fn build_body_blocks(provider: &dyn SkillProvider, names: &[String]) -> String {
    if names.is_empty() { return String::new(); }
    let mut out = String::from("\n");
    for name in names {
        let Some(content) = provider.get(name) else { continue };
        let source_kind = match content.source {
            cogito_protocol::skill::SkillSource::Repo { .. } => "repo",
            cogito_protocol::skill::SkillSource::User => "user",
            cogito_protocol::skill::SkillSource::Plugin { .. } => "plugin",
            cogito_protocol::skill::SkillSource::System => "system",
        };
        out.push_str(&format!("\n<skill name=\"{name}\" source=\"{source_kind}\">\n"));
        out.push_str(&content.body);
        out.push_str("\n</skill>\n");
    }
    out
}
```

Update `crates/cogito-context/Cargo.toml` `[dependencies]` to add:
```toml
cogito-skills = { workspace = true }
```
(Same-layer Hands dep — allowed.)

> Note: the `cogito_skills_sigil` import in the snippet above is a comment-only
> placeholder; the actual `use cogito_skills::sigil::...` is the real path.
> Remove the placeholder lines before saving.

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-context && cargo test -p cogito-context --test skill_injector`

Expected: PASS — 6 tests (2 from Task 11 + 4 from Task 12).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/injector/skill.rs crates/cogito-context/Cargo.toml \
        crates/cogito-context/tests/skill_injector.rs
git commit -m "$(cat <<'EOF'
feat(context): SkillInjector activation logic — user + model channels

User-channel reads TurnStarted.activate_skills of current turn;
model-channel re-derives by scanning AssistantMessageAppended of prior
turns through cogito_skills::sigil. Dedupes against prior
SkillActivated events; idempotent on existing SystemPromptInjected.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: `cogito_context::build_pipeline` factory takes `Option<Arc<dyn SkillProvider>>`

**Files:**
- Modify: `crates/cogito-context/src/pipeline.rs`
- Modify: `crates/cogito-context/src/lib.rs` (export updated factory)
- Modify: callers of `build_pipeline` (currently only `crates/cogito-core/src/runtime/builder.rs:163`)
- Test: `crates/cogito-context/tests/pipeline_assembly.rs` (existing — extend)

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-context/tests/pipeline_assembly.rs`:
```rust
use std::sync::Arc;
use cogito_protocol::context::{ContextConfig, SystemPromptInjectorConfig};
use cogito_protocol::skill::SkillProvider;

struct EmptyProvider;
impl SkillProvider for EmptyProvider {
    fn list(&self) -> Vec<cogito_protocol::skill::SkillMetadata> { vec![] }
    fn get(&self, _: &str) -> Option<cogito_protocol::skill::SkillContent> { None }
    fn is_registered(&self, _: &str) -> bool { false }
}

#[test]
fn skill_injector_requires_provider() {
    let mut cfg = ContextConfig::default();
    cfg.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    let err = cogito_context::build_pipeline_v2(&cfg, None).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("skill"));
}

#[test]
fn skill_injector_builds_with_provider() {
    let mut cfg = ContextConfig::default();
    cfg.system_prompt_injector = SystemPromptInjectorConfig::Skill;
    let provider: Arc<dyn SkillProvider> = Arc::new(EmptyProvider);
    let pipeline = cogito_context::build_pipeline_v2(&cfg, Some(provider)).unwrap();
    assert_eq!(pipeline.injector.id(), "skill");
}

#[test]
fn none_injector_works_without_provider() {
    let cfg = ContextConfig::default();
    let pipeline = cogito_context::build_pipeline_v2(&cfg, None).unwrap();
    assert_eq!(pipeline.injector.id(), "none");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-context --test pipeline_assembly`

Expected: FAIL — `build_pipeline_v2` undefined.

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-context/src/pipeline.rs`, add (alongside the existing `build_pipeline`):
```rust
use std::sync::Arc;
use thiserror::Error;
use cogito_protocol::skill::SkillProvider;

/// Errors from `build_pipeline_v2`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PipelineBuildError {
    /// `SystemPromptInjectorConfig::Skill` was selected but no `SkillProvider`
    /// was supplied.
    #[error("system_prompt_injector kind = 'skill' requires a SkillProvider to be injected at Runtime build time")]
    MissingSkillProvider,
}

/// Build a `ContextPipeline` from config + optional SkillProvider.
///
/// `build_pipeline` (no `_v2` suffix) is preserved for backward compatibility
/// and forwards to this with `skill_provider = None`. Internal callers
/// migrating to Sprint 7 should switch to this entry point.
pub fn build_pipeline_v2(
    config: &cogito_protocol::context::ContextConfig,
    skill_provider: Option<Arc<dyn SkillProvider>>,
) -> Result<cogito_protocol::context::ContextPipeline, PipelineBuildError> {
    use cogito_protocol::context::{
        ContextPipeline, HistoryProjectorConfig, SystemPromptInjectorConfig,
    };
    let compactor = build_compactor(&config.compactor);
    let projector = build_projector(&config.history_projector);
    let overrider = build_overrider(&config.tool_filter_overrider);
    let injector: Arc<dyn cogito_protocol::context::SystemPromptInjector> = match &config.system_prompt_injector {
        SystemPromptInjectorConfig::None => Arc::new(crate::injector::none::NoneInjector),
        SystemPromptInjectorConfig::Skill => {
            let p = skill_provider.ok_or(PipelineBuildError::MissingSkillProvider)?;
            Arc::new(crate::injector::skill::SkillInjector::new(p))
        }
    };
    Ok(ContextPipeline { compactor, projector, injector, overrider })
}

// `build_pipeline` (Sprint 6) stays unchanged signature-wise; internally it
// can now route through build_pipeline_v2 with None:
pub fn build_pipeline(
    config: &cogito_protocol::context::ContextConfig,
) -> cogito_protocol::context::ContextPipeline {
    build_pipeline_v2(config, None).expect("legacy build_pipeline forces injector=None")
}
```

Re-export from `crates/cogito-context/src/lib.rs`:
```rust
pub use pipeline::{PipelineBuildError, build_pipeline, build_pipeline_v2};
```

(The existing `pub use pipeline::build_pipeline;` is preserved; only add the new symbols.)

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-context && cargo test -p cogito-context`

Expected: PASS — all crate tests including the 3 new pipeline_assembly tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-context/src/pipeline.rs crates/cogito-context/src/lib.rs \
        crates/cogito-context/tests/pipeline_assembly.rs
git commit -m "$(cat <<'EOF'
feat(context): build_pipeline_v2 takes Option<Arc<dyn SkillProvider>>

Selecting SystemPromptInjectorConfig::Skill requires a provider; build
fails fast otherwise. Legacy build_pipeline preserved for callers that
don't yet use Skill (forwards with None).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 4: H06 sigil detection (Brain)

### Task 14: Add `Option<Arc<dyn SkillProvider>>` to `TurnDeps`; thread to H06

**Files:**
- Modify: `crates/cogito-core/src/harness/turn_driver/deps.rs`
- Modify: `crates/cogito-core/src/runtime/session_loop.rs` (`spawn_turn_driver` constructs TurnDeps)
- Modify: `crates/cogito-core/src/runtime/builder.rs` (Runtime carries `Option<Arc<dyn SkillProvider>>`)

- [ ] **Step 1: Write the failing test**

(Wiring task — verified by compilation; no new test file. The next task supplies the H06 unit test.)

- [ ] **Step 2: Run baseline build**

Run: `cargo build -p cogito-core`

Expected: green at baseline.

- [ ] **Step 3: Implement**

In `crates/cogito-core/src/harness/turn_driver/deps.rs`, extend `TurnDeps`:
```rust
pub struct TurnDeps {
    // ... existing fields ...
    /// Optional Skill loader provider. `None` for sessions whose strategy
    /// does NOT select `SystemPromptInjectorConfig::Skill`. H06 uses it to
    /// gate sigil detection; H11's SkillInjector holds its own Arc internally.
    pub skills: Option<Arc<dyn cogito_protocol::skill::SkillProvider>>,
}
```

In `crates/cogito-core/src/runtime/builder.rs`:

- Add `skills` field to `Runtime`:
```rust
pub struct Runtime {
    // ... existing fields ...
    skills: Option<Arc<dyn cogito_protocol::skill::SkillProvider>>,
}
```

- Add to `RuntimeBuilder`:
```rust
pub struct RuntimeBuilder {
    // ... existing fields ...
    skills: Option<Arc<dyn cogito_protocol::skill::SkillProvider>>,
}

impl RuntimeBuilder {
    /// Inject a `SkillProvider`. Optional — required only when the strategy
    /// selects `SystemPromptInjectorConfig::Skill`.
    #[must_use]
    pub fn skills(mut self, skills: Arc<dyn cogito_protocol::skill::SkillProvider>) -> Self {
        self.skills = Some(skills);
        self
    }
}
```

- In `RuntimeBuilder::build`, propagate `skills` to `Runtime`:
```rust
Ok(Arc::new(Runtime {
    // ... existing fields ...
    skills: self.skills,
}))
```

- In `Runtime::open_session`, replace the line `let context_pipeline = Arc::new(cogito_context::build_pipeline(&self.strategy.context));` with:
```rust
let context_pipeline = Arc::new(
    cogito_context::build_pipeline_v2(&self.strategy.context, self.skills.clone())
        .map_err(|e| RuntimeError::ResumeFailed {
            id,
            reason: e.to_string(),
        })?,
);
```

In `crates/cogito-core/src/runtime/session_loop.rs`, find `spawn_turn_driver` (or wherever `TurnDeps` is constructed) and add `skills` field passing. The exact call site needs the same `Option<Arc<dyn SkillProvider>>` plumbed through. Since session_loop currently doesn't hold a reference to `Runtime.skills`, propagate via `SessionState` or `SessionDeps`:

- Add `skills: Option<Arc<dyn cogito_protocol::skill::SkillProvider>>` to `SessionDeps` in `runtime/session_loop.rs` (or `runtime/types.rs` if `SessionDeps` lives there).
- Populate at the call site in `Runtime::open_session` where `SessionDeps` is built.
- In `spawn_turn_driver`, pass `deps.skills.clone()` into the new `TurnDeps.skills`.

Run the workspace build to surface every missing field:
```bash
cargo build --all-targets 2>&1 | head -40
```
Walk through errors top-down; each missing `skills: ...` field needs adding.

- [ ] **Step 4: Verify**

Run: `make fmt && make fix CRATE=cogito-core && make test CRATE=cogito-core`

Expected: green; all existing tests still pass with new `skills` field added.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/turn_driver/deps.rs \
        crates/cogito-core/src/runtime/builder.rs \
        crates/cogito-core/src/runtime/session_loop.rs \
        crates/cogito-core/src/runtime/types.rs
git commit -m "$(cat <<'EOF'
feat(core): plumb SkillProvider through Runtime → TurnDeps

Optional Arc<dyn SkillProvider> threaded from RuntimeBuilder through
SessionDeps into TurnDeps. Required only when strategy selects
SystemPromptInjectorConfig::Skill.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 15: H06 — code-fence-aware sigil detection with broadcast emit

**Files:**
- Modify: `crates/cogito-core/src/harness/stream_demux.rs`
- Test: `crates/cogito-core/tests/h06_skill_sigil_detection.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-core/tests/h06_skill_sigil_detection.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! H06 sigil-detection side-channel: validates that the demuxer emits
//! `StreamEvent::SkillActivationRequested` only for registered names and
//! only outside code fences.

use std::sync::Arc;

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::stream::StreamEvent;
use tokio::sync::broadcast;

use cogito_core::harness::stream_demux::sigil_emit_for_test;

struct OnlyFoo;
impl SkillProvider for OnlyFoo {
    fn list(&self) -> Vec<SkillMetadata> { vec![] }
    fn get(&self, _: &str) -> Option<SkillContent> { None }
    fn is_registered(&self, name: &str) -> bool { name == "foo" }
}

#[tokio::test]
async fn emits_for_registered_name_outside_fence() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&Arc::new(OnlyFoo) as &Arc<dyn SkillProvider>, &mut state, "use $foo please", &tx).unwrap();
    let ev = rx.try_recv().unwrap();
    assert!(matches!(ev, StreamEvent::SkillActivationRequested { skill_name } if skill_name == "foo"));
}

#[tokio::test]
async fn does_not_emit_for_unregistered_name() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&Arc::new(OnlyFoo) as &Arc<dyn SkillProvider>, &mut state, "use $bar please", &tx).unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn does_not_emit_inside_fenced_code() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&Arc::new(OnlyFoo) as &Arc<dyn SkillProvider>, &mut state, "```\n$foo\n```\n", &tx).unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn deduplicates_same_name_in_one_chunk() {
    let (tx, mut rx) = broadcast::channel(8);
    let mut state = cogito_skills::sigil::FenceState::default();
    sigil_emit_for_test(&Arc::new(OnlyFoo) as &Arc<dyn SkillProvider>, &mut state, "$foo and $foo again", &tx).unwrap();
    assert!(matches!(rx.try_recv().unwrap(),
        StreamEvent::SkillActivationRequested { skill_name } if skill_name == "foo"));
    assert!(rx.try_recv().is_err(), "second occurrence in same chunk must not re-emit");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-core --test h06_skill_sigil_detection`

Expected: FAIL — `sigil_emit_for_test` undefined.

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-core/src/harness/stream_demux.rs`:

Add at top:
```rust
use std::sync::Arc;

use cogito_skills::sigil::{FenceState, find_sigils_outside_code};
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::stream::StreamEvent;
use tokio::sync::broadcast;
```

Add a free function used by H06 (and exposed for tests):
```rust
/// H06 helper: detect sigils in a text-delta chunk and broadcast
/// `StreamEvent::SkillActivationRequested` for each registered hit.
/// Dedupes within the chunk.
///
/// Pure outside the broadcast `send`; broadcast lag errors are intentionally
/// ignored (broadcast lagged subscribers drop messages by design).
pub fn sigil_emit_for_test(
    provider: &Arc<dyn SkillProvider>,
    state: &mut FenceState,
    chunk: &str,
    broadcast_tx: &broadcast::Sender<StreamEvent>,
) -> Result<(), broadcast::error::SendError<StreamEvent>> {
    let mut seen_this_chunk = std::collections::HashSet::new();
    for hit in find_sigils_outside_code(state, chunk) {
        if !provider.is_registered(&hit.name) { continue; }
        if !seen_this_chunk.insert(hit.name.clone()) { continue; }
        let _ = broadcast_tx.send(StreamEvent::SkillActivationRequested {
            skill_name: hit.name,
        });
    }
    Ok(())
}
```

> Note: the function is named `sigil_emit_for_test` for the unit test surface.
> The real H06 code path calls the same underlying logic from inside the
> demuxer's text-delta handler. Wire it in:
>
> Locate the text-delta handling code in `stream_demux.rs` (a `match` arm or
> dedicated function consuming `ModelEvent::TextDelta { chunk, .. }`).
> Carry a `Option<Arc<dyn SkillProvider>>` + per-block `FenceState` through
> the demuxer's state struct. On each delta, call:
>
> ```rust
> if let Some(provider) = &self.skills {
>     let _ = sigil_emit_for_test(provider, &mut self.fence_state, chunk, &self.broadcast_tx);
> }
> ```
>
> Reset `fence_state` on `ModelEvent::TextBlockCompleted` boundaries.

Add `cogito-skills = { workspace = true }` to `crates/cogito-core/Cargo.toml` `[dependencies]`. Per ADR-0004, Brain (`cogito-core::harness`) may only import `cogito-protocol`. The H06 sigil routine needs the `cogito_skills::sigil` regex/state helpers — these are **pure** functions on `&str`, no I/O. The clean route is to **promote `sigil.rs` into `cogito-protocol`** so Brain can use it. Choose ONE of the following before committing:

**Option A (recommended)**: Move `crates/cogito-skills/src/sigil.rs` → `crates/cogito-protocol/src/sigil.rs` and re-export it from `cogito-skills::sigil` for backward-compat. Brain imports `cogito_protocol::sigil`.

**Option B**: Add `cogito-skills` as a permitted Brain dep — requires updating `scripts/check-layer.sh` and ADR-0004's "Brain only sees Hands/Session/Boundary through Protocol traits" rule. Heavier change.

Pick Option A; redo the import paths:
```rust
use cogito_protocol::sigil::{FenceState, find_sigils_outside_code};
```
Update `cogito-skills/src/sigil.rs` to be a thin `pub use cogito_protocol::sigil::*;`. Rerun `cargo test -p cogito-skills` to confirm green.

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-core && cargo test -p cogito-core --test h06_skill_sigil_detection`

Expected: PASS — 4 tests.

Run `make ci` to ensure layer-check still passes (no Brain → Hands leak).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness/stream_demux.rs crates/cogito-core/Cargo.toml \
        crates/cogito-skills/src/sigil.rs crates/cogito-protocol/src/sigil.rs \
        crates/cogito-protocol/src/lib.rs \
        crates/cogito-core/tests/h06_skill_sigil_detection.rs
git commit -m "$(cat <<'EOF'
feat(core): H06 code-fence-aware sigil detection

Pure sigil helpers moved to cogito-protocol::sigil so Brain layer can
consume them without crossing ADR-0004's Brain→Hands boundary.
Broadcasts StreamEvent::SkillActivationRequested for each registered
hit; deduped per chunk.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 5: Runtime wiring + `try_start_turn` projection

### Task 16: `try_start_turn` projection for `TurnTrigger::SkillActivation`

**Files:**
- Modify: `crates/cogito-core/src/runtime/session_loop.rs` around line 420 (`try_start_turn`)
- Test: `crates/cogito-core/tests/turn_driver_skill_activation_user.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-core/tests/turn_driver_skill_activation_user.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Validates that TurnTrigger::SkillActivation projects to TurnStarted with
//! the right activate_skills + user_input shape. Drives one turn through a
//! mock model that just echoes — assertion is on the recorded TurnStarted
//! event.

use std::sync::Arc;

use cogito_protocol::event::EventPayload;
use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
use cogito_protocol::turn_trigger::TurnTrigger;

#[tokio::test]
async fn skill_activation_with_text_projects_correctly() {
    // Construct a minimal Runtime + open a fresh session, push a
    // TurnTrigger::SkillActivation { names: ["foo"], user_text: Some("hi") },
    // wait for TurnStarted to appear in the recorded events, assert
    // user_input = [Text("hi")] AND activate_skills = ["foo"].
    //
    // For brevity, use the existing test scaffolding patterns from
    // crates/cogito-core/tests/runtime_submit.rs as a template.
    //
    // (Implementation copied from that template — see runtime_submit.rs for
    // the boilerplate: tokio runtime, in-mem store, MockModelGateway, etc.)
    //
    // The assertion that matters:
    //   if let EventPayload::TurnStarted { user_input, activate_skills } = &payload {
    //       assert_eq!(activate_skills, &vec!["foo".to_string()]);
    //       assert_eq!(user_input.len(), 1);
    //   }
}

#[tokio::test]
async fn skill_activation_no_text_projects_to_empty_user_input() {
    // Same scaffolding, trigger has names: ["foo"], user_text: None
    // assert user_input is empty AND activate_skills = ["foo"]
}
```

> The two tests above are stubs — concrete test bodies copy `runtime_submit.rs`'s
> scaffolding. The plan exists primarily to drive the projection wire-up; the
> assertions below in Step 3 confirm the behaviour.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-core --test turn_driver_skill_activation_user`

Expected: FAIL — projection of SkillActivation is not implemented; running an empty test body will compile but assert-out (or panic in the fleshed-out test bodies).

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-core/src/runtime/session_loop.rs`, locate the existing projection (around line 433):

```rust
let user_input: Vec<ContentBlock> = match trigger {
    TurnTrigger::UserText(text) => vec![ContentBlock::Text { text }],
    // `#[non_exhaustive]` guard ...
    _ => {
        tracing::error!(
            "unhandled TurnTrigger variant; dropping turn (this is a build wiring bug)"
        );
        return;
    }
};
```

Replace with:
```rust
let (user_input, activate_skills): (Vec<ContentBlock>, Vec<String>) = match trigger {
    TurnTrigger::UserText(text) => (vec![ContentBlock::Text { text }], Vec::new()),
    TurnTrigger::SkillActivation { names, user_text } => {
        let user_input = match user_text {
            Some(t) if !t.is_empty() => vec![ContentBlock::Text { text: t }],
            _ => Vec::new(),
        };
        (user_input, names)
    }
    // #[non_exhaustive] catch-all (kept for forward-compat).
    _ => {
        tracing::error!(
            "unhandled TurnTrigger variant; dropping turn (this is a build wiring bug)"
        );
        return;
    }
};
```

Then update the `TurnStarted` event construction (also in `try_start_turn`) — find the existing `record_turn_started(...)` or equivalent and add `activate_skills` argument. If the helper is named differently, follow the pattern of how `record_turn_started` signs onto the recorder via `record_*` default impls.

Add (or update) on `EventRecorder` if not already there — a `record_turn_started_with_skills` is unnecessary if the existing helper already calls `append_payload`. Inspect the current `record_turn_started` to confirm; the simplest patch is to pass an `activate_skills` field at the call site.

- [ ] **Step 4: Run test to verify it passes**

Flesh out both test bodies based on `runtime_submit.rs` scaffolding. Then run:
```
make fmt && make fix CRATE=cogito-core
cargo test -p cogito-core --test turn_driver_skill_activation_user
```

Expected: PASS — both tests assert the projection shape.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/runtime/session_loop.rs \
        crates/cogito-core/tests/turn_driver_skill_activation_user.rs
git commit -m "$(cat <<'EOF'
feat(core): project TurnTrigger::SkillActivation into TurnStarted

names → TurnStarted.activate_skills; user_text → user_input. Empty
user_text yields empty user_input (the SkillInjector's suffix is
the only user-visible content that turn).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 17: End-to-end H11 + SkillInjector integration test

**Files:**
- Test: `crates/cogito-core/tests/h11_skill_injection.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-core/tests/h11_skill_injection.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! End-to-end integration: H11 with SkillInjector configured.
//! Drives two consecutive turns: turn 1 has model emit "$foo please" via
//! MockModelGateway, turn 2 is triggered by a follow-up UserText.
//! Assertions:
//!   - Turn 2 history contains a SkillActivated event for "foo"
//!   - Turn 2's SystemPromptInjected suffix contains <skill name="foo"
//!   - No double-activation if turn 3 occurs

// Scaffolding: use cogito-mock-model + cogito-store-jsonl tempdir + Runtime
// builder; configure HarnessStrategy with system_prompt_injector = Skill.
// Inject Arc<dyn SkillProvider> = static "foo" skill.

#[tokio::test]
async fn model_sigil_in_turn1_activates_in_turn2() {
    // TODO: copy boilerplate from session_e2e.rs; pivot on the existing
    // MockModelGateway::with_replies pattern (one reply containing $foo,
    // then a plain echo on turn 2).
}

#[tokio::test]
async fn double_activation_is_skipped() {
    // Turn 1: model emits "$foo"
    // Turn 2: model again emits "$foo"
    // Expect: only one SkillActivated event in the event log.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-core --test h11_skill_injection`

Expected: FAIL (stub bodies pending).

- [ ] **Step 3: Flesh out test bodies**

Use `crates/cogito-core/tests/session_e2e.rs` as scaffolding. The key plumbing:
- Build `HarnessStrategy::default_with_model("test")` and override `strategy.context.system_prompt_injector = SystemPromptInjectorConfig::Skill;`
- Construct a static `SkillProvider` (test-only) with one skill `foo`.
- Build `RuntimeBuilder::default().store(...).model(...).tools(...).skills(provider).strategy(strategy).build()`.

After driving each turn, scan event log via `store.replay(session_id, 0)` and count `EventPayload::SkillActivated` entries.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cogito-core --test h11_skill_injection`

Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/tests/h11_skill_injection.rs
git commit -m "$(cat <<'EOF'
test(core): H11 + SkillInjector end-to-end integration

Two turns through a MockModelGateway with strategy.context.injector =
Skill. Asserts model-sigil activation in turn N+1 and idempotent
double-activation skip.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 6: CLI surface

### Task 18: `cogito-config` — `[skills]` section types

**Files:**
- Modify: `crates/cogito-config/src/types.rs`
- Test: extend `crates/cogito-config/tests/file_loader.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/cogito-config/tests/file_loader.rs`:
```rust
#[test]
fn parses_skills_section() {
    use cogito_config::types::SkillsConfig;
    let toml_text = r#"
[skills]
enabled = true
user_dir = "/tmp/.cogito/skills"
include_system = false
"#;
    let parsed: cogito_config::types::FileConfig = toml::from_str(toml_text).unwrap();
    let skills: SkillsConfig = parsed.skills.unwrap();
    assert!(skills.enabled);
    assert_eq!(skills.user_dir.as_deref(), Some("/tmp/.cogito/skills"));
    assert!(!skills.include_system);
}

#[test]
fn skills_section_optional() {
    let parsed: cogito_config::types::FileConfig =
        toml::from_str("[provider.default]\nkind = \"anthropic\"").unwrap();
    assert!(parsed.skills.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-config --test file_loader parses_skills_section skills_section_optional`

Expected: FAIL — `SkillsConfig` undefined.

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-config/src/types.rs`, add (alongside other section types):
```rust
/// `[skills]` cogito.toml section.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SkillsConfig {
    /// Master switch. When `false`, RuntimeBuilder receives no SkillProvider
    /// and selecting `SystemPromptInjectorConfig::Skill` fails at build time.
    #[serde(default = "default_skills_enabled")]
    pub enabled: bool,
    /// User scope dir. None / empty disables user scope.
    pub user_dir: Option<String>,
    /// Opt-in to bundled (System) skills.
    #[serde(default)]
    pub include_system: bool,
}

fn default_skills_enabled() -> bool { true }

impl Default for SkillsConfig {
    fn default() -> Self {
        Self { enabled: true, user_dir: None, include_system: false }
    }
}
```

Add to `FileConfig`:
```rust
pub struct FileConfig {
    // ... existing fields ...
    pub skills: Option<SkillsConfig>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-config && cargo test -p cogito-config`

Expected: PASS — 2 new tests + existing all green.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-config/src/types.rs crates/cogito-config/tests/file_loader.rs
git commit -m "$(cat <<'EOF'
feat(config): [skills] cogito.toml section

enabled (default true), user_dir, include_system. Plumbed into
RuntimeBuilder by cogito-cli in Task 19.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 19: `cogito-cli chat` — wire up SkillRegistry + slash parser

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs`
- Modify: `crates/cogito-cli/src/chat_config.rs`
- Modify: `crates/cogito-cli/Cargo.toml` (add `cogito-skills` dep)
- Test: `crates/cogito-cli/tests/slash_skill.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-cli/tests/slash_skill.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat::parse_slash_skill;
use cogito_protocol::turn_trigger::TurnTrigger;

fn assert_skill(t: &TurnTrigger, names: &[&str], user_text: Option<&str>) {
    if let TurnTrigger::SkillActivation { names: got_names, user_text: got_text } = t {
        let expected: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        assert_eq!(got_names, &expected);
        assert_eq!(got_text.as_deref(), user_text);
    } else {
        panic!("expected SkillActivation, got {t:?}");
    }
}

fn registered(names: &[&'static str]) -> impl Fn(&str) -> bool {
    let owned: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    move |n| owned.iter().any(|m| m == n)
}

#[test]
fn plain_text_returns_user_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("hello world", &r).unwrap();
    assert!(matches!(t, TurnTrigger::UserText(s) if s == "hello world"));
}

#[test]
fn single_skill_no_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo", &r).unwrap();
    assert_skill(&t, &["foo"], None);
}

#[test]
fn single_skill_with_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo do this thing", &r).unwrap();
    assert_skill(&t, &["foo"], Some("do this thing"));
}

#[test]
fn multiple_skills() {
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo bar do X", &r).unwrap();
    assert_skill(&t, &["foo", "bar"], Some("do X"));
}

#[test]
fn unknown_skill_errors() {
    let r = registered(&["foo"]);
    let err = parse_slash_skill("/skill unknown", &r).unwrap_err();
    assert!(err.to_string().contains("unknown"));
}

#[test]
fn unknown_after_known_treated_as_text_start() {
    // /skill foo unknown bar  → activate ["foo"], user_text = "unknown bar"
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo unknown bar", &r).unwrap();
    assert_skill(&t, &["foo"], Some("unknown bar"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-cli --test slash_skill`

Expected: FAIL — `parse_slash_skill` undefined.

- [ ] **Step 3: Write minimal implementation**

In `crates/cogito-cli/src/chat.rs`, add:
```rust
use thiserror::Error;

/// Errors from `parse_slash_skill`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SlashError {
    /// First token after `/skill ` was not a registered name.
    #[error("unknown skill: {0}")]
    UnknownSkill(String),
    /// `/skill` was followed by nothing.
    #[error("missing skill name after /skill")]
    Empty,
}

/// Parse a REPL input line. Returns either a plain `UserText` trigger or a
/// `SkillActivation` trigger.
///
/// Grammar:
///   "/skill <name>[ <name>...] [ <user_text>]"
///
/// Scanning rule: after `/skill `, read tokens left-to-right; each registered
/// name is added to `names`. The first unknown token (or end of input)
/// switches to user-text accumulation. The first token MUST be registered;
/// otherwise return `UnknownSkill`.
pub fn parse_slash_skill<F>(line: &str, is_registered: &F) -> Result<cogito_protocol::turn_trigger::TurnTrigger, SlashError>
where
    F: Fn(&str) -> bool,
{
    use cogito_protocol::turn_trigger::TurnTrigger;
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("/skill ").or_else(|| trimmed.strip_prefix("/skill\t")) else {
        if trimmed == "/skill" {
            return Err(SlashError::Empty);
        }
        return Ok(TurnTrigger::UserText(line.to_string()));
    };

    let mut names: Vec<String> = Vec::new();
    let mut text_start: Option<usize> = None;
    let mut cursor = 0usize;
    for (idx, tok) in rest.split_whitespace().enumerate() {
        let abs = rest[cursor..].find(tok).map(|p| cursor + p).unwrap_or(cursor);
        cursor = abs + tok.len();
        if is_registered(tok) && text_start.is_none() {
            names.push(tok.to_string());
        } else {
            // First non-name token (or unregistered) → user_text starts here.
            text_start = Some(abs);
            // Stop scanning; the rest is text.
            break;
        }
        let _ = idx;
    }

    if names.is_empty() {
        // The first token wasn't registered.
        let first = rest.split_whitespace().next().unwrap_or("");
        return Err(SlashError::UnknownSkill(first.to_string()));
    }

    let user_text = text_start.map(|pos| rest[pos..].trim().to_string()).filter(|s| !s.is_empty());

    Ok(TurnTrigger::SkillActivation { names, user_text })
}
```

Modify the REPL submit path in `cogito-cli/src/chat.rs`: wherever it currently does `session.send_user_text(line)`, replace with:
```rust
match parse_slash_skill(&line, &|n| skill_registry.is_registered(n)) {
    Ok(trigger) => session.send_trigger(trigger).await?,
    Err(SlashError::UnknownSkill(name)) => {
        eprintln!("unknown skill: {name}");
        continue;
    }
    Err(SlashError::Empty) => {
        eprintln!("usage: /skill <name> [<name>...] [ <user-text>]");
        continue;
    }
}
```

(`SessionHandle::send_trigger(TurnTrigger)` may need to be added — check existing `SessionHandle::send_user` and either add a generic `send_trigger` or have a new `activate_skills(...)` method.)

In `cogito-cli/src/chat_config.rs`, after loading the `cogito.toml` `FileConfig`, build a SkillRegistry if `[skills].enabled`:
```rust
let skills = if cfg.skills.as_ref().map(|s| s.enabled).unwrap_or(true) {
    let s = cfg.skills.clone().unwrap_or_default();
    let scan = cogito_skills::discovery::ScanConfig {
        workspace_root: Some(std::env::current_dir()?),
        user_dir: s.user_dir.map(std::path::PathBuf::from).or_else(default_user_dir),
        include_system: s.include_system,
    };
    Some(Arc::new(cogito_skills::SkillRegistry::scan(scan)?) as Arc<dyn cogito_protocol::skill::SkillProvider>)
} else {
    None
};
```

Pass `skills` to `RuntimeBuilder::skills(...)`.

Update `crates/cogito-cli/Cargo.toml` `[dependencies]`:
```toml
cogito-skills = { workspace = true }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make fmt && make fix CRATE=cogito-cli && cargo test -p cogito-cli`

Expected: PASS — all CLI tests including 6 new slash parser tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-cli/src/chat.rs crates/cogito-cli/src/chat_config.rs \
        crates/cogito-cli/Cargo.toml crates/cogito-cli/tests/slash_skill.rs
git commit -m "$(cat <<'EOF'
feat(cli): /skill <name> [text] slash parser + SkillRegistry wiring

REPL parses /skill into TurnTrigger::SkillActivation; chat_config
builds a SkillRegistry from [skills] cogito.toml + injects via
RuntimeBuilder::skills. Unknown skills are user-facing errors.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 7: Resume chaos

### Task 20: `text_then_skill_then_tool` chaos scenario

**Files:**
- Modify: `crates/cogito-core/tests/resume_chaos.rs`
- Modify: `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs`

- [ ] **Step 1: Write the failing test**

Add a new scenario to `chaos_scenarios.rs` mirroring the existing
`single_tool_happy_path` shape, but with a model reply containing a sigil:
the model emits "Sure, $foo please" then calls a tool. Two crash boundaries
(per spec §11):
  1. After `AssistantMessageAppended` containing the sigil (no activation yet)
  2. After `SkillActivated` of turn N+1, before `SystemPromptInjected`

In `resume_chaos.rs`, add:
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_then_skill_then_tool() {
    cogito_test_fixtures::chaos_scenarios::run_text_then_skill_then_tool().await;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cogito-core --test resume_chaos text_then_skill_then_tool`

Expected: FAIL — scenario undefined.

- [ ] **Step 3: Write minimal implementation**

In `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs`, append:
```rust
pub async fn run_text_then_skill_then_tool() {
    // Use the existing helpers from `single_tool_happy_path` as a base:
    //   - PanicAt at the sigil-containing AssistantMessageAppended
    //   - Resume and assert: SkillActivated written once + SystemPromptInjected
    //     present + final completed turn has the right text.
    //
    // Oracles (apply to every crash boundary):
    //   1. prefix immutable
    //   2. terminal equivalent
    //   3. tool mapping equivalent
    //   4. final text equivalent
    //
    // (Boilerplate omitted here; mirror single_tool_happy_path.)
    todo!("flesh out per single_tool_happy_path template + skill assertions");
}
```

The `todo!()` is intentional in the plan template; the engineer fleshing out this task writes the concrete scenario using the existing scaffolding — DO NOT ship the `todo!()` in committed code. The expected line count is ~80 LoC matching `single_tool_happy_path`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cogito-core --test resume_chaos text_then_skill_then_tool` and `make chaos`

Expected: PASS — all four oracles green across both crash boundaries.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/tests/resume_chaos.rs \
        crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs
git commit -m "$(cat <<'EOF'
test(chaos): text_then_skill_then_tool scenario

Crash injection at the sigil-containing AssistantMessageAppended and
between SkillActivated and SystemPromptInjected. All four oracles
(prefix immutable / terminal equivalent / tool mapping / final text)
preserved.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 8: Documentation + closure

### Task 21: Promote ADR-0020 from Proposed → Accepted

**Files:**
- Modify: `docs/adr/0020-skill-loader.md`

- [ ] **Step 1: Update status**

Change line 3 (`Status: Proposed — placeholder...`) to `Status: Accepted (Sprint 7, YYYY-MM-DD)` with today's date.

Append a "Sprint 7 closure notes" section recording the final values from this sprint:
- Sigil regex: `\$([A-Za-z][A-Za-z0-9_:-]{0,63})`
- Description cap: 1024 chars
- Repo-root stop: `.git/` OR `cogito.toml` OR fs root
- Code-fence skip: yes (Q3=A from spec)
- ADR-0023 still placeholder for bundled scripts

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0020-skill-loader.md
git commit -m "docs(adr-0020): mark Accepted with Sprint 7 closure notes"
```

---

### Task 22: H06 + H11 component docs + create `docs/skills/overview.md`

**Files:**
- Modify: `docs/components/H06-stream-demux.md`
- Modify: `docs/components/H11-context-manage.md`
- Create: `docs/skills/overview.md`

- [ ] **Step 1: H06 doc**

Add a "Sprint 7: sigil side-channel" section to `docs/components/H06-stream-demux.md`:

> H06 maintains a per-text-block `FenceState` and on each text-delta runs
> `cogito_protocol::sigil::find_sigils_outside_code`. For each hit whose name
> is registered in the injected `SkillProvider`, H06 broadcasts a
> `StreamEvent::SkillActivationRequested` (deduped per text-block). The
> broadcast surface is informational; authoritative activation lands in the
> next turn's H11 pass via the SkillInjector.

- [ ] **Step 2: H11 doc**

Add a paragraph to `docs/components/H11-context-manage.md` listing
`SkillInjector` next to `NoneInjector` under the SystemPromptInjector slot,
with a cross-reference to ADR-0020 and Sprint 7's spec.

- [ ] **Step 3: Create overview**

Create `docs/skills/overview.md`:
```markdown
# Skill Loader

Sprint 7 introduces `cogito-skills`, a Hands-layer userland extension surface.
Team members ship knowledge packs as markdown + YAML; the model activates via
`$Name` sigils and users via `/skill <name>`.

See:
- [`ADR-0020`](../adr/0020-skill-loader.md) — locked decisions
- [Sprint 7 design spec](../superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md) — implementation detail
- [`H06 stream-demux`](../components/H06-stream-demux.md) — sigil detection
- [`H11 context-manage`](../components/H11-context-manage.md) — SkillInjector

## Authoring a skill

1. Pick a kebab-case name (`my-helper`).
2. Create `<scope>/.cogito/skills/my-helper/SKILL.md` where `<scope>` is your
   workspace root (Repo) or `~/` (User).
3. Write the SKILL.md:

   ```markdown
   ---
   name: my-helper
   description: Short description shown in the model's "Available Skills" registry.
   version: 0.1.0
   ---

   # My Helper

   Detailed instructions for the model when this skill is activated.
   ```

4. Restart `cogito chat`. The model can now emit `$my-helper` to activate.

## Activation channels

- Model: write `$my-helper` in a reply. H06 detects, H11 of next turn injects.
- User: type `/skill my-helper` in `cogito chat`. CLI emits a
  `TurnTrigger::SkillActivation` trigger; the same H11 path injects.

Both produce a `SkillActivated` event in the conversation log; one event per
session per name (cross-turn dedup).

## Scope precedence

Repo > User > Plugin > System. Higher scope wins on bare-name conflict.
Plugin scope is gated on Sprint 12 / ADR-0021.
```

- [ ] **Step 4: Commit**

```bash
git add docs/components/H06-stream-demux.md docs/components/H11-context-manage.md \
        docs/skills/overview.md
git commit -m "$(cat <<'EOF'
docs(sprint-7): H06/H11 closure notes + skills/overview.md

Cross-link the new component behaviour to ADR-0020 and the design spec.
Authoring guide for skill writers under docs/skills/.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 23: Schema regen + jsonl-v1 additive entries + canonical fixture

**Files:**
- Modify: `docs/data-model/jsonl-v1.md`
- Modify: `docs/schemas/conversation-event-v1.json` (regen)
- Create: `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl`
- Modify: `crates/testing/cogito-test-fixtures/tests/fixture_roundtrip.rs`

- [ ] **Step 1: Add jsonl-v1 entries**

Append to `docs/data-model/jsonl-v1.md`:

> ### Sprint 7 additive entries (no schema bump)
>
> - `turn_started.activate_skills: string[]` — user-channel skill activations
>   carried with this turn. Optional in JSONL; defaults to `[]` on read.
> - `skill_activated` — payload `{ skill_name, source, channel }`. Written by
>   `SkillInjector` in H11. `source` is `{ kind: "repo", dir }` /
>   `{ kind: "user" }` / `{ kind: "plugin", plugin_id }` / `{ kind: "system" }`.
>   `channel` is `{ kind: "model_sigil" }` or `{ kind: "user_slash" }`.

- [ ] **Step 2: Regenerate schema**

```bash
cargo run -p cogito-gen-schema
git diff docs/schemas/conversation-event-v1.json
cargo run -p cogito-gen-schema -- --check
```

Confirm the diff is additive only (no removed required fields).

- [ ] **Step 3: Create canonical fixture**

Create `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl`:
```jsonl
{"schema_version":1,"event_id":"...","session_id":"...","turn_id":null,"seq":0,"ts":"2026-05-23T00:00:00Z","type":"session_started","data":{"meta":{...}}}
{"schema_version":1,"event_id":"...","session_id":"...","turn_id":"t1","seq":1,"ts":"...","type":"turn_started","data":{"user_input":[{"type":"text","text":"hi"}],"activate_skills":["invoice-parser"]}}
{"schema_version":1,"event_id":"...","session_id":"...","turn_id":"t1","seq":2,"ts":"...","type":"skill_activated","data":{"skill_name":"invoice-parser","source":{"kind":"user"},"channel":{"kind":"user_slash"}}}
{"schema_version":1,"event_id":"...","session_id":"...","turn_id":"t1","seq":3,"ts":"...","type":"system_prompt_injected","data":{"turn_id":"t1","suffix":"## Available Skills\n- invoice-parser: ...\n\n<skill name=\"invoice-parser\" source=\"user\">\n# body\n</skill>\n","contributors":["invoice-parser"],"produced_by":"skill"}}
```

Use `crates/testing/cogito-test-fixtures/src/bin/write_sample.rs` as the
canonical generator if helpful; extend it to emit this fixture.

- [ ] **Step 4: Roundtrip test**

In `crates/testing/cogito-test-fixtures/tests/fixture_roundtrip.rs`, add:
```rust
#[test]
fn sample_skill_v1_roundtrips() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-skill-v1.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    for line in text.lines() {
        let ev: cogito_protocol::event::ConversationEvent =
            serde_json::from_str(line).unwrap();
        let reserialized = serde_json::to_string(&ev).unwrap();
        let again: cogito_protocol::event::ConversationEvent =
            serde_json::from_str(&reserialized).unwrap();
        assert_eq!(ev, again);
    }
}
```

Run: `cargo test -p cogito-test-fixtures --test fixture_roundtrip`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/data-model/jsonl-v1.md docs/schemas/conversation-event-v1.json \
        crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl \
        crates/testing/cogito-test-fixtures/tests/fixture_roundtrip.rs
git commit -m "$(cat <<'EOF'
docs(jsonl-v1): Sprint 7 additive entries + canonical skill fixture

turn_started.activate_skills + skill_activated payload. Schema
regenerated; fixture roundtrips. No SCHEMA_VERSION bump (additive
under ADR-0007).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 24: ROADMAP tick + CHANGELOG + final `make ci`

**Files:**
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: ROADMAP**

Tick the 10 Sprint 7 checkboxes (lines under `#### Sprint 7 · Skill loader`).
Update the "Current" header to add Sprint 7 to the completed list.

- [ ] **Step 2: CHANGELOG**

Add a new section under the v0.1 in-flight entry:
```markdown
### Sprint 7 — Skill loader (ADR-0020)

- `cogito-skills` crate (Hands): SkillRegistry impl of SkillProvider; frontmatter parser; sigil regex + code-fence-aware scanner; Repo + User scope discovery.
- `cogito-protocol`: SkillProvider trait + SkillMetadata/Content/Source; SkillActivated event variant; TurnStarted.activate_skills field; TurnTrigger::SkillActivation variant; StreamEvent::SkillActivationRequested broadcast; SystemPromptInjectorConfig::Skill; EventRecorder.record_skill_activated default impl; cogito_protocol::sigil module promoted from cogito-skills for ADR-0004 layer compliance.
- `cogito-context`: SkillInjector impl of SystemPromptInjector; build_pipeline_v2 takes Option<Arc<dyn SkillProvider>>.
- `cogito-core`: H06 sigil side-channel; TurnDeps.skills field; RuntimeBuilder.skills(...); session_loop projects SkillActivation to TurnStarted.activate_skills.
- `cogito-cli`: `/skill <name> [text]` slash parser; `[skills]` cogito.toml section; SkillRegistry built at chat startup.
- Resume chaos: `text_then_skill_then_tool` scenario with crash injection at sigil/activation boundaries.
- Docs: `docs/skills/overview.md`; H06/H11 closure notes; ADR-0020 promoted to Accepted.
- Additive event-log changes (no SCHEMA_VERSION bump).
```

- [ ] **Step 3: Run full CI**

```bash
make ci
make chaos
```

Expected: both green.

- [ ] **Step 4: Commit**

```bash
git add ROADMAP.md CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(sprint-7): tick ROADMAP + CHANGELOG entry

All Sprint 7 deliverables shipped; make ci + make chaos green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-review checklist (run after every Phase)

- [ ] **Spec coverage**: every section of `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md` has at least one task implementing it.
- [ ] **Placeholder scan**: no `TODO`, `TBD`, or `todo!()` left in shipped code (test-template `todo!()` is OK only if explicitly noted).
- [ ] **Type consistency**: `SkillProvider`, `SkillMetadata`, `SkillContent`, `SkillSource`, `SkillActivationChannel`, `EventPayload::SkillActivated`, `TurnTrigger::SkillActivation`, `StreamEvent::SkillActivationRequested`, `SystemPromptInjectorConfig::Skill`, and `record_skill_activated` are spelled identically wherever they appear in this plan.
- [ ] **No backwards-compat hacks**: `TurnStarted.activate_skills` uses `#[serde(default)]` — that's the right additive pattern, not a hack.
- [ ] **Brain → Hands layer rule**: H06 imports only `cogito_protocol::sigil`, not `cogito_skills::*`. Verified by `make ci` layer-check.
- [ ] **All tests green**: `make ci && make chaos` on `feat/sprint-7-skills` HEAD.

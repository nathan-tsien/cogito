# Skill Activation Tool Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make skill activation reliable by adding an `activate_skill` tool as the primary activation channel, a mandatory forcing instruction in the injected skill index, and lean index hygiene — keeping the existing sigil/slash channels as portability fallbacks.

**Architecture:** A new stateful builtin tool (`activate_skill`) returns a skill's full `SKILL.md` body as its `ToolResult`, delivered in-turn and persisted natively by `ToolResultRecorded`. Both this tool and the existing `SkillInjector` render bodies through one shared `cogito-protocol` function so all channels deliver byte-identical content. The injected index gains a `## Skills (mandatory)` forcing instruction plus scope-ordered, capped presentation. Brain (H01–H11) is untouched; every change is an additive Hands impl, an additive protocol helper, or a presentation change in the context injector.

**Tech Stack:** Rust 2024 (MSRV 1.85), `async-trait`, `serde_json`, `cargo nextest`. Crates touched: `cogito-protocol`, `cogito-tools`, `cogito-context`, `cogito-cli`, `cogito-tui`, `cogito-test-fixtures`, `cogito-core` (tests only).

## Global Constraints

- Rust 2024 edition, MSRV 1.85; `unsafe_code = "forbid"`; `missing_docs = "warn"`.
- Clippy `pedantic` (warn) plus `unwrap_used`/`expect_used`/`panic`/`dbg_macro` **deny**; `RUSTFLAGS=-Dwarnings` — warnings break the build. Use `expect_used`/`unwrap_used` only inside `#[cfg(test)]` modules (with the existing `#[allow(...)] // tests` convention).
- All code comments (`///`, `//!`, `//`) in **English**. No decorative glyphs (`①`, `★`, `✓`, …) in comments or docs — plain `1.`/`-`.
- Errors: `thiserror` for libraries, `anyhow` for binaries. Tool failures are structured `ToolResult::Error`, never panics/`unwrap`.
- Layer rule (ADR-0004): `cogito-core::harness` imports only `cogito-protocol`. The new tool lives in `cogito-tools` (Hands); the shared renderer lives in `cogito-protocol`. No new event variant, no `SCHEMA_VERSION` bump (ADR-0007).
- Commands: `make fmt`, `make fix CRATE=<name>`, `make test CRATE=<name>`, `make ci`. Do not kill slow cargo commands.
- Commit message trailer on every commit: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Constants (locked from spec §6): `MAX_LISTED_SKILLS = 50`, `MAX_INDEX_CHARS = 8192`, per-description cap `1024` (unchanged), scope grouping default on.

---

### Task 1: Shared `render_skill_block` in `cogito-protocol`

Extract the per-skill body rendering (currently inlined in `build_body_blocks` in the context injector) into a reusable `cogito-protocol` function, then make the injector delegate to it. Behavior is byte-for-byte unchanged; this just creates the single renderer both channels will share.

**Files:**
- Modify: `crates/cogito-protocol/src/skill.rs` (add `render_skill_block` + test module)
- Modify: `crates/cogito-context/src/injector/skill.rs:221-276` (`build_body_blocks` delegates)

**Interfaces:**
- Produces: `pub fn cogito_protocol::skill::render_skill_block(content: &SkillContent) -> String` — returns a block of the exact form `\n<skill name="{name}" source="{kind}" root="{root}">\n{hint}\n{body}\n</skill>\n` when `content.root` is `Some`, or `\n<skill name="{name}" source="{kind}">\n{body}\n</skill>\n` when `None`. `{kind}` is `repo`/`user`/`plugin`/`system`/`unknown` per `SkillSource`. `{hint}` is the literal string `Bundled files for this skill live under the root path above; resolve any relative path in the instructions below against it.`
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/cogito-protocol/src/skill.rs`:

```rust
#[cfg(test)]
mod render_tests {
    use super::*;

    fn content(root: Option<PathBuf>) -> SkillContent {
        SkillContent {
            name: "brainstorming".into(),
            source: SkillSource::User,
            body: "Do the brainstorm.".into(),
            root,
        }
    }

    #[test]
    fn renders_root_attr_and_hint_when_bundled() {
        let out = render_skill_block(&content(Some(PathBuf::from("/skills/brainstorming"))));
        assert!(out.contains(r#"<skill name="brainstorming" source="user" root="/skills/brainstorming">"#));
        assert!(out.contains("resolve any relative path in the instructions below against it."));
        assert!(out.contains("Do the brainstorm."));
        assert!(out.trim_end().ends_with("</skill>"));
    }

    #[test]
    fn omits_root_attr_when_no_bundle() {
        let out = render_skill_block(&content(None));
        assert!(out.contains(r#"<skill name="brainstorming" source="user">"#));
        assert!(!out.contains("root="));
        assert!(out.contains("Do the brainstorm."));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-protocol`
Expected: FAIL — `cannot find function render_skill_block in this scope`.

- [ ] **Step 3: Add the renderer**

Add to `crates/cogito-protocol/src/skill.rs` (after the `SkillContent` definition, before the `SkillSource` enum or at end of the value-types section):

```rust
/// Render a skill's body block for delivery to the model. Shared by the
/// context injector (sigil/slash channel) and the `activate_skill` tool
/// (tool channel) so every channel delivers byte-identical content.
///
/// When `content.root` is `Some`, emits the ADR-0029 `<skill … root="…">`
/// wrapper plus a one-line resolution hint so relative references in the
/// body (`scripts/`, `references/`, `assets/`) resolve. When `None`, emits
/// the wrapper without a `root` attribute.
///
// TODO(ADR-0029): the `root` path is interpolated unescaped. Operator-
// authored skill dirs are trusted in v0.3; a directory name containing
// `"`, `>`, or a newline would break the tag and inject text into the
// prompt. Escape (or reject at discovery) before skill roots become
// tenant-controlled in the SaaS profile (Phase 3).
#[must_use]
pub fn render_skill_block(content: &SkillContent) -> String {
    use std::fmt::Write as _;

    let source_kind = match content.source {
        SkillSource::Repo { .. } => "repo",
        SkillSource::User => "user",
        SkillSource::Plugin { .. } => "plugin",
        SkillSource::System => "system",
        // `SkillSource` is `#[non_exhaustive]`; future variants render as
        // "unknown" until explicit support lands.
        _ => "unknown",
    };
    let mut out = String::new();
    match content.root.as_deref().map(std::path::Path::display) {
        Some(root) => {
            let name = &content.name;
            // Writing into a `String` via `fmt::Write` is infallible.
            let _ = write!(
                out,
                "\n<skill name=\"{name}\" source=\"{source_kind}\" root=\"{root}\">\n"
            );
            let _ = writeln!(
                out,
                "Bundled files for this skill live under the root path above; \
                 resolve any relative path in the instructions below against it."
            );
        }
        None => {
            let _ = write!(
                out,
                "\n<skill name=\"{}\" source=\"{source_kind}\">\n",
                content.name
            );
        }
    }
    out.push_str(&content.body);
    out.push_str("\n</skill>\n");
    out
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `make test CRATE=cogito-protocol`
Expected: PASS.

- [ ] **Step 5: Make the injector delegate to the shared renderer**

In `crates/cogito-context/src/injector/skill.rs`, replace the body of `build_body_blocks` so each skill is rendered via the shared function. Replace the loop body (lines ~228-273) with:

```rust
fn build_body_blocks(provider: &dyn SkillProvider, names: &[String]) -> String {
    use cogito_protocol::skill::render_skill_block;

    if names.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    for name in names {
        let Some(content) = provider.get(name) else {
            continue;
        };
        out.push_str(&render_skill_block(&content));
    }
    out
}
```

Remove the now-unused `use std::fmt::Write as _;` and the `SkillSource` import if they become unused in this file (let clippy guide you).

- [ ] **Step 6: Run the existing injector tests to confirm no behavior change**

Run: `make test CRATE=cogito-context`
Expected: PASS — existing `skill_injector` / `standard_projection` tests still green (output identical).

- [ ] **Step 7: Format, lint, commit**

```bash
make fmt && make fix CRATE=cogito-protocol && make fix CRATE=cogito-context
git add crates/cogito-protocol/src/skill.rs crates/cogito-context/src/injector/skill.rs
git commit -m "refactor(skills): shared render_skill_block in cogito-protocol

Extract per-skill body rendering into cogito-protocol::skill::render_skill_block
so the upcoming activate_skill tool and the existing SkillInjector deliver
byte-identical bodies. Injector delegates; behavior unchanged.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `activate_skill` builtin tool in `cogito-tools`

Add a stateful builtin tool that, given a skill name, returns the rendered body. Honors `disable-model-invocation` and returns structured errors for unknown/disabled skills.

**Files:**
- Create: `crates/cogito-tools/src/builtins/activate_skill.rs`
- Modify: `crates/cogito-tools/src/builtins/mod.rs` (add `pub mod activate_skill;` + re-export)
- Modify: `crates/cogito-tools/src/lib.rs` (re-export `ActivateSkill` if other builtins are re-exported there)

**Interfaces:**
- Consumes: `cogito_protocol::skill::{SkillProvider, render_skill_block}` (Task 1), `cogito_protocol::tool::{ToolDescriptor, ToolErrorKind, ToolResult, ExecutionClass}`, `crate::provider::BuiltinTool`.
- Produces: `pub struct cogito_tools::ActivateSkill` with `pub fn new(provider: std::sync::Arc<dyn SkillProvider>) -> Self`. Tool name string: `"activate_skill"`.

- [ ] **Step 1: Write the failing test**

Create `crates/cogito-tools/src/builtins/activate_skill.rs` with the test module first (implementation stub follows):

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // tests
mod tests {
    use std::sync::Arc;

    use cogito_protocol::ExecCtx;
    use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
    use cogito_protocol::tool::{ToolErrorKind, ToolResult};

    use super::ActivateSkill;
    use crate::provider::BuiltinTool;

    struct FakeProvider {
        metas: Vec<SkillMetadata>,
    }
    impl SkillProvider for FakeProvider {
        fn list(&self) -> Vec<SkillMetadata> {
            self.metas.clone()
        }
        fn get(&self, name: &str) -> Option<SkillContent> {
            self.metas.iter().find(|m| m.name == name).map(|m| SkillContent {
                name: m.name.clone(),
                source: m.source.clone(),
                body: format!("BODY for {name}"),
                root: None,
            })
        }
        fn is_registered(&self, name: &str) -> bool {
            self.metas.iter().any(|m| m.name == name)
        }
    }

    fn meta(name: &str, disable_model: bool) -> SkillMetadata {
        SkillMetadata {
            name: name.into(),
            description: "d".into(),
            source: SkillSource::User,
            disable_model_invocation: disable_model,
            user_invocable: true,
            version: None,
        }
    }

    fn tool() -> ActivateSkill {
        ActivateSkill::new(Arc::new(FakeProvider {
            metas: vec![meta("brainstorming", false), meta("locked", true)],
        }))
    }

    #[tokio::test]
    async fn returns_rendered_body_for_known_skill() {
        let r = tool().invoke(serde_json::json!({"name": "brainstorming"}), ExecCtx::default()).await;
        match r {
            ToolResult::Output(blocks) => {
                let s = blocks[0].as_str().unwrap();
                assert!(s.contains("BODY for brainstorming"));
                assert!(s.contains(r#"<skill name="brainstorming""#));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_skill_errors_with_available_list() {
        let r = tool().invoke(serde_json::json!({"name": "nope"}), ExecCtx::default()).await;
        match r {
            ToolResult::Error { kind, message, retryable } => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
                assert!(!retryable);
                assert!(message.contains("brainstorming"), "lists available: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_args_error_is_invalid_args() {
        let r = tool().invoke(serde_json::json!({"wrong": 1}), ExecCtx::default()).await;
        assert!(matches!(r, ToolResult::Error { kind: ToolErrorKind::InvalidArgs, .. }));
    }

    #[tokio::test]
    async fn disable_model_invocation_skill_is_refused() {
        let r = tool().invoke(serde_json::json!({"name": "locked"}), ExecCtx::default()).await;
        match r {
            ToolResult::Error { kind, message, .. } => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
                assert!(message.contains("/skill"), "guides user channel: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
```

(Note: confirm `ExecCtx::default()` exists; if `ExecCtx` has no `Default`, build it the way the sibling `read_file` tests build their `ctx` — check `crates/cogito-tools/src/builtins/read_file.rs` test module and reuse that constructor.)

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-tools`
Expected: FAIL — `cannot find type ActivateSkill`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/cogito-tools/src/builtins/activate_skill.rs` (above the test module):

```rust
//! `activate_skill` — primary skill-activation channel (ADR-0042). Given a
//! skill name, returns the skill's full SKILL.md body (rendered identically
//! to the sigil/slash injection path) so the model loads instructions via a
//! native tool call rather than a prose sigil. Unknown or
//! `disable-model-invocation` skills return structured errors.

use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::skill::{SkillProvider, render_skill_block};
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Loads a skill body on model request. Holds the injected `SkillProvider`.
#[derive(Clone)]
pub struct ActivateSkill {
    provider: Arc<dyn SkillProvider>,
}

impl ActivateSkill {
    /// Construct from the runtime's skill provider.
    #[must_use]
    pub fn new(provider: Arc<dyn SkillProvider>) -> Self {
        Self { provider }
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    name: String,
}

#[async_trait]
impl BuiltinTool for ActivateSkill {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "activate_skill".into(),
            description: "Load a skill's full instructions into the conversation. Call this before acting whenever a skill listed in the Skills section is relevant to the task. Returns the skill's complete SKILL.md body.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name exactly as listed in the Skills section."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, _ctx: ExecCtx) -> ToolResult {
        let Args { name } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("activate_skill args: {e}"),
                    retryable: false,
                };
            }
        };
        let Some(meta) = self.provider.get_metadata(&name) else {
            let mut available: Vec<String> =
                self.provider.list().into_iter().map(|m| m.name).collect();
            available.sort();
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!(
                    "unknown skill '{name}'; available: {}",
                    available.join(", ")
                ),
                retryable: false,
            };
        };
        if meta.disable_model_invocation {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!(
                    "skill '{name}' is user-invocable only; ask the user to run /skill {name}"
                ),
                retryable: false,
            };
        }
        let Some(content) = self.provider.get(&name) else {
            // Registered in metadata but body unavailable — treat as failure.
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("skill '{name}' has no loadable body"),
                retryable: false,
            };
        };
        ToolResult::text(render_skill_block(&content))
    }
}
```

- [ ] **Step 4: Register the module and re-export**

In `crates/cogito-tools/src/builtins/mod.rs` add (alongside the other `pub mod` lines and re-exports):

```rust
pub mod activate_skill;
pub use activate_skill::ActivateSkill;
```

If `crates/cogito-tools/src/lib.rs` re-exports builtins (e.g. `pub use builtins::ReadFile;`), add `pub use builtins::ActivateSkill;` there too, matching the existing pattern.

- [ ] **Step 5: Run tests to verify they pass**

Run: `make test CRATE=cogito-tools`
Expected: PASS (all four `activate_skill` tests green).

- [ ] **Step 6: Format, lint, commit**

```bash
make fmt && make fix CRATE=cogito-tools
git add crates/cogito-tools/src/builtins/activate_skill.rs crates/cogito-tools/src/builtins/mod.rs crates/cogito-tools/src/lib.rs
git commit -m "feat(tools): activate_skill — primary skill-activation channel (ADR-0042)

Stateful builtin returning a skill's rendered SKILL.md body. Honors
disable-model-invocation; structured errors for unknown/disabled skills.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Forcing instruction + index hygiene in `build_registry_block`

Turn the passive `## Available Skills` list into a `## Skills (mandatory)` block with an imperative instruction, scope-precedence ordering, scope grouping, and a logged total cap.

**Files:**
- Modify: `crates/cogito-context/src/injector/skill.rs:194-219` (`build_registry_block`)

**Interfaces:**
- Consumes: `SkillProvider::list()` returning `Vec<SkillMetadata>` (with `source`, `description`).
- Produces: changed `build_registry_block` output string. No new public symbols.

- [ ] **Step 1: Write the failing tests**

Add a test module to `crates/cogito-context/src/injector/skill.rs` (or extend an existing one). Note `build_registry_block` is a private fn in this module, so the tests live in the same file.

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // tests
mod registry_block_tests {
    use std::sync::Arc;

    use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};

    use super::{build_registry_block, DESCRIPTION_CAP_CHARS};

    struct P(Vec<SkillMetadata>);
    impl SkillProvider for P {
        fn list(&self) -> Vec<SkillMetadata> { self.0.clone() }
        fn get(&self, _: &str) -> Option<SkillContent> { None }
        fn is_registered(&self, n: &str) -> bool { self.0.iter().any(|m| m.name == n) }
    }
    fn m(name: &str, src: SkillSource) -> SkillMetadata {
        SkillMetadata { name: name.into(), description: "desc".into(), source: src,
            disable_model_invocation: false, user_invocable: true, version: None }
    }

    #[test]
    fn includes_mandatory_instruction_and_tool_name() {
        let p = P(vec![m("a", SkillSource::User)]);
        let out = build_registry_block(&p, DESCRIPTION_CAP_CHARS);
        assert!(out.contains("## Skills (mandatory)"));
        assert!(out.contains("you MUST"));
        assert!(out.contains("activate_skill"));
        assert!(out.contains("$<name>"), "mentions sigil fallback");
    }

    #[test]
    fn orders_repo_before_user_before_plugin() {
        let p = P(vec![
            m("u", SkillSource::User),
            m("r", SkillSource::Repo { dir: "/x".into() }),
            m("p", SkillSource::Plugin { plugin_id: "acme".into() }),
        ]);
        let out = build_registry_block(&p, DESCRIPTION_CAP_CHARS);
        let ri = out.find("- r:").unwrap();
        let ui = out.find("- u:").unwrap();
        let pi = out.find("- p:").unwrap();
        assert!(ri < ui && ui < pi, "repo<user<plugin in:\n{out}");
    }

    #[test]
    fn empty_provider_yields_empty_block() {
        let p = P(vec![]);
        assert!(build_registry_block(&p, DESCRIPTION_CAP_CHARS).is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `make test CRATE=cogito-context`
Expected: FAIL — output lacks `## Skills (mandatory)` / ordering not guaranteed.

- [ ] **Step 3: Rewrite `build_registry_block`**

Replace `build_registry_block` (and add the two module constants near `DESCRIPTION_CAP_CHARS` at the top of the file) with:

```rust
/// Max skills listed in the index before truncation (ADR-0042 §6).
const MAX_LISTED_SKILLS: usize = 50;
/// Max total characters in the index block before truncation (ADR-0042 §6).
const MAX_INDEX_CHARS: usize = 8192;

/// Sort key giving scope precedence Repo > User > Plugin > System.
fn scope_rank(s: &SkillSource) -> u8 {
    match s {
        SkillSource::Repo { .. } => 0,
        SkillSource::User => 1,
        SkillSource::Plugin { .. } => 2,
        SkillSource::System => 3,
        _ => 4,
    }
}

fn scope_header(s: &SkillSource) -> &'static str {
    match s {
        SkillSource::Repo { .. } => "### From this repository",
        SkillSource::User => "### User",
        SkillSource::Plugin { .. } => "### Plugins",
        SkillSource::System => "### Built-in",
        _ => "### Other",
    }
}

fn build_registry_block(provider: &dyn SkillProvider, cap_chars: usize) -> String {
    let mut metas = provider.list();
    if metas.is_empty() {
        return String::new();
    }
    // Stable sort by scope precedence; preserves discovery order within a scope.
    metas.sort_by_key(|m| scope_rank(&m.source));

    let total = metas.len();
    let dropped = total.saturating_sub(MAX_LISTED_SKILLS);
    if dropped > 0 {
        metas.truncate(MAX_LISTED_SKILLS);
        tracing::warn!(
            dropped,
            total,
            limit = MAX_LISTED_SKILLS,
            "skill index truncated: more skills than the listing cap"
        );
    }

    let mut out = String::from(
        "## Skills (mandatory)\n\
         Before responding, scan the skills below. If any skill is relevant to \
         the user's task — even partially — you MUST load it first by calling the \
         `activate_skill` tool with its name. (If you cannot call tools, write \
         `$<name>` instead.) Loading injects the skill's full instructions.\n\n",
    );

    let mut last_rank: Option<u8> = None;
    for m in metas {
        let rank = scope_rank(&m.source);
        if last_rank != Some(rank) {
            out.push_str(scope_header(&m.source));
            out.push('\n');
            last_rank = Some(rank);
        }
        let desc = if m.description.chars().count() > cap_chars {
            let mut t: String = m
                .description
                .chars()
                .take(cap_chars.saturating_sub(1))
                .collect();
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

    if out.len() > MAX_INDEX_CHARS {
        tracing::warn!(
            chars = out.len(),
            limit = MAX_INDEX_CHARS,
            "skill index block exceeds the char cap; downstream context budget may truncate it"
        );
    }
    out
}
```

Add `use cogito_protocol::skill::SkillSource;` to the file imports if not already present.

- [ ] **Step 4: Run tests to verify they pass**

Run: `make test CRATE=cogito-context`
Expected: PASS. If a pre-existing test asserted the literal `## Available Skills` header, update that assertion to `## Skills (mandatory)` — this is the intended behavior change.

- [ ] **Step 5: Format, lint, commit**

```bash
make fmt && make fix CRATE=cogito-context
git add crates/cogito-context/src/injector/skill.rs
git commit -m "feat(skills): mandatory forcing instruction + index hygiene (ADR-0042)

Replace passive '## Available Skills' list with '## Skills (mandatory)' +
imperative activate_skill instruction; scope-precedence ordering, scope
grouping, and logged listing/char caps. No silent truncation.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Cross-channel dedup in `collect_prior_activations`

Make `SkillInjector` treat a prior successful `activate_skill` tool call as an already-activated skill, so a skill loaded via the tool is never re-injected via the sigil path.

**Files:**
- Modify: `crates/cogito-context/src/injector/skill.rs:184-192` (`collect_prior_activations`)

**Interfaces:**
- Consumes: `cogito_protocol::event::{ConversationEvent, EventPayload}` — variants `ToolUseRecorded { call_id, tool_name, args, .. }`, `ToolResultRecorded { call_id, result }`, and `SkillActivated { skill_name, .. }`; `cogito_protocol::tool::ToolResult`.
- Produces: changed `collect_prior_activations`; no new public symbols.

- [ ] **Step 1: Write the failing test**

Add to the test area of `crates/cogito-context/src/injector/skill.rs`:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // tests
mod dedup_tests {
    use cogito_protocol::event::{ConversationEvent, EventPayload};
    use cogito_protocol::tool::ToolResult;

    use super::collect_prior_activations;

    // Build a ConversationEvent carrying the given payload. Mirror the
    // constructor the sibling tests in this crate use (see other test modules
    // in cogito-context for the exact `ConversationEvent` builder/fields).
    fn ev(payload: EventPayload) -> ConversationEvent {
        // TODO(implementer): use the same ConversationEvent construction helper
        // the existing skill.rs / standard_projection tests use.
        super::test_support::event(payload)
    }

    #[test]
    fn successful_activate_skill_tool_call_counts_as_prior_activation() {
        let history = vec![
            ev(EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "activate_skill".into(),
                args: serde_json::json!({"name": "brainstorming"}),
                message_id: None,
            }),
            ev(EventPayload::ToolResultRecorded {
                call_id: "c1".into(),
                result: ToolResult::text("…body…"),
            }),
        ];
        let prior = collect_prior_activations(&history);
        assert!(prior.contains("brainstorming"));
    }

    #[test]
    fn failed_activate_skill_tool_call_does_not_count() {
        let history = vec![
            ev(EventPayload::ToolUseRecorded {
                call_id: "c2".into(),
                tool_name: "activate_skill".into(),
                args: serde_json::json!({"name": "nope"}),
                message_id: None,
            }),
            ev(EventPayload::ToolResultRecorded {
                call_id: "c2".into(),
                result: ToolResult::Error {
                    kind: cogito_protocol::tool::ToolErrorKind::InvocationFailed,
                    message: "unknown".into(),
                    retryable: false,
                },
            }),
        ];
        let prior = collect_prior_activations(&history);
        assert!(!prior.contains("nope"));
    }
}
```

Implementer note: if there is no shared `test_support::event` helper, construct `ConversationEvent` inline exactly as the existing test modules in `crates/cogito-context/` do (grep for `ConversationEvent {` in that crate's tests to copy the field set, including `event_id`, `turn_id`, `payload`). Do not invent fields.

- [ ] **Step 2: Run test to verify it fails**

Run: `make test CRATE=cogito-context`
Expected: FAIL — tool-call activation not yet recognized.

- [ ] **Step 3: Extend `collect_prior_activations`**

Replace `collect_prior_activations` with:

```rust
fn collect_prior_activations(history: &[ConversationEvent]) -> HashSet<String> {
    use cogito_protocol::tool::ToolResult;

    let mut out = HashSet::new();
    // Pass 1: explicit SkillActivated events (sigil/slash channel).
    for ev in history {
        if let EventPayload::SkillActivated { skill_name, .. } = &ev.payload {
            out.insert(skill_name.clone());
        }
    }
    // Pass 2: successful `activate_skill` tool calls (tool channel). The body
    // is already in the persisted ToolResultRecorded, so the sigil path must
    // not re-inject it. A call counts only if its correlated result succeeded.
    let mut succeeded: HashSet<&str> = HashSet::new();
    for ev in history {
        if let EventPayload::ToolResultRecorded { call_id, result } = &ev.payload
            && matches!(result, ToolResult::Output(_))
        {
            succeeded.insert(call_id.as_str());
        }
    }
    for ev in history {
        if let EventPayload::ToolUseRecorded {
            call_id,
            tool_name,
            args,
            ..
        } = &ev.payload
            && tool_name == "activate_skill"
            && succeeded.contains(call_id.as_str())
            && let Some(name) = args.get("name").and_then(|v| v.as_str())
        {
            out.insert(name.to_string());
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `make test CRATE=cogito-context`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
make fmt && make fix CRATE=cogito-context
git add crates/cogito-context/src/injector/skill.rs
git commit -m "feat(skills): dedup tool-channel activations in SkillInjector (ADR-0042)

A successful activate_skill tool call marks the skill activated, so the
sigil path never re-injects a body already delivered via the tool result.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Wire `activate_skill` into the CLI and TUI tool providers

Register the tool wherever a `SkillProvider` is present, so the model is actually offered it.

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs:463-471` (builtin provider assembly)
- Modify: `crates/cogito-tui/src/runtime_build.rs:319-327` (builtin provider assembly)

**Interfaces:**
- Consumes: `cogito_tools::ActivateSkill` (Task 2); the local `skills: Option<Arc<dyn SkillProvider>>` already in scope at both sites.
- Produces: tool present in `BuiltinToolProvider` output when skills are configured.

- [ ] **Step 1: Inspect both sites to confirm the `skills` binding name**

Run:
```bash
sed -n '450,475p' crates/cogito-cli/src/chat.rs
sed -n '95,110p;315,330p' crates/cogito-tui/src/runtime_build.rs
```
Confirm the in-scope `Option<Arc<dyn SkillProvider>>` binding (CLI: `skills`; TUI: `skills` from `build_skill_provider`). Note whether the builder result is used inline (`.build()`) so you can refactor to a `mut` binding.

- [ ] **Step 2: Wire the CLI site**

In `crates/cogito-cli/src/chat.rs`, change the inline builder chain so the tool is appended when skills are present:

```rust
let mut builtins = BuiltinToolProvider::builder()
    .with_tool(Arc::new(ReadFile))
    .with_tool(Arc::new(cogito_tools::WriteFile))
    .with_tool(Arc::new(cogito_tools::ListDir))
    .with_tool(Arc::new(cogito_tools::Edit))
    .with_tool(Arc::new(cogito_tools::Grep))
    .with_tool(Arc::new(cogito_tools::Glob))
    .with_tool(Arc::new(cogito_tools::WebFetch::new(/* keep existing arg */)));
if let Some(sp) = &skills {
    builtins = builtins.with_tool(Arc::new(cogito_tools::ActivateSkill::new(sp.clone())));
}
// then use `builtins.build()` where the previous `.build()` result was used.
```

Match the exact `WebFetch::new(...)` argument already present (lines 470-471). Preserve the surrounding `Arc::new(...)` wrapping of the final provider.

- [ ] **Step 3: Wire the TUI site**

In `crates/cogito-tui/src/runtime_build.rs`, apply the same transform to the builder at lines 319-327, using the `skills` binding from line 102 (`build_skill_provider(&cfg, Vec::new())?`). Import `ActivateSkill` (it is re-exported from `cogito_tools`; the file already imports tools via the `cogito_tools::` path or a `use` group — follow the existing style).

```rust
let mut builtins = BuiltinToolProvider::builder()
    .with_tool(Arc::new(ReadFile))
    .with_tool(Arc::new(WriteFile))
    .with_tool(Arc::new(ListDir))
    .with_tool(Arc::new(Edit))
    .with_tool(Arc::new(Grep))
    .with_tool(Arc::new(Glob))
    .with_tool(Arc::new(WebFetch::new(cfg.tools.web_fetch.clone())));
if let Some(sp) = &skills {
    builtins = builtins.with_tool(Arc::new(ActivateSkill::new(sp.clone())));
}
```

- [ ] **Step 4: Add an integration assertion (CLI)**

In `crates/cogito-cli/tests/slash_skill.rs` (existing skill test) or a new `tests/activate_skill_wired.rs`, add a test that builds the CLI tool provider with a skill configured and asserts `provider.list()` contains a descriptor named `activate_skill`. Follow the existing harness in `slash_skill.rs` for how it constructs the provider/registry. If that wiring is not reachable from a test without a full chat run, instead assert at the unit level that `BuiltinToolProvider::builder().with_tool(Arc::new(ActivateSkill::new(provider))).build().list()` contains `activate_skill` (place in `crates/cogito-tools/`).

```rust
#[test]
fn activate_skill_is_listed_when_registered() {
    // build a provider with one skill, wire ActivateSkill, assert it lists.
    // (Construct the SkillProvider via the same path slash_skill.rs uses.)
}
```

- [ ] **Step 5: Build and test both surfaces**

Run:
```bash
make test CRATE=cogito-cli
make test CRATE=cogito-tui
```
Expected: PASS, including the new assertion.

- [ ] **Step 6: Format, lint, commit**

```bash
make fmt && make fix CRATE=cogito-cli && make fix CRATE=cogito-tui
git add crates/cogito-cli/src/chat.rs crates/cogito-tui/src/runtime_build.rs crates/cogito-cli/tests/
git commit -m "feat(cli,tui): register activate_skill when skills are configured (ADR-0042)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Resume-chaos scenario `tool_activate_skill_then_use`

Prove the tool channel rebuilds on resume: a crash around the `activate_skill` tool boundary, resumed, must reconstruct the body from `ToolResultRecorded` and let the subsequent tool use proceed.

**Files:**
- Modify: `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs` (add scenario fn + add to `all()`)
- Modify: `crates/cogito-core/tests/resume_chaos.rs` (add the inline runner, modeled on the existing `text_then_skill_then_tool` runner)

**Interfaces:**
- Consumes: `ChaosScenario` struct; `ModelEvent` variants used in scenario 6 (`TextDelta`, `TextBlockCompleted`, `ToolUseStarted`, `ToolUseCompleted`, `MessageCompleted`, `StopReason`, `Usage`).
- Produces: `pub fn tool_activate_skill_then_use() -> ChaosScenario`.

- [ ] **Step 1: Add the scenario fixture**

In `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs`, add to the `all()` vec (after `plugin_skill_then_tool()`):

```rust
        tool_activate_skill_then_use(),
```

Then add the function (place after `plugin_skill_then_tool`):

```rust
/// Scenario 8: assistant turn 1 calls the `activate_skill` tool (ADR-0042
/// primary channel), the tool returns the skill body, then the assistant
/// makes a real tool call; the tool returns; the assistant emits a final
/// reply. A crash injected around the `activate_skill` boundary must resume
/// with the body rebuilt from the persisted `ToolResultRecorded`.
///
/// The runner lives inline in `cogito-core/tests/resume_chaos.rs` (it needs a
/// `SkillProvider` exposing the `activate_skill` tool). `model_scripts[0]`
/// drives call 1 (the activate_skill tool_use), `model_scripts[1]` drives
/// call 2 (a `read_file` tool_use after the skill body), `model_scripts[2]`
/// drives call 3 (final reply after the read_file result).
#[must_use]
pub fn tool_activate_skill_then_use() -> ChaosScenario {
    ChaosScenario {
        name: "tool_activate_skill_then_use",
        user_input: vec![ContentBlock::Text {
            text: "do the task".into(),
        }],
        model_scripts: vec![
            // Call 1: call activate_skill{name: "foo"}.
            vec![
                ModelEvent::ToolUseStarted {
                    block_index: 0,
                    call_id: "s1".into(),
                    tool_name: "activate_skill".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "s1".into(),
                    tool_name: "activate_skill".into(),
                    args: serde_json::json!({"name": "foo"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage { input_tokens: 40, output_tokens: 12 },
                },
            ],
            // Call 2 (post-skill-body): make a real tool call.
            vec![
                ModelEvent::ToolUseStarted {
                    block_index: 0,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage { input_tokens: 60, output_tokens: 10 },
                },
            ],
            // Call 3 (post-tool): final reply, EndTurn.
            vec![
                ModelEvent::TextDelta { block_index: 0, chunk: "Done.".into() },
                ModelEvent::TextBlockCompleted { block_index: 0, text: "Done.".into() },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage { input_tokens: 80, output_tokens: 5 },
                },
            ],
        ],
        uses_async_job: false,
    }
}
```

Confirm the exact `ModelEvent::ToolUseStarted`/`ToolUseCompleted` field names against scenario 6 (lines 336-346) — copy them verbatim.

- [ ] **Step 2: Add the inline runner**

In `crates/cogito-core/tests/resume_chaos.rs`, find the test that drives `text_then_skill_then_tool` (grep for `text_then_skill_then_tool`). Copy its structure into a new test `tool_activate_skill_then_use_resumes`, with these differences:
- Wire the `BuiltinToolProvider` to include `ActivateSkill::new(skill_provider.clone())` plus the existing `ReadFile` (the runner already constructs a `SkillProvider` for scenario 6 — reuse it; ensure its registry contains a skill named `foo`).
- Use `tool_activate_skill_then_use()` for the scripts.
- Assert, after running to completion with crash injection at each event boundary (the existing chaos loop), that: (a) the run reaches `EndTurn`; (b) the replayed event log contains a `ToolResultRecorded` for `call_id == "s1"` whose result is `Output` containing the skill body marker; (c) no duplicate `activate_skill` dispatch occurs on resume (the cached result is reused).

Model every line on the existing scenario-6 runner; do not introduce new helpers. If the scenario-6 runner asserts via a shared `assert_resumes_identically`-style helper, reuse it.

- [ ] **Step 3: Run the chaos test**

Run: `make chaos` (or, for a faster focused loop: `cargo nextest run -p cogito-core --test resume_chaos tool_activate_skill_then_use --release`).
Expected: PASS.

- [ ] **Step 4: Format, lint, commit**

```bash
make fmt
git add crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs crates/cogito-core/tests/resume_chaos.rs
git commit -m "test(chaos): tool_activate_skill_then_use resume scenario (ADR-0042)

Crash-injected activate_skill tool boundary; resume rebuilds the body from
ToolResultRecorded and the subsequent tool use proceeds without re-dispatch.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: ADR-0042 + docs sync

Record the decision and update the affected component docs and roadmap.

**Files:**
- Create: `docs/adr/0042-skill-activation-tool-channel.md`
- Modify: `docs/adr/0020-skill-loader.md` (status note pointing to ADR-0042)
- Modify: `docs/components/H04-prompt-composer.md` (skills block now `## Skills (mandatory)` + tool channel)
- Modify: `ROADMAP.md` (v0.3 entry referencing ADR-0042)

**Interfaces:** none (docs only).

- [ ] **Step 1: Write ADR-0042**

Create `docs/adr/0042-skill-activation-tool-channel.md` following the format of `docs/adr/0041-*.md`:

```markdown
# ADR-0042: Skill activation — tool-call primary channel

## Status

Accepted (v0.3). Supersedes ADR-0020 §1 (K5 sigil-primary activation).

## Context

ADR-0020 chose sigil-in-prose (`$SkillName`) as the model activation
channel and explicitly rejected a `load_skill` tool, on two premises: that
no platform used tool-call activation, and that a tool round trip was pure
overhead versus free context injection. Both premises no longer hold:
Claude Code ships a `Skill` tool and NousResearch hermes-agent uses
`skill_view(name)`; and the round-trip cost is dominated by the cost of a
missed activation. Sigil-in-prose is out-of-distribution for tool-trained
models, the injected index carried no forcing instruction, and the sigil
guardrails (code-fence skipping, exact-name matching) create false
negatives — so activation triggered unreliably.

## Decision

1. Add an `activate_skill(name)` builtin tool as the primary activation
   channel. It returns the skill's full SKILL.md body as its `ToolResult`,
   delivered in-turn and persisted by `ToolResultRecorded`.
2. Keep sigil + slash as always-on fallbacks (multi-model portability — the
   original ADR-0020 driver). Three channels run in parallel.
3. The injected index becomes `## Skills (mandatory)` with an imperative
   "you MUST call `activate_skill`" instruction (sigil noted as the no-tools
   fallback), scope-precedence ordering, scope grouping, and logged caps.
4. No new event variant: `ToolUseRecorded` + `ToolResultRecorded` are the
   tool-channel activation record. `SkillActivated` stays sigil/slash-only.
   No `SkillActivationChannel` variant added.
5. `SkillInjector` dedups against prior successful `activate_skill` calls so
   a tool-loaded body is never re-injected via the sigil path.
6. Filtering uses existing metadata only (scope, disable-model-invocation,
   caps) — no frontmatter change (honors ADR-0033).

## Consequences

Brain delta is zero (additive Hands tool + additive protocol renderer +
context-injector presentation). Reliable activation on tool-capable models;
portability preserved for vLLM/SGLang via sigil. Spec:
`docs/superpowers/specs/2026-06-23-skill-activation-redesign-design.md`.

## References

- ADR-0020 (superseded §1), ADR-0029, ADR-0033, ADR-0004, ADR-0007.
- NousResearch hermes-agent `prompt_builder.py` / `skill_*.py`.
```

- [ ] **Step 2: Add the ADR-0020 status note**

At the top of `docs/adr/0020-skill-loader.md` Status section, add:

```markdown
> **Amendment (2026-06-23):** §1 (K5 sigil-primary activation) is superseded
> by **ADR-0042** — `activate_skill` tool is now the primary channel; sigil +
> slash remain as fallbacks. The rest of this ADR (scopes, frontmatter,
> bundled-scripts deferral) stands.
```

- [ ] **Step 3: Update H04 component doc**

In `docs/components/H04-prompt-composer.md`, update the skills-block description from `## Available Skills` (passive list) to the `## Skills (mandatory)` forcing block and note the `activate_skill` tool channel. Keep it to the paragraph(s) describing skill injection.

- [ ] **Step 4: Update ROADMAP**

Add a v0.3 bullet under the relevant sprint referencing ADR-0042 (skill-activation reliability: tool channel + forcing instruction). Match the existing bullet style (plain `-`, no glyphs). Use generic "embedding consumers" language; do not name any specific downstream consumer.

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0042-skill-activation-tool-channel.md docs/adr/0020-skill-loader.md docs/components/H04-prompt-composer.md ROADMAP.md
git commit -m "docs(skills): ADR-0042 + ADR-0020 note + H04/ROADMAP sync

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Full CI gate

- [ ] **Step 1: Run the full gate**

Run: `make ci`
Expected: fmt-check + clippy + layer-check + test all green. Layer-check must confirm `cogito-core::harness` still imports only `cogito-protocol` (the new tool is in `cogito-tools`, the renderer in `cogito-protocol` — no Brain import added).

- [ ] **Step 2: If anything fails, fix inline and re-run**

Address fmt/clippy/test failures in the owning crate; re-run `make ci` until green. Do not `#[ignore]` any test.

- [ ] **Step 3: Final verification of the spec's acceptance points**

Confirm by inspection/tests:
- `activate_skill` returns the body and is offered to the model (Tasks 2, 5).
- Index has the mandatory instruction (Task 3).
- Tool body == sigil body, byte-identical (shared renderer, Task 1).
- Cross-channel dedup holds (Task 4).
- Resume rebuilds from `ToolResultRecorded` (Task 6).

## Self-Review (author)

- **Spec coverage:** §2 levers → Tasks 2 (tool), 3 (instruction+hygiene), 4 (dedup), 1 (shared renderer/body delivery), 5 (wiring), 6 (resume), 7 (ADR/docs). §6 constants → Task 3 Global Constraints. §7 tests → Tasks 1-6 test steps. No uncovered requirement.
- **Placeholders:** the only deferred specifics are the two test-helper constructors (`ExecCtx` ctor in Task 2, `ConversationEvent` ctor in Task 4, the scenario-6 runner in Task 6) — each names the exact existing file to copy from rather than inventing an API, because those constructors are codebase-specific and must match exactly. All production code is complete.
- **Type consistency:** `render_skill_block(&SkillContent) -> String` (Task 1) consumed verbatim in Tasks 2 and 1's injector edit. `ActivateSkill::new(Arc<dyn SkillProvider>)` (Task 2) consumed in Task 5. Event variant names (`ToolUseRecorded`/`ToolResultRecorded`) consistent across Tasks 4 and 6 and the spec.

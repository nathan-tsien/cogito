//! Skill loader protocol surface — trait + value types consumed by
//! `cogito-skills` (provider impl), `cogito-context` (`SkillInjector`),
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
    /// Called once per turn by the `SkillInjector`. MUST be cheap.
    fn list(&self) -> Vec<SkillMetadata>;

    /// Full skill body (SKILL.md text, frontmatter stripped) for
    /// activation. `None` if the name is not registered.
    fn get(&self, name: &str) -> Option<SkillContent>;

    /// O(1) check used by H06 sigil filter — only registered names
    /// activate; unknown `$X` is treated as literal text.
    fn is_registered(&self, name: &str) -> bool;

    /// Metadata for a single skill (used to enforce
    /// `disable-model-invocation` / `user-invocable` at activation
    /// channels). Default impl falls back to a linear scan of `list()`
    /// so existing third-party impls keep working; impls with an O(1)
    /// table (e.g. `SkillRegistry`) should override.
    fn get_metadata(&self, name: &str) -> Option<SkillMetadata> {
        self.list().into_iter().find(|m| m.name == name)
    }

    /// Absolute on-disk roots of registered skills that have a bundle, deduped.
    /// The Runtime injects these into `ExecCtx.skill_roots` (ADR-0032) so the
    /// read-class file tools can read bundled files in place. MUST be cheap (no
    /// I/O). Default: none — for providers whose skills carry no on-disk bundle
    /// (e.g. embedded `System` skills). `SkillRegistry` overrides it.
    fn skill_roots(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
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
    /// Path to the skill's own directory (the folder containing
    /// `SKILL.md`), so the model can resolve relative references in the
    /// body (`scripts/`, `references/`, `assets/`). Absolute when the
    /// provider's configured roots are absolute (the registry does not
    /// canonicalize). `None` for skills with no on-disk bundle (e.g. future
    /// embedded `System` or virtual providers). See ADR-0029.
    ///
    /// This field itself is not part of any persisted event. Note, however,
    /// that `SkillInjector` renders it into the system-prompt suffix, and
    /// that suffix *is* persisted in the `SystemPromptInjected` event and
    /// replayed on resume. So a concrete (machine-specific) path does reach
    /// the event log indirectly. That is fine for single-machine resume;
    /// machine-independent multi-replica resume (v0.4) will need the path
    /// re-resolved at prompt-build time rather than frozen into the suffix.
    pub root: Option<PathBuf>,
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

    // `SkillSource` is `#[non_exhaustive]`; the wildcard arm is unreachable
    // within this crate but is required for callers in other crates that may
    // encounter future variants. Allow the lint locally.
    #[allow(unreachable_patterns)]
    let source_kind = match content.source {
        SkillSource::Repo { .. } => "repo",
        SkillSource::User => "user",
        SkillSource::Plugin { .. } => "plugin",
        SkillSource::System => "system",
        // Future variants render as "unknown" until explicit support lands.
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
        assert!(out.contains(
            r#"<skill name="brainstorming" source="user" root="/skills/brainstorming">"#
        ));
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

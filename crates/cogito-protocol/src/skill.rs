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

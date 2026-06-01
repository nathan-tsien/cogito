//! Hands-layer Skill loader. See `docs/skills/overview.md` and
//! `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md`.

pub mod discovery;
pub mod metadata;
pub mod registry;
pub mod sigil;

pub use discovery::{PluginSkillRoot, ScanConfig};
pub use registry::{SkillRegistry, SkillRegistryError};

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn crate_compiles() {
        // Placeholder; later tasks add real surface.
    }
}

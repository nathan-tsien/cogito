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
    /// Resolved `[runtime]` section.
    pub runtime: RuntimeSection,
    /// Resolved provider list.
    pub providers: Vec<ProviderConfig>,
    /// Sprint 4.5: always empty. Sprint 5 populates by walking
    /// `runtime.strategies_dir`.
    pub strategies: HashMap<String, HarnessStrategy>,
}

/// Finalized `[runtime]` section. All fields are resolved (no `Option`
/// where a default exists).
#[derive(Debug, Clone)]
pub struct RuntimeSection {
    /// Root directory for per-session JSONL stores.
    pub session_root: PathBuf,
    /// Name of the provider chosen at runtime when callers do not pick
    /// one explicitly.
    pub default_provider: Option<String>,
    /// Default model identifier; provider-specific.
    pub default_model: Option<String>,
    /// Directory scanned for strategy files (Sprint 5+).
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
    /// Optional `[runtime]` section contribution.
    pub runtime: Option<RuntimeSectionPartial>,
    /// Optional `[[providers]]` array contribution.
    pub providers: Option<Vec<ProviderConfig>>,
}

/// Partial `[runtime]` section. Every field is `Option<T>` so the
/// merge step can treat `None` as "do not contribute".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSectionPartial {
    /// Override for `RuntimeSection::session_root`.
    pub session_root: Option<PathBuf>,
    /// Override for `RuntimeSection::default_provider`.
    pub default_provider: Option<String>,
    /// Override for `RuntimeSection::default_model`.
    pub default_model: Option<String>,
    /// Override for `RuntimeSection::strategies_dir`.
    pub strategies_dir: Option<PathBuf>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        assert_eq!(
            back.runtime.as_ref().unwrap().default_provider,
            p.runtime.as_ref().unwrap().default_provider
        );
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

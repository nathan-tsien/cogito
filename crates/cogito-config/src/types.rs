//! Value types for cogito runtime configuration. See ADR-0017 §12 for
//! the locked shape and `docs/configuration/overview.md` §6 for the
//! human-facing reference.

use std::collections::HashMap;
use std::path::PathBuf;

use cogito_mcp::{McpServerConfig, McpStartupFailure};
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
    /// Sprint 4 (ADR-0018): successfully-parsed MCP server entries.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Sprint 4 (ADR-0018): per-entry deserialization failures.
    /// Surface code joins these with handshake-time failures from
    /// `build_mcp_provider` and surfaces them in the startup banner.
    pub mcp_parse_failures: Vec<McpStartupFailure>,
    /// Sprint 7: optional `[skills]` section. Surfaces (CLI / TUI) use
    /// this to construct a `SkillRegistry` and inject it into
    /// `RuntimeBuilder::skills`. `None` is equivalent to "section
    /// omitted"; finalize does not synthesize a default so absence
    /// stays distinguishable from `enabled = true`.
    pub skills: Option<SkillsConfig>,
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
    /// Optional default strategy name. If `None` and `--strategy`
    /// is not given, `resolve_strategy` synthesizes a strategy from
    /// `default_model` + CLI flags. See ADR-0026.
    pub default_strategy: Option<String>,
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
    /// Optional `[[mcp_servers]]` array (Sprint 4). Stored as raw
    /// TOML values so per-entry deserialization can be deferred to
    /// finalize, where a bad entry becomes a `McpStartupFailure`
    /// instead of poisoning the whole parse. See ADR-0018 §3.
    pub mcp_servers: Option<Vec<toml::Value>>,
    /// Optional `[skills]` section (Sprint 7). Plumbed into
    /// `RuntimeBuilder` by `cogito-cli` to build a `SkillRegistry`.
    pub skills: Option<SkillsConfig>,
}

/// `[skills]` cogito.toml section.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsConfig {
    /// Master switch. When `false`, `RuntimeBuilder` receives no `SkillProvider`
    /// and selecting `SystemPromptInjectorConfig::Skill` fails at build time.
    #[serde(default = "default_skills_enabled")]
    pub enabled: bool,
    /// User scope dir. None / empty disables user scope.
    pub user_dir: Option<String>,
    /// Opt-in to bundled (System) skills.
    #[serde(default)]
    pub include_system: bool,
}

fn default_skills_enabled() -> bool {
    true
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_dir: None,
            include_system: false,
        }
    }
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
    /// Override for `RuntimeSection::default_strategy`.
    pub default_strategy: Option<String>,
    /// Override for `RuntimeSection::strategies_dir`.
    pub strategies_dir: Option<PathBuf>,
}

/// Per-entry try-deserialize. Successes go to the typed list;
/// failures become `McpStartupFailure::ConfigParse` carrying the
/// 0-based index and the deserialization error message.
pub(crate) fn finalize_mcp_servers(
    raw: Option<Vec<toml::Value>>,
) -> (Vec<McpServerConfig>, Vec<McpStartupFailure>) {
    let Some(entries) = raw else {
        return (Vec::new(), Vec::new());
    };
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for (i, value) in entries.into_iter().enumerate() {
        match value.try_into::<McpServerConfig>() {
            Ok(cfg) => ok.push(cfg),
            Err(e) => errs.push(McpStartupFailure::ConfigParse {
                index: i,
                error: e.to_string(),
            }),
        }
    }
    (ok, errs)
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
                default_strategy: Some("coder".into()),
                strategies_dir: Some(PathBuf::from(".cogito/strategies")),
            }),
            providers: Some(vec![]),
            mcp_servers: None,
            skills: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: RuntimeConfigPartial = serde_json::from_str(&s).unwrap();
        assert_eq!(
            back.runtime.as_ref().unwrap().default_provider,
            p.runtime.as_ref().unwrap().default_provider
        );
        assert_eq!(
            back.runtime.as_ref().unwrap().default_strategy,
            p.runtime.as_ref().unwrap().default_strategy
        );
    }

    #[test]
    fn empty_partial_default_is_all_none() {
        let p = RuntimeConfigPartial::default();
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
        assert!(p.mcp_servers.is_none());
        assert!(p.skills.is_none());
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

    #[test]
    fn mcp_servers_round_trips_through_partial_as_raw_toml_values() {
        let toml_str = r#"
            [[mcp_servers]]
            name = "fs"
            transport = "stdio"
            command = "uvx"
            args = ["mcp-server-filesystem", "/tmp"]
        "#;
        let partial: RuntimeConfigPartial = toml::from_str(toml_str).unwrap();
        let raw = partial.mcp_servers.expect("mcp_servers parsed");
        assert_eq!(raw.len(), 1);
        // Raw form: each entry is still a toml::Value::Table.
        assert!(raw[0].as_table().is_some());
    }

    #[test]
    fn bad_mcp_entry_does_not_poison_provider_parse() {
        let toml_str = r#"
            [[mcp_servers]]
            name = "good"
            transport = "stdio"
            command = "echo"

            [[mcp_servers]]
            name = "bad"
            transport = "websocket"
            url = "ws://x"
        "#;
        let partial: RuntimeConfigPartial =
            toml::from_str(toml_str).expect("top-level parse must succeed even with bad mcp entry");
        let (ok, errs) = finalize_mcp_servers(partial.mcp_servers);
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].name, "good");
        assert_eq!(errs.len(), 1);
        let McpStartupFailure::ConfigParse { index, .. } = &errs[0] else {
            panic!("expected ConfigParse");
        };
        assert_eq!(*index, 1);
    }

    #[test]
    fn skills_config_rejects_unknown_field() {
        // A typo like `userdir` (missing underscore) must surface rather
        // than silently degrade to the default.
        let toml_str = r#"
            [skills]
            enabled = true
            userdir = "/tmp/skills"
        "#;
        let err = toml::from_str::<RuntimeConfigPartial>(toml_str)
            .expect_err("unknown [skills] field must error");
        let msg = err.to_string();
        assert!(
            msg.contains("userdir") || msg.contains("unknown"),
            "error should mention the offending field, got: {msg}"
        );
    }

    #[test]
    fn missing_mcp_servers_section_yields_empty_lists() {
        let partial: RuntimeConfigPartial = toml::from_str(
            r#"
            [runtime]
            session_root = "/tmp/x"
        "#,
        )
        .unwrap();
        let (ok, errs) = finalize_mcp_servers(partial.mcp_servers);
        assert!(ok.is_empty());
        assert!(errs.is_empty());
    }
}

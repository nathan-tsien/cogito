//! Serde structs mirroring the YAML frontmatter shape. Parsed into
//! `StrategyFrontmatter`, then materialized into a fully-resolved
//! `HarnessStrategy` by `parser::materialize`.

use std::path::PathBuf;

use cogito_protocol::context::ContextConfig;
use cogito_protocol::gateway::ModelParams;
use serde::Deserialize;

/// Direct deserialize target of the YAML frontmatter block.
/// All fields except `name` are optional.
//
// `dead_code` is allowed because Task 05 (`parser.rs`) is the first
// consumer of `tool_order` and `context`; without the allow, the
// workspace `-Dwarnings` flag fails the build between tasks.
#[allow(dead_code)]
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

    /// Inner-loop safety budget. `None` -> 16 (`HarnessStrategy` default).
    #[serde(default)]
    pub max_turns: Option<u32>,

    /// Sampling knobs. Strategy keys win on overlay with provider-level
    /// `model_params` (overlay performed by the wiring layer).
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
    /// Sampling temperature override.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Output-token cap override.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Top-p nucleus sampling override.
    #[serde(default)]
    pub top_p: Option<f32>,
    /// Stop-sequence list override.
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
}

impl ModelParamsPartial {
    /// Overlay `self` onto `base`. Self's `Some` keys win.
    pub(crate) fn overlay(&self, base: &mut ModelParams) {
        if let Some(t) = self.temperature {
            base.temperature = Some(t);
        }
        if let Some(mt) = self.max_tokens {
            base.max_tokens = mt;
        }
        if let Some(p) = self.top_p {
            base.top_p = Some(p);
        }
        if let Some(s) = &self.stop_sequences {
            base.stop_sequences.clone_from(s);
        }
    }
}

/// Frontmatter override of the markdown body's role as `system_prompt`.
///
/// - `Inline(String)`: `system_prompt: just a string`
/// - `FileRef { file }`: `system_prompt: { file: ./path.md }` — path
///   relative to the strategy `.md` file's directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum SystemPromptSource {
    /// Inline string literal in the YAML.
    Inline(String),
    /// `{ file: <path> }` form; path is relative to the strategy file.
    FileRef {
        /// Path (relative to the strategy file's directory) of the
        /// markdown file whose contents become the system prompt.
        file: PathBuf,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        let yaml = r"
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
";
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
        assert!(
            matches!(fm.system_prompt, Some(SystemPromptSource::Inline(ref s)) if s == "hello world")
        );
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
        assert!(
            r.is_err(),
            "deny_unknown_fields should have rejected `bogus`"
        );
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

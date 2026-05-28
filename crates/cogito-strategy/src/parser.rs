//! Parse a strategy markdown file into a fully-resolved `HarnessStrategy`.
//!
//! Two steps:
//! 1. Split `---`-delimited YAML frontmatter from the markdown body.
//! 2. Materialize: deserialize frontmatter, resolve `system_prompt`
//!    (frontmatter override > inline body), apply `name`-vs-filename
//!    check, hoist `model` into `ModelParams`.
//!
//! `dead_code` is allowed crate-wide for the items in this module until
//! Task 06 exposes `parse_strategy_file` as `pub` for integration tests;
//! at that point the allow drops out.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use cogito_protocol::gateway::ModelParams;
use cogito_protocol::strategy::{HarnessStrategy, ToolFilter};

use crate::error::LoadError;
use crate::schema::{StrategyFrontmatter, SystemPromptSource};

/// Parse `path` into a fully-resolved `HarnessStrategy` plus the
/// optional `provider:` reference (for the wiring layer to cross-check
/// against `cogito.toml`).
///
/// # Errors
///
/// Returns `LoadError` if the file cannot be read, the frontmatter is
/// missing/malformed, the declared `name` does not match the filename,
/// or both the frontmatter `system_prompt` and the markdown body are empty.
pub(crate) fn parse_strategy_file(path: &Path) -> Result<ParsedStrategy, LoadError> {
    let raw = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let (frontmatter_text, body) =
        split_frontmatter(&raw).ok_or_else(|| LoadError::Frontmatter {
            path: path.to_path_buf(),
        })?;

    let fm: StrategyFrontmatter =
        serde_yaml::from_str(frontmatter_text).map_err(|source| LoadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

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
        context: fm.context.clone().unwrap_or_default(),
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
    /// Fully-materialized strategy (system prompt already resolved).
    pub strategy: HarnessStrategy,
    /// Optional provider reference (`cogito.toml` provider id).
    pub provider: Option<String>,
    /// `true` iff the strategy file explicitly set a `model:` value.
    pub model_present: bool,
    /// Human description (surfaced by `--list-strategies`).
    pub description: Option<String>,
    /// Path of the strategy `.md` file (used in error messages).
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

    let after_first = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))?;

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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        assert!(
            body.contains("more body"),
            "second `---` must NOT be treated as a new frontmatter close"
        );
    }

    #[test]
    fn strips_utf8_bom() {
        let raw = "\u{feff}---\nname: foo\n---\nbody\n";
        let (yaml, body) = split_frontmatter(raw).unwrap();
        assert_eq!(yaml, "name: foo\n");
        assert_eq!(body, "body\n");
    }
}

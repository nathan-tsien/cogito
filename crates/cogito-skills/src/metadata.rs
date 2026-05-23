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
    let description = raw
        .description
        .ok_or(ParseError::MissingField("description"))?;

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
    let Some(first) = iter.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    iter.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
}

/// Number of UTF-8 bytes used by the ellipsis sentinel `'…'`.
const ELLIPSIS_BYTES: usize = '…'.len_utf8();

fn cap_description(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap && s.len() <= cap {
        return s.to_string();
    }
    // Reserve room for the UTF-8 ellipsis `'…'` so the final byte length
    // stays within `cap`. We also bound the char count to `cap - 1` for the
    // chars-style interpretation. Whichever limit kicks in first wins.
    let byte_budget = cap.saturating_sub(ELLIPSIS_BYTES);
    let char_budget = cap.saturating_sub(1);
    let mut out = String::new();
    for (chars_taken, ch) in s.chars().enumerate() {
        if chars_taken >= char_budget {
            break;
        }
        if out.len() + ch.len_utf8() > byte_budget {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

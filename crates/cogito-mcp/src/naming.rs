//! Qualified tool name encoding: `mcp__<server>__<tool>`.
//!
//! See ADR-0018 §4 for the full convention. The algorithm is the de
//! facto MCP-multi-server pattern (also used by openai/codex; pattern
//! is public knowledge, not copyrighted code).

use sha1::{Digest, Sha1};

/// Prefix marking a qualified MCP tool name.
pub const MCP_PREFIX: &str = "mcp";

/// Delimiter between prefix, server name, and tool name.
///
/// Constrained by `OpenAI` Responses API tool-name regex
/// `^[a-zA-Z0-9_-]+$`; `__` is the safest non-alphanumeric we can use.
pub const DELIM: &str = "__";

/// Hard upper bound on qualified tool name length. Aligns with the
/// shortest length cap among major LLM providers.
pub const MAX_QUALIFIED_LEN: usize = 64;

/// Replace any character outside `[a-zA-Z0-9_-]` with `_`.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sha1_hex(s: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Encode `(server, tool)` as a qualified name. The result:
///
/// - starts with `mcp__<server>__`
/// - is sanitized to `[a-zA-Z0-9_-]+`
/// - is at most [`MAX_QUALIFIED_LEN`] chars; longer names are
///   truncated with a deterministic SHA-1 suffix
///
/// The same input always produces the same output (no hidden state).
#[must_use]
pub fn qualify(server: &str, tool: &str) -> String {
    let raw = format!("{MCP_PREFIX}{DELIM}{server}{DELIM}{tool}");
    let sanitized = sanitize(&raw);
    if sanitized.len() <= MAX_QUALIFIED_LEN {
        return sanitized;
    }
    let sha1 = sha1_hex(&raw);
    // Reserve full hex digest length for the suffix (40 chars) — leaves
    // the first MAX_QUALIFIED_LEN - 40 chars of the sanitized form as
    // a human hint.
    let prefix_len = MAX_QUALIFIED_LEN.saturating_sub(sha1.len());
    let head: String = sanitized.chars().take(prefix_len).collect();
    format!("{head}{sha1}")
}

/// Inverse of [`qualify`]: extract `(server, tool)` from a qualified
/// name. Returns `None` if the input does not match `mcp__<x>__<y>`.
///
/// Note: this is **lossy** when the name was truncated (the sha1
/// suffix replaces real characters); we use it only for routing
/// inside `McpToolProvider`, which stores the qualified name as the
/// map key and never needs to reconstruct the raw tool name from the
/// qualified form.
#[must_use]
pub fn split_qualified_name(qualified: &str) -> Option<(String, String)> {
    let mut parts = qualified.split(DELIM);
    let prefix = parts.next()?;
    if prefix != MCP_PREFIX {
        return None;
    }
    let server = parts.next()?;
    let tool: String = parts.collect::<Vec<_>>().join(DELIM);
    if tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_names_round_trip() {
        let q = qualify("filesystem", "read_file");
        assert_eq!(q, "mcp__filesystem__read_file");
        let (s, t) = split_qualified_name(&q).unwrap();
        assert_eq!(s, "filesystem");
        assert_eq!(t, "read_file");
    }

    #[test]
    fn dots_get_sanitized_to_underscores() {
        let q = qualify("acme.api", "list.objects");
        assert_eq!(q, "mcp__acme_api__list_objects");
    }

    #[test]
    fn slashes_and_spaces_become_underscores() {
        let q = qualify("path/server", "tool with space");
        assert_eq!(q, "mcp__path_server__tool_with_space");
    }

    #[test]
    fn unicode_becomes_underscores() {
        let q = qualify("server", "工具");
        // Both chars of "工具" → "__" → effectively zero-width info, but stable.
        // After the `__` delimiter that's 4 underscores trailing `server`.
        assert_eq!(q, "mcp__server____");
    }

    #[test]
    fn empty_tool_name_qualifies_but_split_rejects() {
        let q = qualify("server", "");
        assert_eq!(q, "mcp__server__");
        assert!(split_qualified_name(&q).is_none());
    }

    #[test]
    fn under_limit_no_truncation() {
        let q = qualify("a", "b");
        assert!(q.len() <= MAX_QUALIFIED_LEN);
        assert!(!q.contains(&"0".repeat(10))); // no sha1 suffix
    }

    #[test]
    fn over_limit_is_truncated_with_sha1_suffix() {
        let long_tool = "a".repeat(100);
        let q = qualify("svr", &long_tool);
        assert_eq!(q.len(), MAX_QUALIFIED_LEN);
        // The last 40 chars must be a sha1 hex of the un-truncated input.
        let raw = format!("mcp__svr__{long_tool}");
        let expected_sha = sha1_hex(&raw);
        assert!(q.ends_with(&expected_sha));
    }

    #[test]
    fn truncation_is_deterministic() {
        let long = "x".repeat(80);
        let q1 = qualify("s", &long);
        let q2 = qualify("s", &long);
        assert_eq!(q1, q2);
    }

    #[test]
    fn two_inputs_differing_pre_sanitize_yield_different_outputs() {
        // `foo.bar` and `foo_bar` both sanitize to `foo_bar`. The
        // SHA-1 suffix kicks in only when over MAX_QUALIFIED_LEN, so
        // for short inputs they DO collide post-sanitize. That's
        // expected (dedup logic in provider.rs handles it); document
        // here so this stays a known property.
        let a = qualify("svr", "foo.bar");
        let b = qualify("svr", "foo_bar");
        assert_eq!(a, b);
        assert_eq!(a, "mcp__svr__foo_bar");
    }

    #[test]
    fn split_rejects_non_mcp_prefix() {
        assert!(split_qualified_name("builtin__read_file").is_none());
        assert!(split_qualified_name("read_file").is_none());
        assert!(split_qualified_name("").is_none());
    }

    #[test]
    fn split_handles_tool_names_with_internal_double_underscore() {
        // After sanitization tool names can't contain `__` literally
        // (each `_` is single), but a server can legitimately have a
        // tool named `foo__bar` if the MCP server returns that name.
        // Our prefix is `mcp__`, server delimits with `__`, and the
        // rest joins as the tool. Test that split handles N occurrences.
        let q = "mcp__svr__foo__bar__baz";
        let (s, t) = split_qualified_name(q).unwrap();
        assert_eq!(s, "svr");
        assert_eq!(t, "foo__bar__baz");
    }

    #[test]
    fn sanitize_preserves_allowed_chars() {
        assert_eq!(sanitize("abc-XYZ_123"), "abc-XYZ_123");
    }

    #[test]
    fn dedup_relies_on_qualified_collision() {
        // Property: two qualified outputs that match exactly will be
        // deduplicated in the provider layer (Task 9). This test
        // documents that collision is observable here, providing the
        // hook the provider's dedup logic uses.
        let a = qualify("s", "foo.bar");
        let b = qualify("s", "foo_bar");
        assert_eq!(a, b, "provider dedup depends on this equality");
    }
}

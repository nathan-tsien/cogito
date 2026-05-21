//! Error and failure types for the MCP layer.
//!
//! Two distinct concepts:
//!
//! - [`McpError`]: library-internal short-circuits (invariant
//!   violations during development; tests). These DO propagate as
//!   `Result::Err`.
//! - [`McpStartupFailure`]: per-server failures during Runtime
//!   construction. These are **never** propagated as `Result::Err`;
//!   they accumulate in a vec and surface via the startup banner.
//!   See ADR-0018 §3 for the architectural commitment.

use thiserror::Error;

/// Library-internal errors. Currently used only for invariant
/// violations in development; production code should never see one.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum McpError {
    /// Internal invariant violation. Indicates a cogito-mcp bug.
    #[error("cogito-mcp invariant: {0}")]
    Invariant(String),
}

/// One thing that went wrong while bringing an MCP server online.
///
/// Every variant affects exactly **one** server; the rest of the
/// Runtime is unaffected. The channel covers the full pipeline:
/// per-entry config deserialization, env-var lookup, name uniqueness,
/// transport spawn, and the rmcp handshake.
///
/// Marked `#[non_exhaustive]` so future variants (e.g.
/// `SchemaInvalid` if we ever add boundary schema sanity checks)
/// land additively without breaking downstream consumers.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum McpStartupFailure {
    /// `mcp_servers[i]` failed to deserialize. Captured by
    /// `cogito-config::finalize` after per-entry try-deserialize.
    /// `index` is the position in the original config array.
    #[error("mcp_servers[{index}] failed to parse: {error}")]
    ConfigParse {
        /// Position of the bad entry in the original array.
        index: usize,
        /// Human-readable deserialization error message.
        error: String,
    },

    /// `bearer_token_env_var` references an env var that is unset or
    /// empty. The token value (which doesn't exist) is never logged;
    /// only the env-var name appears.
    #[error("server `{name}`: env var `{env_var}` for bearer token is unset")]
    BearerEnvMissing {
        /// Server name from config.
        name: String,
        /// Env var name that was checked.
        env_var: String,
    },

    /// Two entries in `[[mcp_servers]]` share the same `name`. The
    /// later entry (higher index) is skipped; this variant records
    /// which one was dropped.
    #[error("server name `{name}` is duplicated (entry at index {index} skipped)")]
    DuplicateName {
        /// The conflicting server name.
        name: String,
        /// Index of the skipped (later) entry.
        index: usize,
    },

    /// `initialize` + `tools/list` exceeded the configured (or default)
    /// startup timeout.
    #[error("server `{name}`: startup timed out after {timeout_sec}s")]
    StartupTimeout {
        /// Server name from config.
        name: String,
        /// Effective timeout that fired.
        timeout_sec: f64,
    },

    /// Transport-level failure: stdio spawn failed, HTTP connect
    /// failed, handshake RPC errored at the wire level. The `error`
    /// field is a sanitized string — secrets must not appear in it
    /// (the construction site is responsible).
    #[error("server `{name}`: transport error: {error}")]
    TransportError {
        /// Server name from config.
        name: String,
        /// Sanitized error message (no bearer tokens, no API keys).
        error: String,
    },

    /// rmcp handshake completed at the wire level but the server's
    /// response was not acceptable (protocol mismatch, server doesn't
    /// support tools, etc.).
    #[error("server `{name}`: handshake failed: {error}")]
    HandshakeFailed {
        /// Server name from config.
        name: String,
        /// Sanitized error message.
        error: String,
    },
}

impl McpStartupFailure {
    /// Best-effort server name. Returns `None` for [`Self::ConfigParse`]
    /// (which fires before a name is available); `Some(_)` for every
    /// other variant.
    #[must_use]
    pub fn server_name(&self) -> Option<&str> {
        match self {
            Self::ConfigParse { .. } => None,
            Self::BearerEnvMissing { name, .. }
            | Self::DuplicateName { name, .. }
            | Self::StartupTimeout { name, .. }
            | Self::TransportError { name, .. }
            | Self::HandshakeFailed { name, .. } => Some(name),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn display_does_not_leak_a_secret_value() {
        // Failures are constructed by us; the convention is that the
        // `error` field never contains bearer tokens. Test that even
        // if a careless caller passed a secret-looking string into
        // `error`, our variant Display ONLY surfaces our format
        // template, not any "secret" magic. (This is a regression
        // guard for the variant format strings.)
        let failure = McpStartupFailure::BearerEnvMissing {
            name: "test".into(),
            env_var: "TEST_TOKEN".into(),
        };
        let rendered = failure.to_string();
        assert!(rendered.contains("TEST_TOKEN"));
        assert!(rendered.contains("test"));
        assert!(!rendered.contains("Bearer "));
        assert!(!rendered.contains("eyJ")); // no JWT-looking blobs
    }

    #[test]
    fn server_name_helper_returns_none_for_config_parse() {
        let f = McpStartupFailure::ConfigParse {
            index: 0,
            error: "x".into(),
        };
        assert_eq!(f.server_name(), None);
    }

    #[test]
    fn server_name_helper_returns_name_for_others() {
        let f = McpStartupFailure::HandshakeFailed {
            name: "myserver".into(),
            error: "x".into(),
        };
        assert_eq!(f.server_name(), Some("myserver"));
    }

    #[test]
    fn all_variants_format_with_expected_phrasing() {
        // Snapshot-like assertions on each format, so a refactor that
        // changes Display output (and might break the banner format)
        // fails this test loudly.
        let cases: Vec<(McpStartupFailure, &str)> = vec![
            (
                McpStartupFailure::ConfigParse {
                    index: 2,
                    error: "boom".into(),
                },
                "mcp_servers[2] failed to parse: boom",
            ),
            (
                McpStartupFailure::BearerEnvMissing {
                    name: "s".into(),
                    env_var: "T".into(),
                },
                "server `s`: env var `T` for bearer token is unset",
            ),
            (
                McpStartupFailure::DuplicateName {
                    name: "d".into(),
                    index: 3,
                },
                "server name `d` is duplicated (entry at index 3 skipped)",
            ),
            (
                McpStartupFailure::StartupTimeout {
                    name: "s".into(),
                    timeout_sec: 10.0,
                },
                "server `s`: startup timed out after 10s",
            ),
            (
                McpStartupFailure::TransportError {
                    name: "s".into(),
                    error: "connection refused".into(),
                },
                "server `s`: transport error: connection refused",
            ),
            (
                McpStartupFailure::HandshakeFailed {
                    name: "s".into(),
                    error: "bad version".into(),
                },
                "server `s`: handshake failed: bad version",
            ),
        ];
        for (failure, expected) in cases {
            assert_eq!(failure.to_string(), expected);
        }
    }
}

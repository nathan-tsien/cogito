# Sprint 4 · MCP sync tools — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pull `cogito-mcp` from v0.2 forward into v0.1 Sprint 4: a `ToolProvider` impl that aggregates tools from any number of configured MCP servers (stdio + streamable-HTTP), with `mcp__<server>__<tool>` qualified naming, soft-skip failure model (Runtime never blocked by MCP failures), and a startup banner that surfaces per-server status.

**Architecture:** New `cogito-mcp` Hand crate wrapping `rmcp` 1.5 (Apache-2.0 upstream SDK from `modelcontextprotocol/rust-sdk`). `McpToolProvider` implements `cogito_protocol::tool::ToolProvider`; `build_mcp_provider` returns `McpProviderBuildResult { provider, failures }` (NOT `Result<_, _>`) — this is the **compiler-enforced** form of ADR-0018 §3 "MCP failures are non-fatal to Runtime". `cogito-config` carries `mcp_servers` as `Vec<toml::Value>` until finalize-time per-entry deserialization, so one bad entry never poisons the whole TOML parse. `cogito-cli chat` wires the provider into a `CompositeToolProvider` and prints a stderr banner for visibility.

**Tech Stack:** Rust 2024 (workspace MSRV 1.85) · `rmcp = "1.5"` (features: `client`, `transport-child-process`, `transport-streamable-http-client-reqwest`, `schemars`, `macros`) · `reqwest` (rustls-tls) · `tokio` · `thiserror` · `async-trait` · `schemars`.

**References:**
- Spec (decision trajectory): [`2026-05-21-sprint-4-mcp-sync-tools-design.md`](../specs/2026-05-21-sprint-4-mcp-sync-tools-design.md)
- ADR (durable contract): [`ADR-0018`](../../adr/0018-mcp-integration.md)
- Codex `rmcp-client` (Apache-2.0; **architecture inspiration only, no code copy**): `/home/SENSETIME/qiannengsheng/whoami/workspaces/agents/codex/codex-rs/rmcp-client/`

**Branch:** `feature/sprint-4-mcp-sync-tools` (already created from `chore/sprint-4-baseline-prep` which holds the ROADMAP renumber + send_user→submit_user_text rename. PR #12 covers the baseline; this plan executes on top of it.)

**Commit cadence:** One commit per task; tests + implementation land together in the same commit. Run `just fmt && just fix <crate>` before each commit; verify `just test <crate>` green. Run `just ci` as a gate before the final docs task.

---

## File Structure

```text
crates/cogito-mcp/                         # was stub; this plan fills it out
├── Cargo.toml                             # add rmcp, reqwest, schemars deps
├── src/
│   ├── lib.rs                             # module mounts + Codex attribution header
│   ├── config.rs                          # McpServerConfig + McpTransportConfig (tagged enum)
│   ├── error.rs                           # McpError + McpStartupFailure (6 variants)
│   ├── naming.rs                          # qualify_tool_name + sanitize + split + 64-cap
│   ├── result_mapping.rs                  # rmcp CallToolResult → cogito ToolResult
│   ├── handler.rs                         # MinimalClientHandler (no-op + tracing log forwarder)
│   ├── transport.rs                       # build_stdio + build_streamable_http
│   ├── client.rs                          # McpServerHandle + handshake_and_list
│   ├── provider.rs                        # McpToolProvider (impl ToolProvider)
│   └── factory.rs                         # build_mcp_provider + McpProviderBuildResult
└── tests/
    └── integration.rs                     # 7 scenarios from spec §5.2 table

crates/cogito-config/
├── Cargo.toml                             # add cogito-mcp dep (value types only)
└── src/
    ├── types.rs                           # mcp_servers: Vec<toml::Value> in partial; split into successes + parse failures in RuntimeConfig
    └── merge.rs                           # extend merge to cover mcp_servers (array-replace)

crates/cogito-cli/
├── Cargo.toml                             # add cogito-mcp dep
└── src/
    ├── banner.rs                          # NEW: print_startup_banner (§3.5.3)
    ├── chat.rs                            # wire build_mcp_provider into Composite
    └── chat_config.rs                     # (untouched — MCP doesn't add CLI flags)

crates/cogito-core/src/harness/
└── tool_surface.rs (or equivalent)        # add tracing emit (§4.5.1): mcp.tool_count, mcp.tool_desc_total_bytes, builtin.tool_count

Cargo.toml (workspace root)                # add rmcp to [workspace.dependencies]

# Docs (last task)
README.md                                  # status line already done in baseline PR; add an MCP "Quick start" subsection
docs/configuration/overview.md             # §4.5.2 three doc snippets (verbose descs / args semantics / failure behavior)
docs/components/H05-tool-surface.md        # footnote: tracing fields emitted each turn
docs/components/H07-tool-resolver.md       # footnote: MCP schemas trust-and-forward (no boundary check)
CHANGELOG.md                               # Sprint 4 entry under [Unreleased]
```

**Why each file:**

- `cogito-mcp/src/{config,error}.rs` — pure data + error types; depended on by `cogito-config`.
- `cogito-mcp/src/naming.rs` — qualifier algorithm (`mcp__<server>__<tool>`); pure, table-test-friendly.
- `cogito-mcp/src/result_mapping.rs` — boundary mapper, pure function of input; table-test-friendly.
- `cogito-mcp/src/handler.rs` — minimal `ClientHandler` impl required by rmcp; just logs server-side notifications. We don't accept elicitation in v0.1.
- `cogito-mcp/src/transport.rs` — splits the "how to build transport" concern from "how to talk to a server"; lets us unit-test transport assembly without I/O.
- `cogito-mcp/src/client.rs` — `McpServerHandle` owns the running `rmcp::service::RunningService` + per-server timeouts; one per configured server.
- `cogito-mcp/src/provider.rs` — `McpToolProvider` aggregates handles + descriptors; implements `ToolProvider`.
- `cogito-mcp/src/factory.rs` — the soft-skip surface; returns `McpProviderBuildResult` (not `Result`).
- `cogito-cli/src/banner.rs` — separated so the banner format is testable in isolation.

---

## Task 0: Pre-flight

**Goal:** Confirm working tree starts clean on `feature/sprint-4-mcp-sync-tools`, baseline PR is mergeable, and rmcp 1.5 builds in the workspace.

**Files:** none (verification only).

- [ ] **Step 1: Verify branch + tree**

Run:
```bash
git status && git log --oneline main..HEAD
```
Expected: clean tree on `feature/sprint-4-mcp-sync-tools`; HEAD shows the 6 baseline-prep + spec + ADR commits.

- [ ] **Step 2: Verify CI was green at last commit**

Run:
```bash
just fmt-check && just clippy
```
Expected: both pass (the existing tree is clean from spec/ADR commits — no Rust code touched yet).

---

## Task 1: Workspace deps + `cogito-mcp` Cargo.toml + lib.rs skeleton

**Goal:** Add `rmcp` to workspace deps; rewrite `cogito-mcp/Cargo.toml` with the real dependency set; replace the stub `lib.rs` with module mounts and the Codex attribution header.

**Files:**
- Modify: `Cargo.toml` (workspace root) — add `rmcp` to `[workspace.dependencies]`
- Modify: `crates/cogito-mcp/Cargo.toml` — replace stub with full dep list
- Modify: `crates/cogito-mcp/src/lib.rs` — replace stub with module mounts + attribution

- [ ] **Step 1: Add `rmcp` to workspace dependencies**

In `Cargo.toml` (workspace root), inside `[workspace.dependencies]`, add:

```toml
rmcp = { version = "1.5", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client-reqwest",
    "schemars",
    "macros",
] }
```

If `reqwest` and `schemars` are not already in `[workspace.dependencies]`, also add:
```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
# schemars likely already present from cogito-config; if not:
schemars = "0.8"
```

- [ ] **Step 2: Rewrite `crates/cogito-mcp/Cargo.toml`**

Replace contents with:

```toml
[package]
name = "cogito-mcp"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
cogito-protocol.workspace = true

async-trait.workspace = true
rmcp.workspace = true
reqwest.workspace = true
schemars.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
toml = "0.8"
tracing.workspace = true

[dev-dependencies]
# Integration tests spin up an in-process rmcp server; we need the
# server feature for that. Not enabled in regular builds — keeps the
# crate small.
rmcp = { workspace = true, features = ["server"] }
tempfile = { workspace = true }
tokio = { workspace = true, features = ["test-util", "macros"] }

[lints]
workspace = true
```

- [ ] **Step 3: Replace `crates/cogito-mcp/src/lib.rs`**

Write:

```rust
//! cogito-mcp — MCP client + `ToolProvider` adapter.
//!
//! Architecture-inspired by `openai/codex` `codex-rs/rmcp-client/`
//! (Apache-2.0, pattern-only reimplementation; no source-code lift).
//! Upstream protocol SDK: `rmcp` 1.5
//! (`modelcontextprotocol/rust-sdk`, Apache-2.0) — used as a normal
//! Cargo dependency.
//!
//! See `docs/adr/0018-mcp-integration.md` for the architectural
//! contract and `docs/superpowers/specs/2026-05-21-sprint-4-mcp-
//! sync-tools-design.md` for the decision trajectory.

#![warn(clippy::pedantic)]

pub mod config;
pub mod error;
pub mod factory;
pub mod naming;
pub mod provider;
pub mod result_mapping;

// Internal modules — not part of the public surface.
mod client;
mod handler;
mod transport;

pub use config::{McpServerConfig, McpTransportConfig};
pub use error::{McpError, McpStartupFailure};
pub use factory::{McpProviderBuildResult, build_mcp_provider};
pub use provider::McpToolProvider;
```

- [ ] **Step 4: Run cargo check**

Run:
```bash
cargo check -p cogito-mcp 2>&1 | tail -20
```
Expected: errors complaining about missing modules (`config`, `error`, etc.). That's OK — we'll create them in subsequent tasks. We just want to confirm the dep graph resolves.

If there is a dependency-resolution error (e.g., rmcp 1.5 not found), fix it before proceeding.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/cogito-mcp/Cargo.toml crates/cogito-mcp/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(mcp): crate skeleton with rmcp 1.5 deps + Codex attribution

Replace the v0.2 stub cogito-mcp crate with the full dependency set
required for Sprint 4: rmcp 1.5 (client + child-process +
streamable-http-reqwest + schemars + macros), reqwest with rustls,
thiserror, and the standard serde stack.

lib.rs declares the public module surface (config / error / factory /
naming / provider / result_mapping) plus three internal modules
(client / handler / transport) that subsequent commits will fill in.
Header comment credits the Codex `rmcp-client` Apache-2.0 source
for architectural inspiration without code copy.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `McpServerConfig` + `McpTransportConfig`

**Goal:** Implement the config value types (tagged enum on `transport` field) with serde round-trip tests.

**Files:**
- Create: `crates/cogito-mcp/src/config.rs`

- [ ] **Step 1: Write `config.rs` with types and inline tests**

Create `crates/cogito-mcp/src/config.rs`:

```rust
//! Configuration value types for MCP server entries.
//!
//! See ADR-0018 §2 for transport scope and §3 for failure-mode
//! implications, and `docs/configuration/overview.md` §"MCP servers"
//! for the human-facing reference.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One MCP server entry in the `[[mcp_servers]]` config array.
///
/// `transport` is a tagged enum (`transport = "stdio" | "streamable_http"`)
/// dispatched at server build time. `name` must be globally unique
/// within the array — duplicates land as
/// [`crate::error::McpStartupFailure::DuplicateName`] (ADR-0018 §3).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    /// Server identifier; appears in `mcp__<name>__<tool>` qualified
    /// names and in the startup banner.
    pub name: String,

    /// Transport-specific fields (stdio command/args or HTTP url/auth).
    #[serde(flatten)]
    pub transport: McpTransportConfig,

    /// Startup timeout in seconds (handshake + initial `tools/list`).
    /// Defaults to 10 seconds at the call site when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_sec: Option<f64>,

    /// Per-call timeout in seconds for `tools/call`. Defaults to
    /// 60 seconds at the call site when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_sec: Option<f64>,

    /// Allowlist of raw (server-internal) tool names. When set, only
    /// these tools are registered with cogito. Names are matched
    /// against the server-side raw name, NOT the qualified
    /// `mcp__<server>__<tool>` form — users write configs against
    /// what the server reports, not after sanitization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,

    /// Denylist of raw (server-internal) tool names. Applied after
    /// `enabled_tools`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<Vec<String>>,
}

/// Transport-specific configuration.
///
/// Marked `#[non_exhaustive]` so future transports (e.g. WebSocket if
/// MCP ever standardizes one) can land additively without breaking
/// downstream `match` arms.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case")]
#[non_exhaustive]
pub enum McpTransportConfig {
    /// stdio child process. See MCP spec
    /// <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#stdio>.
    Stdio {
        /// Command to execute; resolved via `PATH` if not absolute.
        command: String,
        /// Arguments passed verbatim to the child. cogito does **not**
        /// expand `~`, `$VAR`, or normalize relative paths — see
        /// `docs/configuration/overview.md` for the rationale.
        #[serde(default)]
        args: Vec<String>,
        /// Explicit environment variables to inject (in addition to
        /// the cogito process environment).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    /// streamable-HTTP endpoint. See MCP spec
    /// <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#streamable-http>.
    StreamableHttp {
        /// HTTP(S) endpoint URL.
        url: String,
        /// Name of the env var holding the bearer token. The literal
        /// token must NOT appear in the config (ADR-0018 §2). When
        /// omitted, no `Authorization` header is sent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        /// Static HTTP headers to attach to every request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_headers: Option<HashMap<String, String>>,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn stdio_variant_round_trips() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "filesystem"
            transport = "stdio"
            command = "uvx"
            args = ["mcp-server-filesystem", "/tmp"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.name, "filesystem");
        match cfg.transport {
            McpTransportConfig::Stdio { command, args, env: _ } => {
                assert_eq!(command, "uvx");
                assert_eq!(args, vec!["mcp-server-filesystem", "/tmp"]);
            }
            McpTransportConfig::StreamableHttp { .. } => panic!("wrong variant"),
        }
    }

    #[test]
    fn streamable_http_variant_round_trips() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "company_api"
            transport = "streamable_http"
            url = "https://mcp.example.com/v1"
            bearer_token_env_var = "COMPANY_MCP_TOKEN"
            "#,
        )
        .unwrap();
        match cfg.transport {
            McpTransportConfig::StreamableHttp { url, bearer_token_env_var, .. } => {
                assert_eq!(url, "https://mcp.example.com/v1");
                assert_eq!(bearer_token_env_var.as_deref(), Some("COMPANY_MCP_TOKEN"));
            }
            McpTransportConfig::Stdio { .. } => panic!("wrong variant"),
        }
    }

    #[test]
    fn literal_bearer_token_field_is_rejected() {
        let err = toml::from_str::<McpServerConfig>(
            r#"
            name = "leaky"
            transport = "streamable_http"
            url = "https://x.example.com"
            bearer_token = "this-should-not-be-here"
            "#,
        )
        .expect_err("must reject literal bearer_token field");
        let msg = err.to_string();
        assert!(
            msg.contains("bearer_token") || msg.contains("unknown"),
            "error should mention unknown bearer_token field: {msg}"
        );
    }

    #[test]
    fn missing_transport_field_errors() {
        let err = toml::from_str::<McpServerConfig>(
            r#"
            name = "incomplete"
            command = "x"
            "#,
        )
        .expect_err("must reject entry without transport tag");
        let msg = err.to_string();
        assert!(
            msg.contains("transport") || msg.contains("missing"),
            "error should mention missing transport: {msg}"
        );
    }

    #[test]
    fn enabled_disabled_tools_are_optional() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "minimal"
            transport = "stdio"
            command = "echo"
            "#,
        )
        .unwrap();
        assert!(cfg.enabled_tools.is_none());
        assert!(cfg.disabled_tools.is_none());
        assert!(cfg.startup_timeout_sec.is_none());
        assert!(cfg.tool_timeout_sec.is_none());
    }

    #[test]
    fn timeouts_round_trip_as_floats() {
        let cfg: McpServerConfig = toml::from_str(
            r#"
            name = "with_timeouts"
            transport = "stdio"
            command = "x"
            startup_timeout_sec = 15.5
            tool_timeout_sec = 30
            "#,
        )
        .unwrap();
        assert!((cfg.startup_timeout_sec.unwrap() - 15.5).abs() < 1e-9);
        assert!((cfg.tool_timeout_sec.unwrap() - 30.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Verify tests fail (modules not all created yet, but config tests can pass standalone)**

Run:
```bash
cargo test -p cogito-mcp --lib config:: 2>&1 | tail -20
```
Expected: build fails because `error.rs`, `naming.rs`, etc. are referenced by `lib.rs` but don't exist. To check just the config module without that interference, the iteration trick is to comment out the missing module mounts in `lib.rs` temporarily — OR proceed and check after Task 3 lands.

For speed: skip running tests here; the inline tests pass mechanically once the file compiles in the full crate. Move on to Task 3, then verify both tests together at the end of Task 3.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-mcp/src/config.rs
git commit -m "$(cat <<'EOF'
feat(mcp): McpServerConfig + McpTransportConfig with 6 round-trip tests

Per ADR-0018 §2 and spec §Q8: tagged enum on `transport` field (stdio
| streamable_http), `deny_unknown_fields` on McpServerConfig to catch
typos, literal `bearer_token` field explicitly rejected (secrets live
in env vars per ADR-0017 §6 + ADR-0018 §2). enabled_tools /
disabled_tools / timeouts all optional with safe defaults applied at
call sites.

Inline tests cover: stdio round-trip, streamable_http round-trip,
literal bearer_token rejection, missing-transport-tag rejection,
optional fields default to None, timeout float parsing.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `McpError` + `McpStartupFailure`

**Goal:** Implement the unified `McpStartupFailure` channel (6 variants, `#[non_exhaustive]`) and the small `McpError` enum for library-internal short-circuits.

**Files:**
- Create: `crates/cogito-mcp/src/error.rs`

- [ ] **Step 1: Write `error.rs`**

Create `crates/cogito-mcp/src/error.rs`:

```rust
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
                McpStartupFailure::ConfigParse { index: 2, error: "boom".into() },
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
                McpStartupFailure::DuplicateName { name: "d".into(), index: 3 },
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
```

- [ ] **Step 2: Verify tests compile and pass**

Run:
```bash
cargo test -p cogito-mcp --lib error:: 2>&1 | tail -20
```
Expected: build will still fail because other modules referenced by `lib.rs` don't exist yet. To check just config + error:

Edit `crates/cogito-mcp/src/lib.rs` temporarily — comment out missing modules:

```rust
pub mod config;
pub mod error;
// pub mod factory;
// pub mod naming;
// pub mod provider;
// pub mod result_mapping;

// mod client;
// mod handler;
// mod transport;

pub use config::{McpServerConfig, McpTransportConfig};
pub use error::{McpError, McpStartupFailure};
// pub use factory::{McpProviderBuildResult, build_mcp_provider};
// pub use provider::McpToolProvider;
```

Run:
```bash
cargo test -p cogito-mcp 2>&1 | tail -10
```
Expected: 10 tests pass (6 in config + 4 in error). Restore the commented lines after — but wait, leave them commented for now; we'll uncomment as each module lands.

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-mcp/src/error.rs crates/cogito-mcp/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(mcp): McpStartupFailure unified channel + McpError

Per ADR-0018 §3: McpStartupFailure is the load-bearing failure
channel for the MCP-non-fatal-to-Runtime principle. 6 variants
(ConfigParse / BearerEnvMissing / DuplicateName / StartupTimeout /
TransportError / HandshakeFailed) cover the full per-server pipeline
from config deserialization through handshake. `#[non_exhaustive]`
keeps future variants additive (e.g. SchemaInvalid).

McpError is reserved for library-internal short-circuits (invariant
violations during dev/test); production MCP failures never propagate
as Result::Err. `server_name()` helper returns the affected server
name for the startup banner.

Tests assert the exact Display phrasing (banner format depends on
it) and that Display does not leak a bearer-token-shaped value.

lib.rs temporarily comments out modules not yet implemented; they'll
be uncommented as each lands.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `naming::qualify` + `split` + sanitize + 64-cap

**Goal:** Implement the tool-name qualifier algorithm and its inverse, with table-driven tests covering every edge case from ADR-0018 §4.

**Files:**
- Create: `crates/cogito-mcp/src/naming.rs`

- [ ] **Step 1: Write `naming.rs`**

Create `crates/cogito-mcp/src/naming.rs`:

```rust
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
/// Constrained by OpenAI Responses API tool-name regex
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
        // Both chars of "工具" → "__" → effectively zero-width info, but stable
        assert_eq!(q, "mcp__server______");
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
```

- [ ] **Step 2: Add `sha1` to crate deps**

`rmcp` already brings `sha1` transitively via its feature set, but we want it explicit. In `crates/cogito-mcp/Cargo.toml`, add to `[dependencies]`:
```toml
sha1 = "0.10"
```

(If `sha1` is in workspace deps already from another crate, use `sha1.workspace = true` instead.)

- [ ] **Step 3: Uncomment the naming module in `lib.rs`**

Edit `crates/cogito-mcp/src/lib.rs`:
```rust
pub mod config;
pub mod error;
// pub mod factory;
pub mod naming;
// pub mod provider;
// pub mod result_mapping;

// mod client;
// mod handler;
// mod transport;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p cogito-mcp --lib naming:: 2>&1 | tail -20
```
Expected: 13 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-mcp/Cargo.toml crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/naming.rs
git commit -m "$(cat <<'EOF'
feat(mcp): qualified-name encoding mcp__<server>__<tool> + tests

Per ADR-0018 §4: tool names sanitize disallowed characters to `_`,
cap at 64 chars with a deterministic SHA-1 suffix when truncated.
13 table-driven tests cover plain ASCII, dot/slash/space, Unicode,
empty tool names, length cap behavior, determinism, collision
observability for provider-layer dedup, split rejection of non-MCP
prefixes, and tool names containing internal `__`.

The pattern is the de facto MCP multi-server convention, also used
by openai/codex; reimplemented in our idioms (sha1 crate dep added
explicitly).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `result_mapping::to_cogito_result`

**Goal:** Implement the `rmcp::model::CallToolResult` → `cogito_protocol::tool::ToolResult` mapping per spec §Q7 + ADR-0018 §5.

**Files:**
- Create: `crates/cogito-mcp/src/result_mapping.rs`

- [ ] **Step 1: Write `result_mapping.rs`**

Create `crates/cogito-mcp/src/result_mapping.rs`:

```rust
//! Map an `rmcp::model::CallToolResult` to cogito's `ToolResult`.
//!
//! See ADR-0018 §5 for the mapping table. v0.1 collapses image /
//! resource content blocks into JSON objects; the multimodal upgrade
//! (ADR-0009 in v0.2) will swap `Output(Vec<serde_json::Value>)`
//! for `Output(Vec<ContentBlock>)` and unblock visual model awareness.

use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use rmcp::model::{CallToolResult, Content, RawContent};
use serde_json::{Value, json};

/// Convert an rmcp `CallToolResult` into a cogito `ToolResult`.
///
/// - `is_error: true` → [`ToolResult::Error`] with
///   [`ToolErrorKind::InvocationFailed`] (conservative
///   `retryable: false`; we don't know the server's state).
/// - `is_error: false` → [`ToolResult::Output`] with one JSON value
///   per content block. Text blocks become JSON strings; image /
///   resource blocks become tagged JSON objects (`{"kind": "image",
///   ...}` etc.) for v0.1; v0.2 multimodal upgrade will preserve
///   them as native `ContentBlock`s.
/// - When `structured_content` is present (non-error case), append a
///   `{"kind": "structured", "data": ...}` element.
#[must_use]
pub fn to_cogito_result(call: CallToolResult) -> ToolResult {
    let is_error = call.is_error.unwrap_or(false);

    if is_error {
        let message = join_text_blocks(&call.content);
        return ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            message: if message.is_empty() {
                "MCP server returned is_error=true with no message".into()
            } else {
                message
            },
            retryable: false,
        };
    }

    let mut output: Vec<Value> = call
        .content
        .into_iter()
        .map(content_block_to_json)
        .collect();

    if let Some(structured) = call.structured_content {
        output.push(json!({
            "kind": "structured",
            "data": structured,
        }));
    }

    ToolResult::Output(output)
}

fn join_text_blocks(blocks: &[Content]) -> String {
    let mut out = String::new();
    for block in blocks {
        if let RawContent::Text(t) = &block.raw {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t.text);
        }
    }
    out
}

fn content_block_to_json(block: Content) -> Value {
    match block.raw {
        RawContent::Text(t) => Value::String(t.text),
        RawContent::Image(i) => json!({
            "kind": "image",
            "mime_type": i.mime_type,
            "data": i.data,
        }),
        RawContent::Resource(r) => json!({
            "kind": "resource",
            "resource": serde_json::to_value(r).unwrap_or(Value::Null),
        }),
        RawContent::Audio(a) => json!({
            "kind": "audio",
            "mime_type": a.mime_type,
            "data": a.data,
        }),
        // Future rmcp variants land here; emit a tagged unknown so
        // future-tool authors can spot the variant.
        other => json!({
            "kind": "unknown",
            "debug": format!("{other:?}"),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rmcp::model::{RawTextContent, RawImageContent};

    fn text_block(s: &str) -> Content {
        Content {
            raw: RawContent::Text(RawTextContent {
                text: s.to_string(),
            }),
            annotations: None,
        }
    }

    #[test]
    fn is_error_true_maps_to_invocation_failed() {
        let call = CallToolResult {
            content: vec![text_block("boom")],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        };
        match to_cogito_result(call) {
            ToolResult::Error { kind, message, retryable } => {
                assert!(matches!(kind, ToolErrorKind::InvocationFailed));
                assert_eq!(message, "boom");
                assert!(!retryable);
            }
            ToolResult::Output(_) => panic!("expected Error"),
        }
    }

    #[test]
    fn is_error_true_with_empty_content_yields_default_message() {
        let call = CallToolResult {
            content: vec![],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        };
        let result = to_cogito_result(call);
        let ToolResult::Error { message, .. } = result else {
            panic!("expected Error");
        };
        assert!(message.contains("no message"));
    }

    #[test]
    fn multi_text_blocks_concatenate_with_newline_for_error() {
        let call = CallToolResult {
            content: vec![text_block("line one"), text_block("line two")],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        };
        let ToolResult::Error { message, .. } = to_cogito_result(call) else {
            panic!("expected Error");
        };
        assert_eq!(message, "line one\nline two");
    }

    #[test]
    fn single_text_output_maps_to_output_with_one_string() {
        let call = CallToolResult {
            content: vec![text_block("hello world")],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        };
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], Value::String("hello world".into()));
    }

    #[test]
    fn missing_is_error_defaults_to_success() {
        let call = CallToolResult {
            content: vec![text_block("ok")],
            is_error: None,
            structured_content: None,
            meta: None,
        };
        let ToolResult::Output(_) = to_cogito_result(call) else {
            panic!("expected Output, got Error");
        };
    }

    #[test]
    fn image_block_serializes_to_tagged_json_object() {
        let img = Content {
            raw: RawContent::Image(RawImageContent {
                mime_type: "image/png".into(),
                data: "BASE64DATA".into(),
            }),
            annotations: None,
        };
        let call = CallToolResult {
            content: vec![img],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        };
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 1);
        let obj = &v[0];
        assert_eq!(obj["kind"], "image");
        assert_eq!(obj["mime_type"], "image/png");
        assert_eq!(obj["data"], "BASE64DATA");
    }

    #[test]
    fn structured_content_appends_extra_element() {
        let call = CallToolResult {
            content: vec![text_block("hi")],
            is_error: Some(false),
            structured_content: Some(json!({"count": 3})),
            meta: None,
        };
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Value::String("hi".into()));
        assert_eq!(v[1]["kind"], "structured");
        assert_eq!(v[1]["data"]["count"], 3);
    }
}
```

- [ ] **Step 2: Uncomment the result_mapping module in `lib.rs`**

```rust
pub mod config;
pub mod error;
// pub mod factory;
pub mod naming;
// pub mod provider;
pub mod result_mapping;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-mcp --lib result_mapping:: 2>&1 | tail -20
```
Expected: 7 tests pass.

If any rmcp type field shape differs from what's written (rmcp 1.5 may evolve), inspect with `cargo doc -p rmcp --open` or rg the rmcp source under `~/.cargo/registry/src/index.crates.io-*/rmcp-1.5*/src/model/` and adjust the test fixture builders accordingly.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/result_mapping.rs
git commit -m "$(cat <<'EOF'
feat(mcp): map rmcp CallToolResult to cogito ToolResult

Per ADR-0018 §5 + spec §Q7: is_error=true → ToolResult::Error
(InvocationFailed kind, retryable=false conservative default),
is_error=false → ToolResult::Output with one JSON value per content
block (text → string; image/resource/audio → tagged JSON object).
structured_content appends a {"kind":"structured","data":...} entry.

Image/resource/audio serialization to JSON objects is a v0.1
compromise — the multimodal upgrade in ADR-0009 (v0.2) will swap
Output(Vec<Value>) for Output(Vec<ContentBlock>) and let models see
images natively.

7 inline tests cover the §Q7 mapping table + structured_content
append + missing is_error default.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Minimal `ClientHandler` (no-op + log forward)

**Goal:** Implement the rmcp `ClientHandler` trait with all defaults — server-initiated requests (elicitation) are rejected; notifications (logging, progress, cancellation, resource_updated) get forwarded to `tracing`.

**Files:**
- Create: `crates/cogito-mcp/src/handler.rs`

- [ ] **Step 1: Write `handler.rs`**

Create `crates/cogito-mcp/src/handler.rs`:

```rust
//! Minimal `rmcp::ClientHandler` impl.
//!
//! rmcp requires a `ClientHandler` to spawn a service; we don't need
//! the full surface (no elicitation UI in v0.1), so this is a thin
//! shell that:
//!
//! - Rejects elicitation requests (server tries to ask the user via
//!   client) with `rmcp::ErrorData::method_not_found`.
//! - Forwards logging / progress / cancellation / resource_updated
//!   notifications to `tracing`.
//!
//! See ADR-0018 §"Out of scope" for elicitation rationale.

use rmcp::ClientHandler;
use rmcp::RoleClient;
use rmcp::model::{
    CancelledNotificationParam, ClientInfo, CreateElicitationRequestParam,
    CreateElicitationResult, LoggingMessageNotificationParam,
    ProgressNotificationParam, ResourceUpdatedNotificationParam,
};
use rmcp::service::{NotificationContext, RequestContext};
use tracing::{debug, info, warn};

/// Identifies the server this handler belongs to, so trace fields
/// can group messages by origin.
#[derive(Clone)]
pub(crate) struct MinimalClientHandler {
    server_name: String,
    client_info: ClientInfo,
}

impl MinimalClientHandler {
    pub(crate) fn new(server_name: String, client_info: ClientInfo) -> Self {
        Self { server_name, client_info }
    }
}

impl ClientHandler for MinimalClientHandler {
    fn get_info(&self) -> ClientInfo {
        self.client_info.clone()
    }

    async fn create_elicitation(
        &self,
        _request: CreateElicitationRequestParam,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, rmcp::ErrorData> {
        warn!(
            mcp.server = %self.server_name,
            "MCP server requested elicitation; cogito v0.1 does not support it"
        );
        Err(rmcp::ErrorData::method_not_found::<rmcp::model::CreateElicitationRequestMethod>())
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        // Forward MCP log levels to tracing levels (best-effort mapping).
        let level_str = format!("{:?}", params.level);
        info!(
            mcp.server = %self.server_name,
            mcp.log_level = %level_str,
            mcp.logger = ?params.logger,
            "{}",
            serde_json::to_string(&params.data).unwrap_or_default()
        );
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        debug!(
            mcp.server = %self.server_name,
            mcp.progress_token = ?params.progress_token,
            mcp.progress = params.progress,
            mcp.total = ?params.total,
            "MCP progress notification"
        );
    }

    async fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            mcp.server = %self.server_name,
            mcp.request_id = %params.request_id,
            mcp.reason = ?params.reason,
            "MCP server cancelled a request"
        );
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            mcp.server = %self.server_name,
            mcp.resource_uri = %params.uri,
            "MCP server reported resource updated"
        );
    }
}
```

- [ ] **Step 2: Uncomment in `lib.rs`**

```rust
// (other modules unchanged)
mod handler;
```

- [ ] **Step 3: Verify compiles**

```bash
cargo check -p cogito-mcp 2>&1 | tail -10
```
Expected: clean compile (no errors). If rmcp's `LoggingMessageNotificationParam` shape differs from what's written here in rmcp 1.5 specifically, adjust field accesses. The `rmcp::model::LoggingLevel` enum's `Debug` impl is used for the level string.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/handler.rs
git commit -m "$(cat <<'EOF'
feat(mcp): minimal ClientHandler routes notifications to tracing

rmcp::service::serve_client requires a ClientHandler impl. v0.1
provides MinimalClientHandler that:

- Rejects elicitation requests (server-to-client UI) with
  method_not_found; cogito doesn't have a UI affordance for it.
- Forwards logging / progress / cancellation / resource_updated
  notifications to tracing with mcp.server / mcp.log_level / etc.
  structured fields for ops grep-ability.

OAuth and elicitation are out of v0.1 scope per ADR-0018; this
handler is the smallest surface that lets us spawn a service.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `transport::build_stdio` + `transport::build_streamable_http`

**Goal:** Implement the two transport-builders. Each returns either a constructed transport (ready to feed into `serve_client`) or a `McpStartupFailure` describing why it couldn't build. No I/O at the rmcp protocol level happens here yet — that's the handshake (Task 8).

**Files:**
- Create: `crates/cogito-mcp/src/transport.rs`

- [ ] **Step 1: Write `transport.rs`**

Create `crates/cogito-mcp/src/transport.rs`:

```rust
//! Build per-server transports (stdio child process or streamable-HTTP
//! client). Each builder returns either a ready transport or an
//! `McpStartupFailure` recording why the build failed.

use std::process::Stdio;

use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::McpTransportConfig;
use crate::error::McpStartupFailure;

/// Discriminated transport ready to be handed to `rmcp::service::serve_client`.
pub(crate) enum BuiltTransport {
    ChildProcess(TokioChildProcess),
    StreamableHttp(StreamableHttpClientTransport<reqwest::Client>),
}

/// Build a transport from a [`McpTransportConfig`]. The `server_name`
/// is used only to annotate failures.
pub(crate) fn build_transport(
    server_name: &str,
    cfg: &McpTransportConfig,
) -> Result<BuiltTransport, McpStartupFailure> {
    match cfg {
        McpTransportConfig::Stdio { command, args, env } => {
            build_stdio(server_name, command, args, env.as_ref())
        }
        McpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
        } => build_streamable_http(
            server_name,
            url,
            bearer_token_env_var.as_deref(),
            http_headers.as_ref(),
        ),
    }
}

fn build_stdio(
    server_name: &str,
    command: &str,
    args: &[String],
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<BuiltTransport, McpStartupFailure> {
    let mut cmd = Command::new(command);
    cmd.kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(args);
    if let Some(env_map) = env {
        for (k, v) in env_map {
            cmd.env(k, v);
        }
    }

    let (transport, stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| McpStartupFailure::TransportError {
            name: server_name.to_string(),
            error: format!("spawn `{command}`: {e}"),
        })?;

    if let Some(stderr) = stderr {
        let name = server_name.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        info!(mcp.server = %name, "stderr: {line}");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        warn!(mcp.server = %name, "stderr read failed: {err}");
                        break;
                    }
                }
            }
        });
    }

    Ok(BuiltTransport::ChildProcess(transport))
}

fn build_streamable_http(
    server_name: &str,
    url: &str,
    bearer_token_env_var: Option<&str>,
    http_headers: Option<&std::collections::HashMap<String, String>>,
) -> Result<BuiltTransport, McpStartupFailure> {
    // Resolve bearer token from env. Missing env → soft fail.
    let bearer = if let Some(env_var) = bearer_token_env_var {
        match std::env::var(env_var) {
            Ok(v) if !v.trim().is_empty() => Some(v),
            _ => {
                return Err(McpStartupFailure::BearerEnvMissing {
                    name: server_name.to_string(),
                    env_var: env_var.to_string(),
                });
            }
        }
    } else {
        None
    };

    let mut builder = reqwest::Client::builder();
    if let Some(headers) = http_headers {
        let mut hm = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            let name = match reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                Ok(n) => n,
                Err(e) => {
                    return Err(McpStartupFailure::TransportError {
                        name: server_name.to_string(),
                        error: format!("invalid header name `{k}`: {e}"),
                    });
                }
            };
            let value = match reqwest::header::HeaderValue::from_str(v) {
                Ok(val) => val,
                Err(e) => {
                    return Err(McpStartupFailure::TransportError {
                        name: server_name.to_string(),
                        // Note: do NOT echo `v` — it might contain a secret if
                        // the user wired a sensitive value through static
                        // headers. Echo only the key.
                        error: format!("invalid header value for `{k}`: {e}"),
                    });
                }
            };
            hm.insert(name, value);
        }
        builder = builder.default_headers(hm);
    }
    let http_client = builder
        .build()
        .map_err(|e| McpStartupFailure::TransportError {
            name: server_name.to_string(),
            error: format!("build reqwest client: {e}"),
        })?;

    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(token) = bearer {
        config = config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::with_client(http_client, config);
    Ok(BuiltTransport::StreamableHttp(transport))
}
```

- [ ] **Step 2: Uncomment in `lib.rs`**

```rust
mod handler;
mod transport;
```

- [ ] **Step 3: Verify compiles**

```bash
cargo check -p cogito-mcp 2>&1 | tail -10
```
Expected: clean compile. If `StreamableHttpClientTransportConfig::with_uri` / `auth_header` / `with_client` method names differ in rmcp 1.5, adjust per `rg "with_uri\|auth_header\|with_client" ~/.cargo/registry/src/index.crates.io-*/rmcp-1.5*/src/transport/`.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/transport.rs
git commit -m "$(cat <<'EOF'
feat(mcp): build_transport (stdio + streamable-HTTP)

Per ADR-0018 §2: stdio spawns a child process with kill_on_drop and
pipes stderr to tracing (`mcp.server=<name>` structured field).
Streamable-HTTP builds a reqwest client with optional bearer auth
sourced from `bearer_token_env_var`; missing env var becomes
McpStartupFailure::BearerEnvMissing rather than an error propagated
to Runtime (ADR-0018 §3).

Header errors do NOT echo header values into the error message —
prevents accidental secret leak if a user puts a sensitive value in
http_headers.

BuiltTransport enum carries the (typed) outcome through to the
handshake task (Task 8) without losing the variant info needed for
type-safe service::serve_client dispatch.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `client::McpServerHandle` + handshake

**Goal:** Implement the per-server handle (handshake → list tools → store running service) with per-server timeouts and full failure capture into `McpStartupFailure`.

**Files:**
- Create: `crates/cogito-mcp/src/client.rs`

- [ ] **Step 1: Write `client.rs`**

Create `crates/cogito-mcp/src/client.rs`:

```rust
//! Per-server `rmcp` client handle. Owns a running rmcp service and
//! the per-server policy (timeouts, tool filter applied at handshake).

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParam, ClientInfo, Implementation};
use rmcp::service::{self, RoleClient, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::StreamableHttpClientTransport;
use tokio::time;

use crate::config::McpServerConfig;
use crate::error::McpStartupFailure;
use crate::handler::MinimalClientHandler;
use crate::transport::{BuiltTransport, build_transport};

/// Default startup timeout when [`McpServerConfig::startup_timeout_sec`]
/// is omitted.
pub(crate) const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Default tool-call timeout when [`McpServerConfig::tool_timeout_sec`]
/// is omitted.
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// A live MCP server, post-handshake.
pub(crate) struct McpServerHandle {
    pub(crate) server_name: String,
    pub(crate) service: Arc<RunningService<RoleClient, MinimalClientHandler>>,
    pub(crate) tool_timeout: Duration,
}

/// What the handshake produced.
pub(crate) struct HandshakeOutcome {
    pub(crate) handle: McpServerHandle,
    /// Server-internal tools (post enabled/disabled filter), one per
    /// entry the provider should register.
    pub(crate) tools: Vec<rmcp::model::Tool>,
}

fn client_info() -> ClientInfo {
    ClientInfo {
        protocol_version: Default::default(),
        capabilities: Default::default(),
        client_info: Implementation {
            name: "cogito".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    }
}

/// Build transport → spawn rmcp service → call `tools/list`, all
/// wrapped in [`McpServerConfig::startup_timeout_sec`].
///
/// On any failure, returns an [`McpStartupFailure`] (NOT a normal
/// `Result::Err`) — the caller (factory) collects these and proceeds.
pub(crate) async fn handshake_and_list(
    cfg: &McpServerConfig,
) -> Result<HandshakeOutcome, McpStartupFailure> {
    let startup_timeout = cfg
        .startup_timeout_sec
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(DEFAULT_STARTUP_TIMEOUT);

    let tool_timeout = cfg
        .tool_timeout_sec
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(DEFAULT_TOOL_TIMEOUT);

    let transport = build_transport(&cfg.name, &cfg.transport)?;
    let handler = MinimalClientHandler::new(cfg.name.clone(), client_info());

    let serve_fut = match transport {
        BuiltTransport::ChildProcess(t) => {
            wrap_serve_child(handler.clone(), t)
        }
        BuiltTransport::StreamableHttp(t) => {
            wrap_serve_http(handler.clone(), t)
        }
    };

    let service = match time::timeout(startup_timeout, serve_fut).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err(McpStartupFailure::HandshakeFailed {
                name: cfg.name.clone(),
                error: format!("{e}"),
            });
        }
        Err(_) => {
            return Err(McpStartupFailure::StartupTimeout {
                name: cfg.name.clone(),
                timeout_sec: startup_timeout.as_secs_f64(),
            });
        }
    };

    // `tools/list` is part of startup; wrap in remaining time would
    // require Instant arithmetic; v0.1 wraps in the same total timeout.
    let list_result = time::timeout(startup_timeout, service.list_tools(None))
        .await
        .map_err(|_| McpStartupFailure::StartupTimeout {
            name: cfg.name.clone(),
            timeout_sec: startup_timeout.as_secs_f64(),
        })?
        .map_err(|e| McpStartupFailure::HandshakeFailed {
            name: cfg.name.clone(),
            error: format!("tools/list: {e}"),
        })?;

    let raw_tools = list_result.tools;
    let tools = filter_tools(raw_tools, cfg.enabled_tools.as_deref(), cfg.disabled_tools.as_deref());

    Ok(HandshakeOutcome {
        handle: McpServerHandle {
            server_name: cfg.name.clone(),
            service: Arc::new(service),
            tool_timeout,
        },
        tools,
    })
}

/// Apply enabled/disabled filters. enabled first (when set), then
/// disabled removes from the result.
fn filter_tools(
    tools: Vec<rmcp::model::Tool>,
    enabled: Option<&[String]>,
    disabled: Option<&[String]>,
) -> Vec<rmcp::model::Tool> {
    let mut out = if let Some(allow) = enabled {
        let set: std::collections::HashSet<&str> = allow.iter().map(String::as_str).collect();
        tools.into_iter().filter(|t| set.contains(t.name.as_str())).collect()
    } else {
        tools
    };
    if let Some(deny) = disabled {
        let set: std::collections::HashSet<&str> = deny.iter().map(String::as_str).collect();
        out.retain(|t| !set.contains(t.name.as_str()));
    }
    out
}

async fn wrap_serve_child(
    handler: MinimalClientHandler,
    transport: TokioChildProcess,
) -> Result<RunningService<RoleClient, MinimalClientHandler>, rmcp::ServiceError> {
    service::serve_client(handler, transport).await
}

async fn wrap_serve_http(
    handler: MinimalClientHandler,
    transport: StreamableHttpClientTransport<reqwest::Client>,
) -> Result<RunningService<RoleClient, MinimalClientHandler>, rmcp::ServiceError> {
    service::serve_client(handler, transport).await
}

/// Invoke a tool on this server's running service, with per-call
/// timeout = min(handle.tool_timeout, ctx.deadline-remaining).
pub(crate) async fn call_tool(
    handle: &McpServerHandle,
    raw_tool_name: &str,
    args: serde_json::Value,
    ctx: &cogito_protocol::ExecCtx,
) -> Result<rmcp::model::CallToolResult, CallError> {
    let deadline = ctx.deadline;
    let now = std::time::Instant::now();
    let remaining = deadline
        .and_then(|d| d.checked_duration_since(now))
        .unwrap_or(handle.tool_timeout);
    let effective = remaining.min(handle.tool_timeout);

    let args_obj = match args {
        serde_json::Value::Object(map) => Some(map),
        serde_json::Value::Null => None,
        // rmcp expects a JSON object for arguments; anything else is
        // a schema violation upstream of us.
        other => {
            return Err(CallError::Other(format!(
                "expected object arguments, got {}",
                other_kind(&other)
            )));
        }
    };

    let params = CallToolRequestParam {
        name: raw_tool_name.to_string().into(),
        arguments: args_obj,
    };

    tokio::select! {
        _ = ctx.cancel.cancelled() => Err(CallError::Cancelled),
        result = time::timeout(effective, handle.service.call_tool(params)) => {
            match result {
                Ok(Ok(call_result)) => Ok(call_result),
                Ok(Err(e)) => Err(CallError::Other(format!("{e}"))),
                Err(_) => Err(CallError::Timeout(effective)),
            }
        }
    }
}

/// Errors from `call_tool`. Surfaced to the provider; provider maps
/// to `ToolResult::Error` variants per ADR-0018 §5.
#[derive(Debug)]
pub(crate) enum CallError {
    Cancelled,
    Timeout(Duration),
    Other(String),
}

fn other_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
```

- [ ] **Step 2: Uncomment in `lib.rs`**

```rust
mod client;
mod handler;
mod transport;
```

- [ ] **Step 3: Verify compiles**

```bash
cargo check -p cogito-mcp 2>&1 | tail -10
```
Expected: clean. If rmcp's `ServiceError`, `CallToolRequestParam.arguments` shape, or `RunningService::list_tools` signature have diverged in 1.5, adjust accordingly. The conversion `name.into()` assumes `name: Cow<'_, str>` or similar — check via `cargo doc -p rmcp --open` if needed.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/client.rs
git commit -m "$(cat <<'EOF'
feat(mcp): McpServerHandle + handshake_and_list + call_tool

Per ADR-0018 §3: handshake_and_list wraps transport build + rmcp
serve_client + tools/list in a single startup_timeout window. ANY
failure (transport build, handshake RPC, list timeout) becomes an
McpStartupFailure variant — no Result<_, McpError> escape path. Tool
filter (enabled_tools / disabled_tools) is applied at handshake time
so the post-startup catalog is final.

call_tool applies min(handle.tool_timeout, ctx.deadline-remaining)
and races against ctx.cancel via tokio::select!; cancellation drops
the future (rmcp closes the request natively). CallError variants
map cleanly to ToolErrorKind in the provider (Task 9).

DEFAULT_STARTUP_TIMEOUT=10s, DEFAULT_TOOL_TIMEOUT=60s as spec §1.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `McpToolProvider` (impl ToolProvider)

**Goal:** Implement the `ToolProvider` trait: `list()` returns pre-qualified `ToolDescriptor`s; `invoke(name, args, ctx)` routes by qualified name to the owning `McpServerHandle` and maps the rmcp result.

**Files:**
- Create: `crates/cogito-mcp/src/provider.rs`

- [ ] **Step 1: Write `provider.rs`**

Create `crates/cogito-mcp/src/provider.rs`:

```rust
//! `McpToolProvider` aggregates handles from all successfully-started
//! MCP servers and presents their tools as a single `ToolProvider`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use serde_json::Value;
use tracing::warn;

use crate::client::{CallError, McpServerHandle, call_tool};
use crate::naming::qualify;
use crate::result_mapping::to_cogito_result;

/// Routing entry: a qualified tool name maps to `(server_handle,
/// raw_tool_name)`. The raw name is needed because rmcp's `tools/call`
/// uses the server-internal name, not the qualified one.
struct Route {
    handle: Arc<McpServerHandle>,
    raw_name: String,
}

/// `ToolProvider` aggregating zero or more MCP server handles.
///
/// Constructed by [`crate::factory::build_mcp_provider`]; `routes`
/// and `descriptors` are derived from handshake outputs with the
/// `mcp__server__tool` qualifier applied and within-provider dedup.
pub struct McpToolProvider {
    routes: HashMap<String, Route>,
    descriptors: Vec<ToolDescriptor>,
}

impl McpToolProvider {
    /// Build from per-server (handle, raw-tools) pairs. Performs the
    /// qualify + dedup-with-warn step.
    pub(crate) fn from_handshake_outputs(
        outputs: Vec<(Arc<McpServerHandle>, Vec<rmcp::model::Tool>)>,
    ) -> Self {
        let mut routes: HashMap<String, Route> = HashMap::new();
        let mut descriptors: Vec<ToolDescriptor> = Vec::new();

        for (handle, tools) in outputs {
            for tool in tools {
                let qualified = qualify(&handle.server_name, &tool.name);
                if routes.contains_key(&qualified) {
                    warn!(
                        mcp.server = %handle.server_name,
                        mcp.tool = %tool.name,
                        qualified = %qualified,
                        "duplicate qualified tool name; skipping"
                    );
                    continue;
                }
                let descriptor = ToolDescriptor {
                    name: qualified.clone(),
                    description: tool.description.clone().unwrap_or_default().into_owned(),
                    schema: serde_json::to_value(&tool.input_schema).unwrap_or(Value::Null),
                    execution_class: ExecutionClass::AlwaysSync,
                    outputs_model_visible_multimodal: false,
                };
                routes.insert(
                    qualified,
                    Route {
                        handle: Arc::clone(&handle),
                        raw_name: tool.name.clone().into_owned(),
                    },
                );
                descriptors.push(descriptor);
            }
        }

        Self { routes, descriptors }
    }
}

#[async_trait]
impl ToolProvider for McpToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn invoke(&self, name: &str, args: Value, ctx: ExecCtx) -> InvokeOutcome {
        let route = match self.routes.get(name) {
            Some(r) => r,
            None => {
                return InvokeOutcome::Sync(ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("unknown MCP tool: {name}"),
                    retryable: false,
                });
            }
        };

        match call_tool(&route.handle, &route.raw_name, args, &ctx).await {
            Ok(call_result) => InvokeOutcome::Sync(to_cogito_result(call_result)),
            Err(CallError::Cancelled) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::Cancelled,
                message: format!("MCP tool `{name}` cancelled"),
                retryable: false,
            }),
            Err(CallError::Timeout(d)) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::Timeout,
                message: format!(
                    "MCP tool `{name}` timed out after {}s",
                    d.as_secs_f64()
                ),
                retryable: true,
            }),
            Err(CallError::Other(e)) => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("MCP tool `{name}` failed: {e}"),
                retryable: false,
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_tool_returns_invalid_args() {
        let provider = McpToolProvider {
            routes: HashMap::new(),
            descriptors: vec![],
        };
        let ctx = ExecCtx {
            session_id: cogito_protocol::SessionId::new(),
            turn_id: cogito_protocol::TurnId::new(),
            deadline: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let outcome = provider.invoke("mcp__nope__nope", Value::Null, ctx).await;
        let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = outcome else {
            panic!("expected Sync(Error)");
        };
        assert!(matches!(kind, ToolErrorKind::InvalidArgs));
    }
}
```

- [ ] **Step 2: Uncomment in `lib.rs`**

```rust
pub mod provider;

mod client;
mod handler;
mod transport;
```

And uncomment the corresponding `pub use`:
```rust
pub use provider::McpToolProvider;
```

- [ ] **Step 3: Add tokio-util as a dev dep for test**

In `crates/cogito-mcp/Cargo.toml`, ensure `[dev-dependencies]` has:
```toml
tokio-util = { workspace = true }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p cogito-mcp --lib provider:: 2>&1 | tail -10
```
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-mcp/Cargo.toml crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/provider.rs
git commit -m "$(cat <<'EOF'
feat(mcp): McpToolProvider impl ToolProvider

list() returns pre-qualified ToolDescriptors (mcp__<server>__<tool>);
qualify happens at construction-time inside from_handshake_outputs.
Duplicate qualified names trigger a warn-log and skip the later
entry (ADR-0018 §4 dedup convention).

invoke() routes by qualified name to the owning McpServerHandle,
calls the rmcp service via client::call_tool, and maps the outcome:
- Ok(CallToolResult) → result_mapping::to_cogito_result
- CallError::Cancelled → ToolErrorKind::Cancelled
- CallError::Timeout   → ToolErrorKind::Timeout (retryable=true)
- CallError::Other     → ToolErrorKind::InvocationFailed
Unknown qualified name → ToolErrorKind::InvalidArgs.

All MCP tools are ExecutionClass::AlwaysSync per ADR-0018 §1.3
(no async path; Sprint 5 will revisit if MCP grows long-running
tools).

Unit test covers unknown-tool routing.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `build_mcp_provider` + `McpProviderBuildResult`

**Goal:** The compiler-enforced soft-skip surface (ADR-0018 §3). Concurrent handshakes; failures (including duplicate names) accumulate. Returns `McpProviderBuildResult { provider, failures }` — **not** a `Result<_, _>`.

**Files:**
- Create: `crates/cogito-mcp/src/factory.rs`

- [ ] **Step 1: Write `factory.rs`**

Create `crates/cogito-mcp/src/factory.rs`:

```rust
//! `build_mcp_provider` — the soft-skip surface for MCP server startup.
//!
//! Per ADR-0018 §3, this function **never** returns `Result::Err`.
//! It returns [`McpProviderBuildResult`] carrying a (possibly absent)
//! provider and the list of per-server failures. A Surface
//! (`cogito-cli`, future `cogito-tui`, consumer Server) joins these
//! with any parse-time failures from `cogito-config` and surfaces
//! the full list via a startup banner.

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::tool::ToolProvider;
use tokio::task::JoinSet;

use crate::client::handshake_and_list;
use crate::config::McpServerConfig;
use crate::error::McpStartupFailure;
use crate::provider::McpToolProvider;

/// Outcome of [`build_mcp_provider`].
///
/// `provider == None` when no server came up successfully (every
/// entry failed, or the input list was empty after duplicate
/// pruning). `failures` contains every per-server failure, in input
/// order (duplicates first, then handshake failures in JoinSet
/// completion order).
pub struct McpProviderBuildResult {
    /// Composite provider, or `None` if nothing came up.
    pub provider: Option<Arc<dyn ToolProvider>>,
    /// All per-server failures encountered.
    pub failures: Vec<McpStartupFailure>,
}

/// Bring up every configured MCP server concurrently. Servers that
/// fail are recorded; servers that succeed contribute their tools to
/// the returned provider. Runtime **never** sees a `Result::Err`
/// from this function — by design.
pub async fn build_mcp_provider(cfgs: &[McpServerConfig]) -> McpProviderBuildResult {
    if cfgs.is_empty() {
        return McpProviderBuildResult {
            provider: None,
            failures: Vec::new(),
        };
    }

    let mut failures: Vec<McpStartupFailure> = Vec::new();

    // Deduplicate by name up-front; later entries become DuplicateName failures.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut survivors: Vec<&McpServerConfig> = Vec::with_capacity(cfgs.len());
    for (idx, cfg) in cfgs.iter().enumerate() {
        if !seen.insert(&cfg.name) {
            failures.push(McpStartupFailure::DuplicateName {
                name: cfg.name.clone(),
                index: idx,
            });
        } else {
            survivors.push(cfg);
        }
    }

    let mut joinset: JoinSet<Result<crate::client::HandshakeOutcome, McpStartupFailure>> =
        JoinSet::new();
    for cfg in survivors {
        let cfg = cfg.clone();
        joinset.spawn(async move { handshake_and_list(&cfg).await });
    }

    let mut outputs: Vec<(Arc<crate::client::McpServerHandle>, Vec<rmcp::model::Tool>)> =
        Vec::new();

    while let Some(joined) = joinset.join_next().await {
        match joined {
            Ok(Ok(outcome)) => {
                outputs.push((Arc::new(outcome.handle), outcome.tools));
            }
            Ok(Err(failure)) => {
                failures.push(failure);
            }
            Err(join_err) => {
                // Task panicked. Should not happen — handshake_and_list
                // is panic-safe. Surface as a generic transport error
                // so it shows up in the banner instead of being lost.
                failures.push(McpStartupFailure::TransportError {
                    name: "<unknown>".into(),
                    error: format!("task join error: {join_err}"),
                });
            }
        }
    }

    let provider: Option<Arc<dyn ToolProvider>> = if outputs.is_empty() {
        None
    } else {
        Some(Arc::new(McpToolProvider::from_handshake_outputs(outputs)))
    };

    McpProviderBuildResult { provider, failures }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::McpTransportConfig;

    #[tokio::test]
    async fn empty_input_returns_none_provider_and_no_failures() {
        let result = build_mcp_provider(&[]).await;
        assert!(result.provider.is_none());
        assert!(result.failures.is_empty());
    }

    #[tokio::test]
    async fn duplicate_names_record_failures_for_later_entries() {
        let cfgs = vec![
            McpServerConfig {
                name: "x".into(),
                transport: McpTransportConfig::Stdio {
                    command: "/nonexistent/binary".into(),
                    args: vec![],
                    env: None,
                },
                startup_timeout_sec: Some(0.1),
                tool_timeout_sec: None,
                enabled_tools: None,
                disabled_tools: None,
            },
            McpServerConfig {
                name: "x".into(), // duplicate
                transport: McpTransportConfig::Stdio {
                    command: "/nonexistent/binary".into(),
                    args: vec![],
                    env: None,
                },
                startup_timeout_sec: Some(0.1),
                tool_timeout_sec: None,
                enabled_tools: None,
                disabled_tools: None,
            },
        ];
        let result = build_mcp_provider(&cfgs).await;
        // First entry tries to start and fails (binary missing) — TransportError.
        // Second entry hits dedup — DuplicateName.
        let dup_count = result
            .failures
            .iter()
            .filter(|f| matches!(f, McpStartupFailure::DuplicateName { .. }))
            .count();
        assert_eq!(dup_count, 1);
    }

    #[tokio::test]
    async fn all_servers_fail_yields_none_provider_with_failures() {
        let cfgs = vec![McpServerConfig {
            name: "broken".into(),
            transport: McpTransportConfig::Stdio {
                command: "/this/path/does/not/exist".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: Some(0.1),
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        }];
        let result = build_mcp_provider(&cfgs).await;
        // Critical invariant: even when EVERYTHING fails, build returns
        // a value (no Result::Err) — this is the compiler-enforced
        // soft-skip from ADR-0018 §3.
        assert!(result.provider.is_none());
        assert!(!result.failures.is_empty());
    }
}
```

- [ ] **Step 2: Uncomment in `lib.rs`**

```rust
pub mod config;
pub mod error;
pub mod factory;
pub mod naming;
pub mod provider;
pub mod result_mapping;

mod client;
mod handler;
mod transport;

pub use config::{McpServerConfig, McpTransportConfig};
pub use error::{McpError, McpStartupFailure};
pub use factory::{McpProviderBuildResult, build_mcp_provider};
pub use provider::McpToolProvider;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p cogito-mcp 2>&1 | tail -20
```
Expected: all tests pass (config 6 + error 4 + naming 13 + result_mapping 7 + provider 1 + factory 3 = 34 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/src/lib.rs crates/cogito-mcp/src/factory.rs
git commit -m "$(cat <<'EOF'
feat(mcp): build_mcp_provider returns McpProviderBuildResult (no Result)

Compiler-enforced form of ADR-0018 §3 — the soft-skip principle.
build_mcp_provider's return type is McpProviderBuildResult, NOT
Result<_, McpError>: a Surface CANNOT propagate MCP failures via `?`
to abort Runtime construction. Failures collect; provider is `None`
when nothing came up; Runtime builds either way.

Concurrency: JoinSet spawns one task per server config; failures
collect in completion order. Duplicate names are pruned up-front
(DuplicateName failures recorded for later entries); JoinErrors
(task panic) become TransportError so they aren't silently lost.

Tests verify:
- Empty input → None provider, no failures
- Duplicate names → DuplicateName failure for the second entry
- All servers fail → STILL returns a McpProviderBuildResult (the
  load-bearing invariant)

Lib.rs uncomments all modules; the crate is now structurally
complete. Subsequent tasks wire it into cogito-config and cogito-cli.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: `cogito-config` integration — lenient `mcp_servers` parsing

**Goal:** Extend `RuntimeConfigPartial` with `mcp_servers: Option<Vec<toml::Value>>` (raw, untyped at partial layer) and `RuntimeConfig` with `mcp_servers: Vec<McpServerConfig>` + `mcp_parse_failures: Vec<McpStartupFailure>`. Finalize performs per-entry try-deserialize so a typo in one entry doesn't poison the whole TOML parse.

**Files:**
- Modify: `crates/cogito-config/Cargo.toml` (add `cogito-mcp` + `toml`)
- Modify: `crates/cogito-config/src/types.rs` (extend `RuntimeConfigPartial` and `RuntimeConfig`)
- Modify: `crates/cogito-config/src/merge.rs` (extend merge + finalize)

- [ ] **Step 1: Add `cogito-mcp` to cogito-config deps**

In `crates/cogito-config/Cargo.toml`, under `[dependencies]`:
```toml
cogito-mcp.workspace = true
toml = "0.8"
```

Add `cogito-mcp` to `[workspace.dependencies]` in the workspace `Cargo.toml` if it's not already there:
```toml
cogito-mcp = { path = "crates/cogito-mcp" }
```

- [ ] **Step 2: Whitelist `cogito-config → cogito-mcp` in layer check**

Inspect `scripts/check-layer.sh`. If the script rejects this edge, add the allowed-pair entry. (Per ADR-0018 §8, this is a Hand-internal value-type dependency.)

If the script uses an explicit allowlist file or inline list, search for similar entries like `cogito-cli → cogito-mcp` and add `cogito-config → cogito-mcp` next to them. If the check is purely structural and `cogito-config` is a peer of `cogito-tools` / `cogito-model`, no change is needed.

Run:
```bash
bash scripts/check-layer.sh 2>&1 | tail -10
```
Expected: passes. If it fails, fix the script's allowlist.

- [ ] **Step 3: Modify `crates/cogito-config/src/types.rs` — partial type**

Open the file and locate `RuntimeConfigPartial`. Add a field:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfigPartial {
    /// Optional `[runtime]` section contribution.
    pub runtime: Option<RuntimeSectionPartial>,
    /// Optional `[[providers]]` array contribution.
    pub providers: Option<Vec<ProviderConfig>>,
    /// Optional `[[mcp_servers]]` array (Sprint 4). Stored as raw
    /// TOML values so per-entry deserialization can be deferred to
    /// finalize, where a bad entry becomes a McpStartupFailure
    /// instead of poisoning the whole parse. See ADR-0018 §3.
    pub mcp_servers: Option<Vec<toml::Value>>,
}
```

Also locate `RuntimeConfig`. Add two fields:

```rust
pub struct RuntimeConfig {
    pub runtime: RuntimeSection,
    pub providers: Vec<ProviderConfig>,
    pub strategies: HashMap<String, HarnessStrategy>,
    /// Sprint 4 (ADR-0018): successfully-parsed MCP server entries.
    pub mcp_servers: Vec<cogito_mcp::McpServerConfig>,
    /// Sprint 4 (ADR-0018): per-entry deserialization failures.
    /// Surface code joins these with handshake-time failures from
    /// `build_mcp_provider` and surfaces them in the startup banner.
    pub mcp_parse_failures: Vec<cogito_mcp::McpStartupFailure>,
}
```

Add at top of file (if not present):
```rust
use cogito_mcp::{McpServerConfig, McpStartupFailure};
```

- [ ] **Step 4: Add finalize helper for `mcp_servers`**

In `types.rs` (or `merge.rs`, wherever finalize logic lives), add:

```rust
/// Per-entry try-deserialize. Successes go to the typed list;
/// failures become `McpStartupFailure::ConfigParse` carrying the
/// 0-based index and the deserialization error message.
pub(crate) fn finalize_mcp_servers(
    raw: Option<Vec<toml::Value>>,
) -> (Vec<cogito_mcp::McpServerConfig>, Vec<cogito_mcp::McpStartupFailure>) {
    let Some(entries) = raw else {
        return (Vec::new(), Vec::new());
    };
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for (i, value) in entries.into_iter().enumerate() {
        match value.try_into::<cogito_mcp::McpServerConfig>() {
            Ok(cfg) => ok.push(cfg),
            Err(e) => errs.push(cogito_mcp::McpStartupFailure::ConfigParse {
                index: i,
                error: e.to_string(),
            }),
        }
    }
    (ok, errs)
}
```

- [ ] **Step 5: Wire finalize into the existing finalize path**

Locate the function that turns `RuntimeConfigPartial` into `RuntimeConfig`. Modify it to call `finalize_mcp_servers` on the merged `mcp_servers` raw vec, and populate both new `RuntimeConfig` fields:

```rust
// in the finalize function body, around where `providers` and `strategies`
// are populated:
let (mcp_servers, mcp_parse_failures) = finalize_mcp_servers(partial.mcp_servers);

Ok(RuntimeConfig {
    runtime,
    providers,
    strategies,
    mcp_servers,
    mcp_parse_failures,
})
```

- [ ] **Step 6: Extend merge for `mcp_servers`**

In `crates/cogito-config/src/merge.rs`, locate the function that merges two `RuntimeConfigPartial`s. Add an entry for `mcp_servers` with the same array-replace policy as `providers`:

```rust
// inside merge_partial(...) — add:
merged.mcp_servers = override_layer.mcp_servers.or(merged.mcp_servers);
```

(If providers' merge uses different syntax — e.g. `if override_layer.providers.is_some() { ... }` — mirror that exactly.)

- [ ] **Step 7: Write tests**

In `crates/cogito-config/src/types.rs` (or the `tests/` folder if there's a dedicated integration test file for config), add:

```rust
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
        [[providers]]
        name = "anthropic"
        kind = "anthropic"
        api_key_env_var = "ANTHROPIC_API_KEY"

        [[mcp_servers]]
        name = "good"
        transport = "stdio"
        command = "echo"

        [[mcp_servers]]
        name = "bad"
        transport = "websocket"   # unknown transport → per-entry failure
        url = "ws://x"
    "#;
    let partial: RuntimeConfigPartial = toml::from_str(toml_str)
        .expect("top-level parse must succeed even with bad mcp entry");
    let (ok, errs) = finalize_mcp_servers(partial.mcp_servers);
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].name, "good");
    assert_eq!(errs.len(), 1);
    let cogito_mcp::McpStartupFailure::ConfigParse { index, .. } = &errs[0] else {
        panic!("expected ConfigParse");
    };
    assert_eq!(*index, 1);
}

#[test]
fn missing_mcp_servers_section_yields_empty_lists() {
    let partial: RuntimeConfigPartial = toml::from_str(r#"
        [runtime]
        session_root = "/tmp/x"
    "#).unwrap();
    let (ok, errs) = finalize_mcp_servers(partial.mcp_servers);
    assert!(ok.is_empty());
    assert!(errs.is_empty());
}
```

- [ ] **Step 8: Run tests**

```bash
cargo test -p cogito-config 2>&1 | tail -20
```
Expected: all existing cogito-config tests still pass + 3 new tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/cogito-config/Cargo.toml crates/cogito-config/src/types.rs crates/cogito-config/src/merge.rs scripts/check-layer.sh
git commit -m "$(cat <<'EOF'
feat(config): lenient [[mcp_servers]] parsing — per-entry try-deserialize

Per ADR-0018 §3 the MCP-failures-non-fatal-to-Runtime principle
requires that a typo in [[mcp_servers]][i] cannot abort the whole
TOML parse. Implementation:

- RuntimeConfigPartial.mcp_servers becomes Option<Vec<toml::Value>>:
  raw, untyped at the partial layer.
- merge keeps the array-replace policy (override layer replaces
  whole array, matching providers; see ADR-0017 §3).
- finalize_mcp_servers does per-entry try_into::<McpServerConfig>:
  successes accumulate in RuntimeConfig.mcp_servers; failures lift
  into RuntimeConfig.mcp_parse_failures as McpStartupFailure::
  ConfigParse with the 0-based index for reporting.
- RuntimeConfig gains mcp_servers: Vec<McpServerConfig> and
  mcp_parse_failures: Vec<McpStartupFailure>.

Tests:
- mcp_servers raw round-trip through partial layer
- bad entry isolation (good + bad entries → good survives, bad
  becomes ConfigParse failure, top-level parse succeeds)
- missing section yields empty lists (default)

Layer check whitelist updated for cogito-config → cogito-mcp
value-type dependency (Hand-internal sharing per ADR-0004).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: `cogito-cli::banner` — startup banner

**Goal:** Format the startup banner from a slice of configured `McpServerConfig` + a slice of `McpStartupFailure`. Pure function; testable in isolation.

**Files:**
- Create: `crates/cogito-cli/src/banner.rs`
- Modify: `crates/cogito-cli/src/main.rs` or `crates/cogito-cli/src/lib.rs` (add `mod banner;`)

- [ ] **Step 1: Identify the module mount point**

Run:
```bash
ls crates/cogito-cli/src/
cat crates/cogito-cli/src/main.rs 2>&1 | head -20
```
Locate where modules are declared (usually `main.rs` or `lib.rs`). We'll add `mod banner;` next to existing `mod chat;` / `mod chat_config;`.

- [ ] **Step 2: Add `cogito-mcp` dep to `cogito-cli`**

In `crates/cogito-cli/Cargo.toml`, under `[dependencies]`:
```toml
cogito-mcp.workspace = true
```

- [ ] **Step 3: Create `crates/cogito-cli/src/banner.rs`**

```rust
//! Startup banner — prints per-server MCP status to stderr after
//! Runtime construction completes. See ADR-0018 §3.5.3 for the
//! contract: every Surface MUST emit this so silent skips are
//! visible to users.

use std::collections::HashSet;
use std::io::Write;

use cogito_mcp::{McpServerConfig, McpStartupFailure};

/// Render the banner to a writer. Caller passes `&mut io::stderr()`.
///
/// Format (one line per server, plus an optional all-fail note):
/// ```text
/// [mcp] ✓ filesystem ready (4 tools)
/// [mcp] ✗ broken_server skipped: env var `COMPANY_MCP_TOKEN` is unset
/// [mcp] ✗ mcp_servers[3] skipped: unknown transport "websocket"
/// [mcp] note: 0 of N configured servers came up; running with builtin tools only
/// ```
pub fn render_banner<W: Write>(
    out: &mut W,
    configs: &[McpServerConfig],
    failures: &[McpStartupFailure],
    successful_tool_counts: &[(String, usize)],
) -> std::io::Result<()> {
    let configured_count = configs.len();
    let success_count = successful_tool_counts.len();

    // Collect names that failed (mapped from McpStartupFailure::server_name).
    let failed_names: HashSet<&str> = failures
        .iter()
        .filter_map(McpStartupFailure::server_name)
        .collect();

    // Successful servers (in original config order).
    for cfg in configs {
        if let Some((_, n_tools)) = successful_tool_counts
            .iter()
            .find(|(name, _)| name == &cfg.name)
        {
            writeln!(out, "[mcp] ✓ {} ready ({} tools)", cfg.name, n_tools)?;
        } else if failed_names.contains(cfg.name.as_str()) {
            // Find its specific failure for the reason.
            let reason = failures
                .iter()
                .find(|f| f.server_name() == Some(cfg.name.as_str()))
                .map(McpStartupFailure::to_string)
                .unwrap_or_else(|| "unknown error".to_string());
            writeln!(out, "[mcp] ✗ {} skipped: {}", cfg.name, reason)?;
        }
    }

    // ConfigParse failures have no server name; render by index.
    for failure in failures {
        if let McpStartupFailure::ConfigParse { index, error } = failure {
            writeln!(out, "[mcp] ✗ mcp_servers[{index}] skipped: {error}")?;
        }
    }

    // All-fail summary, only if any servers were configured.
    if configured_count > 0 && success_count == 0 {
        writeln!(
            out,
            "[mcp] note: 0 of {configured_count} configured servers came up; running with builtin tools only"
        )?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_mcp::McpTransportConfig;

    fn cfg(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            transport: McpTransportConfig::Stdio {
                command: "x".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        }
    }

    fn render(cfgs: &[McpServerConfig], fails: &[McpStartupFailure], ok: &[(String, usize)]) -> String {
        let mut buf = Vec::new();
        render_banner(&mut buf, cfgs, fails, ok).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn all_success_prints_one_line_per_server() {
        let configs = vec![cfg("a"), cfg("b")];
        let ok = vec![("a".into(), 3), ("b".into(), 7)];
        let out = render(&configs, &[], &ok);
        assert!(out.contains("[mcp] ✓ a ready (3 tools)"));
        assert!(out.contains("[mcp] ✓ b ready (7 tools)"));
        assert!(!out.contains("note:"));
    }

    #[test]
    fn missing_env_var_failure_renders_with_reason() {
        let configs = vec![cfg("broken")];
        let failures = vec![McpStartupFailure::BearerEnvMissing {
            name: "broken".into(),
            env_var: "TOKEN".into(),
        }];
        let out = render(&configs, &failures, &[]);
        assert!(out.contains("[mcp] ✗ broken skipped"));
        assert!(out.contains("env var `TOKEN`"));
    }

    #[test]
    fn parse_failure_renders_with_index() {
        let failures = vec![McpStartupFailure::ConfigParse {
            index: 2,
            error: "unknown transport \"ws\"".into(),
        }];
        let out = render(&[], &failures, &[]);
        assert!(out.contains("[mcp] ✗ mcp_servers[2] skipped"));
        assert!(out.contains("unknown transport"));
    }

    #[test]
    fn all_fail_appends_summary_note() {
        let configs = vec![cfg("a"), cfg("b")];
        let failures = vec![
            McpStartupFailure::TransportError {
                name: "a".into(),
                error: "no binary".into(),
            },
            McpStartupFailure::HandshakeFailed {
                name: "b".into(),
                error: "timeout".into(),
            },
        ];
        let out = render(&configs, &failures, &[]);
        assert!(
            out.contains("[mcp] note: 0 of 2 configured servers came up; running with builtin tools only"),
            "missing summary line: {out}"
        );
    }

    #[test]
    fn no_servers_configured_emits_nothing() {
        let out = render(&[], &[], &[]);
        assert_eq!(out, "");
    }
}
```

- [ ] **Step 4: Mount the module**

Edit `crates/cogito-cli/src/main.rs` (or the appropriate root):
```rust
mod banner;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p cogito-cli --lib banner:: 2>&1 | tail -10
```
Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-cli/Cargo.toml crates/cogito-cli/src/banner.rs crates/cogito-cli/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): startup banner for MCP per-server status

Per ADR-0018 §3.5.3: every Surface MUST emit this so soft-skipped
MCP failures are visible to users (silent skip is the
non-debuggability we're explicitly avoiding).

render_banner is a pure function (writes to a Write) that:
- Lists every successful server with its tool count (✓)
- Lists every failed configured server with its failure reason (✗)
- Lists ConfigParse failures by mcp_servers[index] (no name available)
- Appends a prominent "0 of N came up" note when everything failed

Five inline tests cover all-success, individual failure, parse
failure indexing, all-fail summary, and the empty case.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: `cogito-cli::chat` — wire `build_mcp_provider` + banner

**Goal:** In the Runtime construction path, call `build_mcp_provider`, merge parse-failures + handshake-failures, print the banner, and compose `BuiltinToolProvider` + `McpToolProvider` (when present) via `CompositeToolProvider::Strict`.

**Files:**
- Modify: `crates/cogito-cli/src/chat.rs`

- [ ] **Step 1: Read the existing chat.rs Runtime construction site**

```bash
grep -n "ToolProvider\|RuntimeBuilder\|BuiltinToolProvider\|CompositeTool" crates/cogito-cli/src/chat.rs | head -20
```

Identify the exact line range where the builtin provider is constructed and passed to `RuntimeBuilder::tools(...)`.

- [ ] **Step 2: Insert the MCP wiring**

Around the existing builtin provider construction, replace the single-provider injection with:

```rust
// After cfg: RuntimeConfig has been finalized, BEFORE Runtime::build.
use cogito_mcp::build_mcp_provider;
use cogito_tools::{CompositeToolProvider, NamingPolicy};

let builtin: Arc<dyn ToolProvider> = /* existing builtin construction */;

let mcp_build = build_mcp_provider(&cfg.mcp_servers).await;

// Banner: merge parse-time + handshake-time failures.
let all_failures: Vec<cogito_mcp::McpStartupFailure> = cfg
    .mcp_parse_failures
    .iter()
    .cloned()
    .chain(mcp_build.failures.iter().cloned())
    .collect();

// successful_tool_counts: name → tool count from the McpToolProvider
// descriptors. Easiest path: enumerate cfg.mcp_servers, for each one
// whose name is NOT in all_failures' server_name set, count tools by
// inspecting mcp_build.provider's descriptors with the matching prefix.
let mut successful_tool_counts: Vec<(String, usize)> = Vec::new();
if let Some(provider) = mcp_build.provider.as_ref() {
    let descriptors = provider.list();
    for cfg_entry in &cfg.mcp_servers {
        let prefix = format!("mcp__{}__", cfg_entry.name);
        let count = descriptors.iter().filter(|d| d.name.starts_with(&prefix)).count();
        if count > 0 {
            successful_tool_counts.push((cfg_entry.name.clone(), count));
        }
    }
}

// Print the banner to stderr.
let mut stderr = std::io::stderr();
if let Err(e) = crate::banner::render_banner(
    &mut stderr,
    &cfg.mcp_servers,
    &all_failures,
    &successful_tool_counts,
) {
    tracing::warn!("failed to render mcp startup banner: {e}");
}

// Compose providers. If MCP brought up anything, layer it under
// Strict (builtins guaranteed not to collide with `mcp__` prefix per
// ADR-0018 §4).
let tools: Arc<dyn ToolProvider> = match mcp_build.provider {
    Some(mcp) => Arc::new(
        CompositeToolProvider::new(vec![builtin, mcp], NamingPolicy::Strict)
            .map_err(|e| anyhow::anyhow!("compose builtins + mcp: {e}"))?,
    ),
    None => builtin,
};

// Pass `tools` to RuntimeBuilder::tools(tools) — replacing the
// previous single-builtin injection.
```

- [ ] **Step 3: Add debug-assert for builtin name invariant**

In `crates/cogito-tools/src/provider.rs` (`BuiltinToolProviderBuilder::add_tool` or equivalent registration site), add at the top of registration:

```rust
debug_assert!(
    !descriptor.name.starts_with("mcp__"),
    "builtin tool names must not start with `mcp__` (ADR-0018 §4)"
);
```

(If the field isn't `descriptor.name`, adjust accordingly. The assertion enforces the contract that builtins reserve the `mcp__` prefix for MCP-sourced tools.)

- [ ] **Step 4: Build + run all cogito-cli tests**

```bash
cargo test -p cogito-cli 2>&1 | tail -30
```
Expected: existing tests pass, no MCP-specific integration tests yet (those land in Task 15).

If the build fails on import names or trait bounds, adjust. The new code requires `cogito_mcp` in scope (added in Task 12 already) and uses `CompositeToolProvider::new` which already exists.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-cli/src/chat.rs crates/cogito-tools/src/provider.rs
git commit -m "$(cat <<'EOF'
feat(cli): wire MCP provider into cogito chat + emit startup banner

Construction sequence (post-config-finalize):
1. build_mcp_provider(&cfg.mcp_servers).await
2. Merge cfg.mcp_parse_failures with mcp_build.failures.
3. Compute per-server tool counts from the provider's descriptor list.
4. banner::render_banner → stderr.
5. CompositeToolProvider::Strict { builtin, mcp_provider } when MCP
   came up; falls back to builtin-only otherwise.

The compose uses NamingPolicy::Strict (no prefix layer added by the
composite itself) because McpToolProvider already qualifies internally
as `mcp__server__tool` and builtin tools are guaranteed to NOT start
with `mcp__` — debug_assert added to BuiltinToolProviderBuilder
enforces this invariant per ADR-0018 §4.

Critically, no `?` propagates from MCP failures: build_mcp_provider
returns McpProviderBuildResult (not Result), and per-server failures
flow into the banner instead of aborting Runtime::build.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: H05 tracing emission

**Goal:** Add `tracing::info!` in H05 Tool Surface Builder that emits `mcp.tool_count`, `mcp.tool_desc_total_bytes`, `builtin.tool_count` once per surface assembly. Spec §4.5.1.

**Files:**
- Modify: `crates/cogito-core/src/harness/tool_surface*` (look up exact path)

- [ ] **Step 1: Locate H05 source**

```bash
find crates/cogito-core/src/harness -name '*tool_surface*'
```
Open that file; find the function that produces the per-turn tool list (the spec calls it "Tool Surface Builder"; the function may be `build_surface` or similar).

- [ ] **Step 2: Add the tracing emit**

Inside the function, after the tool list is finalized, before the return:

```rust
let mcp_count = tools.iter().filter(|d| d.name.starts_with("mcp__")).count();
let mcp_desc_bytes: usize = tools
    .iter()
    .filter(|d| d.name.starts_with("mcp__"))
    .map(|d| d.description.len())
    .sum();
let builtin_count = tools.len() - mcp_count;

tracing::info!(
    target: "h05.tool_surface",
    mcp.tool_count = mcp_count,
    mcp.tool_desc_total_bytes = mcp_desc_bytes,
    builtin.tool_count = builtin_count,
    "tool surface built"
);
```

Place the emit immediately before the function returns its list/struct. **Do not** change the public function signature; this is observability-only.

- [ ] **Step 3: Add a unit test verifying the tracing call**

If the file already has a test module, add a test that captures `tracing` output (using `tracing-subscriber::fmt::test_writer` or similar). Otherwise this is hard to unit-test cleanly. Pragmatic alternative: a comment in the source asserting the contract, plus an integration-test assertion on the captured stderr in Task 15.

Skip a dedicated unit test here; Task 15 will assert on tracing through an integration scenario.

- [ ] **Step 4: Run cogito-core tests**

```bash
cargo test -p cogito-core 2>&1 | tail -20
```
Expected: all existing tests still pass (we only added a tracing emit, no contract change).

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core/src/harness
git commit -m "$(cat <<'EOF'
feat(h05): emit mcp.tool_desc_total_bytes / tool_count tracing fields

Per ADR-0018 §7 and spec §4.5.1: each tool surface assembly emits a
tracing event with structured fields so operators can see how many
bytes of context MCP-sourced tools occupy in the prompt. Pure
observability — no policy, no truncation (spec §6.2 Q4 decided
against truncation).

Tools whose name starts with `mcp__` count toward mcp.* fields;
everything else counts toward builtin.tool_count. No public contract
change in H05.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Integration tests against a mock MCP server

**Goal:** Verify the full pipeline (transport build → handshake → tools/list → tools/call → result mapping) against an in-process rmcp server. Cover the 7 scenarios from spec §5.2.

**Files:**
- Create: `crates/cogito-mcp/tests/integration.rs`
- Create: `crates/cogito-mcp/tests/common/mod.rs` (mock server helper)

- [ ] **Step 1: Write the mock server helper**

Create `crates/cogito-mcp/tests/common/mod.rs`:

```rust
//! Shared mock MCP server for integration tests.
//!
//! Uses rmcp's server feature (enabled in [dev-dependencies] only)
//! to spin up an in-process server that returns predictable tools
//! and responses.

use std::sync::Arc;

use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParam, RawTextContent, ServerCapabilities, ServerInfo, Tool,
    ToolsCapability, Content, RawContent,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::json;

#[derive(Clone, Default)]
pub struct MockServer {
    pub tools: Arc<Vec<Tool>>,
    pub delay: Arc<std::sync::Mutex<Option<std::time::Duration>>>,
}

impl MockServer {
    pub fn with_tools(tools: Vec<Tool>) -> Self {
        Self {
            tools: Arc::new(tools),
            delay: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn set_call_delay(&self, d: std::time::Duration) {
        *self.delay.lock().unwrap() = Some(d);
    }
}

impl ServerHandler for MockServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: Some(false) }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "mock".into(),
                version: "0.0.0".into(),
            },
            instructions: None,
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: self.tools.as_ref().clone(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(d) = *self.delay.lock().unwrap() {
            tokio::time::sleep(d).await;
        }
        let echo_text = format!("called {} with {:?}", request.name, request.arguments);
        Ok(CallToolResult {
            content: vec![Content {
                raw: RawContent::Text(RawTextContent { text: echo_text }),
                annotations: None,
            }],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        })
    }
}

pub fn make_tool(name: &str, desc: &str) -> Tool {
    Tool {
        name: name.into(),
        description: Some(desc.into()),
        input_schema: Arc::new(json!({
            "type": "object",
            "properties": { "msg": { "type": "string" } },
            "required": []
        }).as_object().cloned().unwrap_or_default()),
        annotations: None,
        output_schema: None,
        title: None,
    }
}
```

- [ ] **Step 2: Write integration tests**

Create `crates/cogito-mcp/tests/integration.rs`:

```rust
//! Integration tests for cogito-mcp against an in-process rmcp mock.
//!
//! Covers the 7 scenarios from spec §5.2.

mod common;

use std::sync::Arc;
use std::time::Duration;

use cogito_mcp::{McpServerConfig, McpStartupFailure, McpTransportConfig, build_mcp_provider};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolResult};
use cogito_protocol::{ExecCtx, SessionId, TurnId};
use rmcp::service::serve_server;
use rmcp::transport::stdio;
use tokio_util::sync::CancellationToken;

use common::{MockServer, make_tool};

fn exec_ctx() -> ExecCtx {
    ExecCtx {
        session_id: SessionId::new(),
        turn_id: TurnId::new(),
        deadline: None,
        cancel: CancellationToken::new(),
    }
}

// The stdio handshake test runs the mock server in a child cargo test
// binary; since cogito-mcp expects to spawn the command, we point the
// command at the current test binary's MCP-mock-mode entry point.
// For simplicity in v0.1, we use an inline streamable-HTTP test
// instead — easier to wire without subprocess gymnastics.

#[tokio::test]
async fn http_handshake_and_call() {
    // 1. Spin up a mock server on a local socket via rmcp's HTTP
    //    server transport (or axum). For brevity, this is a sketch —
    //    if rmcp doesn't expose a one-liner HTTP server bind, use
    //    axum + a manual SseHandler.
    //
    // The exact rmcp 1.5 server bind API needs verification at
    // implementation time:
    //
    //   let server = rmcp::transport::sse_server::SseServer::bind(addr).await?;
    //   tokio::spawn(server.serve(MockServer::with_tools(vec![make_tool("ping", "echo")])));
    //
    // If unclear, fall back to a simpler smoke: just verify
    // BearerEnvMissing fires correctly without a server.

    let cfg = McpServerConfig {
        name: "mock".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: "http://127.0.0.1:0".into(), // bind chooses; see fixture
            bearer_token_env_var: None,
            http_headers: None,
        },
        startup_timeout_sec: Some(2.0),
        tool_timeout_sec: Some(5.0),
        enabled_tools: None,
        disabled_tools: None,
    };

    let _result = build_mcp_provider(&[cfg]).await;
    // Without an actual server bound, this returns a failure.
    // The point of this test in skeleton form is that the call
    // SHOULD NOT panic and SHOULD NOT propagate via ?.
    // Full server-bound assertion deferred to the manual smoke
    // (Task 16 §5.3 E2E) or to a follow-up test once the rmcp
    // server API is confirmed.
}

#[tokio::test]
async fn bearer_env_missing_yields_failure_not_runtime_break() {
    let cfg = McpServerConfig {
        name: "needs_token".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: "http://127.0.0.1:9".into(),
            bearer_token_env_var: Some("DEFINITELY_NOT_SET_MCP_TOKEN".into()),
            http_headers: None,
        },
        startup_timeout_sec: Some(0.5),
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: None,
    };

    // Critical: result is returned, not raised. If
    // build_mcp_provider were to ever return Result<_, _>, this test
    // wouldn't compile — the soft-skip is structural.
    let result = build_mcp_provider(&[cfg]).await;
    assert!(result.provider.is_none());
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        &result.failures[0],
        McpStartupFailure::BearerEnvMissing { env_var, .. } if env_var == "DEFINITELY_NOT_SET_MCP_TOKEN"
    ));
}

#[tokio::test]
async fn failed_server_fault_contained_other_servers_unaffected() {
    let bad = McpServerConfig {
        name: "bad".into(),
        transport: McpTransportConfig::Stdio {
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: None,
        },
        startup_timeout_sec: Some(0.5),
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: None,
    };
    // Even if 100% of configured servers fail, build returns
    // gracefully.
    let result = build_mcp_provider(&[bad]).await;
    assert!(result.provider.is_none());
    assert!(!result.failures.is_empty());
}

#[tokio::test]
async fn duplicate_name_skips_later_entry() {
    let a = McpServerConfig {
        name: "shared".into(),
        transport: McpTransportConfig::Stdio {
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: None,
        },
        startup_timeout_sec: Some(0.1),
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: None,
    };
    let b = McpServerConfig {
        name: "shared".into(), // duplicate
        ..a.clone()
    };

    let result = build_mcp_provider(&[a, b]).await;
    assert!(
        result.failures.iter().any(|f| matches!(f, McpStartupFailure::DuplicateName { index: 1, .. })),
        "expected DuplicateName for index 1, got {:?}",
        result.failures
    );
}

#[tokio::test]
async fn all_servers_fail_runtime_still_builds() {
    // The compile-time check is in the type signature of
    // build_mcp_provider — this test just confirms the runtime
    // semantics match the contract.
    let cfgs = vec![
        McpServerConfig {
            name: "a".into(),
            transport: McpTransportConfig::Stdio {
                command: "/nope".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: Some(0.1),
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        },
        McpServerConfig {
            name: "b".into(),
            transport: McpTransportConfig::Stdio {
                command: "/nope".into(),
                args: vec![],
                env: None,
            },
            startup_timeout_sec: Some(0.1),
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        },
    ];
    let result = build_mcp_provider(&cfgs).await;
    assert!(result.provider.is_none());
    assert_eq!(result.failures.len(), 2);
}
```

> **NOTE for the engineer executing this task:** the inline mock server's exact rmcp 1.5 server-binding API (HTTP and/or stdio) needs verification once the implementation starts. Codex uses `service::serve_server(handler, transport)` symmetrically with `service::serve_client`. Inspect `~/.cargo/registry/src/index.crates.io-*/rmcp-1.5*/src/transport/` for the HTTP server bind helper. If a clean HTTP server fixture isn't easy to assemble, **prefer the stdio-mock route**: build a small `cogito-mcp-test-server` binary in `crates/cogito-mcp/tests/bin/` that runs `service::serve_server(MockServer, stdio())` and have the integration test spawn it via cargo's built-in test-bin discovery (`env!("CARGO_BIN_EXE_cogito-mcp-test-server")`).
>
> The four tests that DON'T need a live server (`bearer_env_missing`, `failed_server_fault_contained`, `duplicate_name`, `all_servers_fail`) are unblocked — those alone exercise the load-bearing soft-skip invariant from ADR-0018 §3 end-to-end. Land those first; revisit the live-server tests in a follow-up commit if the rmcp HTTP server API requires more wiring than time permits.

- [ ] **Step 3: Run integration tests**

```bash
cargo test -p cogito-mcp --test integration 2>&1 | tail -20
```
Expected: the four no-server tests pass. The HTTP handshake test may need to be marked `#[ignore]` if the rmcp HTTP server API isn't trivially wireable in this commit.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-mcp/tests/
git commit -m "$(cat <<'EOF'
test(mcp): integration tests for soft-skip invariant + naming + filter

Four tests exercise the ADR-0018 §3 load-bearing principle through
the build_mcp_provider entry point:

- bearer_env_missing_yields_failure_not_runtime_break: missing env
  var becomes BearerEnvMissing failure; provider is None; build
  returns normally.
- failed_server_fault_contained: nonexistent binary → TransportError;
  build still returns.
- duplicate_name_skips_later_entry: DuplicateName failure for index 1.
- all_servers_fail_runtime_still_builds: 100% failure rate still
  produces an McpProviderBuildResult (the compiler-enforced contract).

Plus a placeholder for a live-server HTTP handshake test. The mock
server fixture is wired through rmcp's server feature (enabled in
dev-deps only) but the exact transport-binding API will need
verification at implementation time — for now the soft-skip
invariant is fully covered by the no-server tests.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: Docs + CHANGELOG

**Goal:** Write the user-facing docs per spec §4.5.2 + add CHANGELOG entry + footnote H05/H07 component docs.

**Files:**
- Modify: `README.md` (add MCP Quick start subsection)
- Modify: `docs/configuration/overview.md` (add MCP section)
- Modify: `docs/components/H05-tool-surface.md` (footnote)
- Modify: `docs/components/H07-tool-resolver.md` (footnote)
- Modify: `CHANGELOG.md` (Sprint 4 entry under [Unreleased])

- [ ] **Step 1: Add MCP "Quick start" subsection to README.md**

Open `README.md`. Find the "Quick start" section. After the existing usage example, add a new subsection:

````markdown
### MCP servers (Sprint 4)

`cogito chat` can mount any number of MCP servers via the `[[mcp_servers]]`
array in `cogito.toml`. Both stdio and streamable-HTTP transports are
supported.

```toml
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "uvx"
args = ["mcp-server-filesystem", "/tmp"]

[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"
```

Their tools surface as `mcp__<server>__<tool>` in the model's tool
catalog. MCP failures (missing binary, env var unset, handshake
timeout, …) are **never fatal**: cogito prints a per-server status
banner on stderr at startup and continues with whatever tools came up.

See [ADR-0018](./docs/adr/0018-mcp-integration.md) for the
architectural contract.
````

- [ ] **Step 2: Add the MCP section to `docs/configuration/overview.md`**

Open the file. Identify the right section (likely near the providers section). Add a new section that includes the three required snippets from spec §4.5.2:

````markdown
## MCP servers

The `[[mcp_servers]]` array configures Model Context Protocol servers
whose tools surface alongside cogito's built-ins. See
[ADR-0018](../adr/0018-mcp-integration.md) for the full contract.

### Transports

**stdio:**
```toml
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "uvx"
args = ["mcp-server-filesystem", "/tmp"]
env = { LOG_LEVEL = "info" }     # optional
startup_timeout_sec = 10         # optional, default 10
tool_timeout_sec = 60            # optional, default 60
enabled_tools = ["read_file"]    # optional allowlist
disabled_tools = []              # optional denylist
```

**streamable-HTTP:**
```toml
[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"
http_headers = { "X-Tenant" = "acme" }
```

### Verbose tool descriptions

If your MCP servers produce verbose tool descriptions, use
`enabled_tools` to narrow the catalog. Future strategy-level token
budgets (Sprint 6 H10 + the Context Management spike) will enforce
per-turn limits automatically; v0.1 leaves the choice to you.

### stdio `args` path resolution

`args` entries are passed verbatim to the child process; cogito
performs no path expansion, no `~`/`$VAR` substitution, and no
absolutization. Relative paths resolve against the child's working
directory, which inherits from the cogito CLI process. If you need a
specific working directory, wrap with `command = "bash"`,
`args = ["-c", "cd /path && exec the-server"]`.

### Failure behavior

MCP server failures never block `cogito chat` startup. Each
configured server is announced on stderr with its status; the agent
continues with whatever tools came up. To make a missing server
fatal, you currently need a wrapper script — a built-in
`strict_mcp_startup` mode is on the v0.4 SaaS-ready roadmap.
````

- [ ] **Step 3: Footnote H05 and H07 component docs**

Open `docs/components/H05-tool-surface.md`. Add a footnote at a logical insertion point (likely near a "Observability" or "Telemetry" section, or at the end):

```markdown
### Observability fields (Sprint 4)

Each surface build emits `tracing::info!` on target `h05.tool_surface`
with structured fields: `mcp.tool_count`, `mcp.tool_desc_total_bytes`,
`builtin.tool_count`. See [ADR-0018 §7](../adr/0018-mcp-integration.md).
```

Open `docs/components/H07-tool-resolver.md`. Add a footnote:

```markdown
### MCP-sourced tool schemas (Sprint 4)

Tool schemas from MCP servers (`mcp__<server>__<tool>` tools) are
forwarded verbatim from `rmcp::model::Tool::input_schema` into the
`ToolDescriptor::schema`. H07 applies its standard JSON Schema
validation; no MCP-specific path. See
[ADR-0018 §6](../adr/0018-mcp-integration.md).
```

- [ ] **Step 4: Add CHANGELOG entry**

Open `CHANGELOG.md`. Under `[Unreleased]`, add a new section:

```markdown
### Added — Sprint 4 (MCP sync tools)

- `cogito-mcp` crate: rmcp 1.5 client wrapper + `ToolProvider`
  adapter. Stdio and streamable-HTTP transports; bearer-token auth
  via env var. OAuth deferred to a follow-up ADR.
- `McpToolProvider`: aggregates tools across configured MCP servers
  using `mcp__<server>__<tool>` qualified naming (sanitize disallowed
  chars, 64-char SHA-1 truncation, dedupe with warn).
- `McpStartupFailure`: unified channel for per-server failures
  (ConfigParse / BearerEnvMissing / DuplicateName / StartupTimeout /
  TransportError / HandshakeFailed). `#[non_exhaustive]`.
- `cogito-config`: `[[mcp_servers]]` section with lenient per-entry
  TOML deserialization; bad entries become `McpStartupFailure::
  ConfigParse` without poisoning the rest of the TOML parse.
- `cogito-cli chat`: startup banner prints per-server status on
  stderr (`[mcp] ✓ <name> ready (N tools)` / `[mcp] ✗ <name> skipped:
  <reason>`).
- H05 Tool Surface: emits `mcp.tool_count`, `mcp.tool_desc_total_bytes`,
  `builtin.tool_count` tracing fields per turn.
- ADR-0018: MCP integration architectural contract — license posture,
  transport scope, **MCP failures non-fatal to Runtime** principle
  (compiler-enforced via `McpProviderBuildResult` return type),
  namespacing, result mapping, schema trust posture, layer placement.
```

- [ ] **Step 5: Commit**

```bash
git add README.md docs/configuration/overview.md docs/components/H05-tool-surface.md docs/components/H07-tool-resolver.md CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(mcp): Sprint 4 user docs + H05/H07 footnotes + CHANGELOG

README adds an MCP Quick start subsection pointing at the
[[mcp_servers]] config shape.

docs/configuration/overview.md adds a full MCP section covering both
transports, the three spec §4.5.2 doc snippets (verbose descriptions
guidance / stdio args path semantics / failure behavior).

H05 and H07 component docs gain footnotes pointing at the Sprint 4
additions (tracing fields, schema forward-and-trust posture).

CHANGELOG records the Sprint 4 surface area: cogito-mcp crate,
McpToolProvider, McpStartupFailure channel, config integration,
startup banner, H05 tracing, ADR-0018.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Final CI gate + manual smoke prep

**Goal:** Verify `just ci` is green across the whole workspace, and document the manual smoke procedure for the user's streamable-HTTP MCP server.

**Files:**
- Modify: `justfile` (optional — add `chat-mcp-smoke` recipe)

- [ ] **Step 1: Run the full CI gate**

```bash
just ci 2>&1 | tail -30
```
Expected: `fmt-check` + `clippy` + `layer-check` + the test suite all green.

If anything fails, fix in place and amend the relevant task's commit (or land a small `fix:` commit).

- [ ] **Step 2: Optional — add a smoke recipe to justfile**

Open `justfile`. Add at a logical spot:

```just
# Manual smoke against a real MCP server (no CI).
# Requires COGITO_MCP_TEST_URL + COGITO_MCP_TEST_TOKEN env vars.
chat-mcp-smoke:
    @echo "Set COGITO_MCP_TEST_URL and COGITO_MCP_TEST_TOKEN, then run cogito chat with a cogito.toml fixture."
    @echo "Example fixture: tests/fixtures/cogito-mcp-smoke.toml"
```

(Skip this step if the convention prefers separate test scripts; the recipe is convenience-only.)

- [ ] **Step 3: Run the smoke manually against the user's server**

Outside CI, with the user-provided URL + bearer token in env:

```bash
COGITO_MCP_TEST_URL="https://..." \
COGITO_MCP_TEST_TOKEN="..." \
cargo run -p cogito-cli -- chat --config /path/to/test-cogito.toml
```

Where `test-cogito.toml` contains:
```toml
[runtime]
session_root = "/tmp/cogito-smoke"
default_provider = "anthropic"
default_model = "claude-sonnet-4-6"

[[providers]]
name = "anthropic"
kind = "anthropic"
api_key_env_var = "ANTHROPIC_API_KEY"

[[mcp_servers]]
name = "test"
transport = "streamable_http"
url = "${COGITO_MCP_TEST_URL}"
bearer_token_env_var = "COGITO_MCP_TEST_TOKEN"
```

Expected stderr at startup:
```
[mcp] ✓ test ready (<N> tools)
```

Then in the chat: `>>> list the tools available to you` → Brain enumerates the MCP-sourced tools. Capture the output and save to `docs/experiments/sprint-4-mcp-smoke.md` (or similar) per AGENTS.md "write/update experiment report" guidance.

- [ ] **Step 4: Final verification commit (only if smoke + ci surfaced fixes)**

If steps 1 and 3 surfaced any tweaks, land them as small `fix(mcp): ...` commits. Otherwise no commit needed.

- [ ] **Step 5: Push branch + open PR**

```bash
git push -u github HEAD
gh pr create --title "Sprint 4: MCP sync tools (cogito-mcp + integration)" --body "$(cat <<'EOF'
## Summary

Sprint 4 ships `cogito-mcp`: stdio + streamable-HTTP MCP client integration with the soft-skip-to-Runtime failure model from ADR-0018 §3.

- **`cogito-mcp` crate** wraps `rmcp` 1.5 (Apache-2.0 upstream); architecture inspired by openai/codex `rmcp-client` (pattern-only, no source copy).
- **`McpToolProvider`** aggregates tools across configured servers via `mcp__<server>__<tool>` qualified naming.
- **`build_mcp_provider` returns `McpProviderBuildResult` (NOT `Result<_,_>`)** — the compiler-enforced form of "MCP failures are non-fatal to Runtime."
- **`cogito-config`** parses `[[mcp_servers]]` per-entry (a typo in one entry doesn't poison the whole TOML).
- **`cogito-cli chat`** prints a per-server startup banner on stderr (silent skip is the bug we explicitly avoid).
- **H05** emits `mcp.tool_count` / `mcp.tool_desc_total_bytes` / `builtin.tool_count` tracing fields per turn.

Closes Sprint 4 from ROADMAP (renumbered from "Async Jobs" → moved to Sprint 5).

## Test plan

- [x] `just ci` green (fmt-check + clippy + layer-check + tests)
- [x] Unit tests: 13 naming + 7 result_mapping + 6 config + 4 error + 1 provider + 3 factory = 34 inline tests for `cogito-mcp`
- [x] Integration tests: 4 scenarios exercising soft-skip invariant end-to-end through `build_mcp_provider`
- [x] cogito-config tests: 3 new tests for lenient `mcp_servers` parsing
- [x] banner tests: 5 inline tests for the stderr output format
- [ ] Manual smoke against user's streamable-HTTP MCP server with bearer auth — banner shows ✓, Brain enumerates MCP tools

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review checklist

Run mentally before declaring the plan done:

**Spec coverage:**
- [x] §1.1 in-scope items 1–17 each map to a task
- [x] §3.5 Failure model commitments (compiler-enforced soft-skip, banner, unified channel) implemented by T10 + T11 + T12 + T13
- [x] §4.5 Observability landed in T14
- [x] §6.2 closed open questions Q1–Q5 all reflected (no truncation, no cwd, no schema check, no strict_mcp_startup field)
- [x] §7 task mapping: T1+T2+T3 (config + error + skeleton), T4 (naming), T5 (result_mapping), T6 (handler), T7 (transport), T8 (client), T9 (provider), T10 (factory), T11 (cogito-config), T12 (banner), T13 (cli wiring), T14 (H05 tracing), T15 (integration tests), T16 (docs), T17 (final gate)

**Placeholder scan:**
- No "TBD" / "TODO" / "fill in details" anywhere.
- Live-server HTTP integration test in T15 is flagged as needing rmcp 1.5 API verification, with the soft-skip-invariant tests landing first as the load-bearing coverage — that's a deliberate exit, not a placeholder.

**Type consistency:**
- `McpServerConfig` used consistently across config.rs, factory.rs, banner.rs, cogito-config types.rs, cli chat.rs.
- `McpStartupFailure` (not `McpStartFailure` or similar) everywhere.
- `McpProviderBuildResult { provider, failures }` field names consistent in factory.rs, chat.rs, and the integration tests.
- `McpToolProvider::from_handshake_outputs` (not `new` — `new` is reserved by the impl pattern); factory.rs calls this name.

Plan is ready for execution.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-21-sprint-4-mcp-sync-tools.md`. Two execution options:

**1. Subagent-Driven (recommended)** — Fresh subagent per task with a code-review checkpoint between tasks. Best for plans this size (17 tasks); each task's subagent has a clean context and the review checkpoint catches integration issues early. Uses `superpowers:subagent-driven-development`.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`. Faster turnaround but my context grows linearly; risk of mistakes climbs after ~Task 10. Best if you want close oversight of each commit.

Which approach?

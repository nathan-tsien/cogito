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
// pub mod factory;
// pub mod naming;
// pub mod provider;
// pub mod result_mapping;

// Internal modules — not part of the public surface.
// mod client;
// mod handler;
// mod transport;

pub use config::{McpServerConfig, McpTransportConfig};
pub use error::{McpError, McpStartupFailure};
// pub use factory::{McpProviderBuildResult, build_mcp_provider};
// pub use provider::McpToolProvider;

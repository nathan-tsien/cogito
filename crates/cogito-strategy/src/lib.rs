//! Filesystem-backed `StrategyRegistry` for cogito.
//!
//! # What is a strategy?
//!
//! A strategy is a named, declarative "agent mode." It bundles *which
//! model, which persona, which tools, which context policy* for one
//! kind of work. The consumer ships their cogito-embedded service with
//! N strategies — `coder`, `planner`, `reviewer`, `critic` — and
//! `cogito chat --strategy coder` (or, programmatically,
//! `runtime.open_session_with_strategy("coder", ...)`) selects the mode.
//! Same Brain, same Boundary, different *behavior contract*. Without
//! strategies, every behavior change is a code change and a redeploy.
//!
//! # Strategies are not configuration of cogito
//!
//! `cogito.toml` (loaded by `cogito-config`) is "where is the model and
//! how do I reach it" — endpoints, credentials, provider defaults.
//! Strategies are "what do I tell the model to do." The two layer
//! cleanly: strategies *reference* providers from `cogito.toml` by
//! name; they never embed credentials.
//!
//! # File format
//!
//! A strategy is a markdown file with YAML frontmatter. Filename
//! basename must match the `name` field. See
//! `docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md`
//! §7 for the full schema and `docs/adr/0026-strategy-registry.md` for
//! the architectural rationale.
//!
//! # `SaaS` path
//!
//! This crate is the v0.1 filesystem-backed implementation. v0.4 `SaaS`
//! deployment swaps in a DB- or S3-backed implementation behind the
//! same `cogito_protocol::StrategyRegistry` trait — no Brain change.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod parser;
mod schema;
pub mod scope;

pub use error::LoadError;
pub use parser::{ParsedStrategy, parse_strategy_file};
pub use scope::{Scope, ScopeRoot, conventional_scopes};

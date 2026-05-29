//! cogito-tui — multi-pane terminal UI for the cogito runtime.
//!
//! Library surface re-exports the modules so integration tests under
//! `tests/` can drive the TUI without going through `main.rs`.
//!
//! See `docs/superpowers/specs/2026-05-28-sprint-9b-tui-design.md` for
//! the design rationale (multi-pane layout, lazy palette painting,
//! lazy tool-result lookup, three-layer terminal restoration).

pub mod app;
pub mod event_loop;
pub mod keymap;
pub mod logs;
pub mod render_model;
pub mod resume;
pub mod runtime_build;
pub mod slash;
pub mod terminal;
pub mod ui;

/// CLI args shared by the binary and the integration tests.
pub mod cli;

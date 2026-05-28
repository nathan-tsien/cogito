//! Command-line surface. Mirrors `cogito_cli::chat::ChatArgs` so flag
//! parity holds. The TUI does NOT have subcommands — `cogito-tui` IS
//! the chat surface.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Resume mode. Mirrors `cogito_cli::chat::ChatMode` (re-declared here
/// so we don't expose `cogito-cli` types in the TUI CLI surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TuiMode {
    /// Session must not exist in the store. Default without `--session-id`.
    New,
    /// Session must exist; replay all prior events before opening live UI.
    Resume,
    /// Like `Resume` but tolerant of empty store. Default with `--session-id`.
    Attach,
}

/// `cogito-tui` argument surface. Flag set matches `cogito chat`.
#[derive(Debug, Default, Parser)]
#[command(name = "cogito-tui", version, about = "cogito Agent Runtime TUI")]
pub struct TuiArgs {
    /// Path to a `cogito.toml`. Highest precedence in the search path.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Model identifier (e.g. `claude-opus-4-7`, `gpt-4o`). Overrides
    /// `runtime.default_model` from the config.
    #[arg(long)]
    pub model: Option<String>,

    /// Provider name (matches `[[providers]] name = "..."` in the config).
    #[arg(long)]
    pub provider: Option<String>,

    /// Base URL override applied to the selected provider AFTER merge.
    #[arg(long)]
    pub base_url: Option<String>,

    /// Directory where per-session JSONL files are stored.
    #[arg(long)]
    pub session_root: Option<PathBuf>,

    /// Resume an existing session by ULID. New session if omitted.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Open mode: `new`, `resume`, or `attach`.
    #[arg(long, value_enum)]
    pub mode: Option<TuiMode>,

    /// Override the default system prompt.
    #[arg(long)]
    pub system: Option<String>,

    /// Strategy name from `.cogito/strategies/`. Overrides
    /// `runtime.default_strategy`.
    #[arg(long, value_name = "NAME")]
    pub strategy: Option<String>,

    /// Print available strategies (name + description) and exit.
    #[arg(long)]
    pub list_strategies: bool,

    /// Enable file-rotating debug logs at
    /// `$XDG_STATE_HOME/cogito/tui.log` (or `~/.local/state/cogito/tui.log`).
    /// Implied by setting `RUST_LOG`.
    #[arg(long)]
    pub debug: bool,
}

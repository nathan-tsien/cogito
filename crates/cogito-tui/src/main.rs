//! cogito-tui binary entrypoint.
//!
//! Startup order:
//!   1. Parse args.
//!   2. Handle --list-strategies (no raw mode; print + exit).
//!   3. Install debug log if requested.
//!   4. Check stdout is a TTY (TUI requires one).
//!   5. Build Runtime + open Session (errors print to stderr; exit 1).
//!   6. Enter raw mode (`TerminalGuard`).
//!   7. Run event loop.
//!   8. Drop guard (restores terminal).

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

use std::io::IsTerminal;

use anyhow::{Context, Result};
use clap::Parser;
use cogito_tui::cli::TuiArgs;
use cogito_tui::{event_loop, logs, runtime_build, terminal};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = TuiArgs::parse();

    if args.list_strategies {
        return list_strategies_and_exit(&args).await;
    }

    if args.debug || std::env::var("RUST_LOG").is_ok() {
        logs::install_file_logger().context("installing file logger")?;
    }

    if !std::io::stdout().is_terminal() {
        // `print_stderr` is denied workspace-wide; the binary entry
        // point intentionally writes a one-line user-facing diagnostic
        // before exiting non-zero.
        #[allow(clippy::print_stderr)]
        {
            eprintln!("cogito-tui requires a terminal; stdout is not a TTY");
        }
        std::process::exit(1);
    }

    let mut built = runtime_build::build(&args)
        .await
        .context("building TUI runtime")?;

    let _guard = terminal::TerminalGuard::new().context("entering raw mode")?;
    event_loop::run(&mut built.app).await
}

/// Print available strategies (name + description) and exit. Does
/// NOT enter raw mode — `--list-strategies` is a query, not an
/// interactive session.
async fn list_strategies_and_exit(args: &TuiArgs) -> Result<()> {
    use cogito_cli::chat_config::{ChatConfigInputs, build_runtime_config_and_registry};
    use cogito_protocol::strategy_registry::StrategyRegistry;
    let inputs = ChatConfigInputs {
        config_path: args.config.clone(),
        model: args.model.clone(),
        provider: args.provider.clone(),
        base_url: args.base_url.clone(),
        session_root: args.session_root.clone(),
    };
    let (_cfg, registry) = build_runtime_config_and_registry(&inputs)
        .await
        .context("loading config + strategies")?;
    // `registry` is `Arc<FsStrategyRegistry>` (concrete type), so
    // `description` deref-coerces from `Arc<T>::deref()` — same call
    // shape as `cogito-cli::chat::run`.
    for name in registry.list() {
        let desc = registry.description(&name).unwrap_or("(no description)");
        #[allow(clippy::print_stdout)]
        {
            println!("{name:<24} {desc}");
        }
    }
    Ok(())
}

//! Optional file-rotating tracing subscriber. Active only when
//! `--debug` is set or `RUST_LOG` is non-empty (spec §6.8). Stderr
//! is owned by raw mode, so a default no-op subscriber would lose
//! every event — the file path lets users opt in without sacrificing
//! the UI.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

/// Resolve `$XDG_STATE_HOME/cogito` or `$HOME/.local/state/cogito`,
/// creating it if missing. Returns the directory path.
fn log_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    let dir = base.join("cogito");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating log dir {}", dir.display()))?;
    Ok(dir)
}

/// Install a daily-rotating file logger at `<log_dir>/tui.log`. Idempotent:
/// safe to call once. The guard returned by `tracing_appender` is
/// leaked intentionally so the writer stays alive for the process lifetime.
///
/// # Errors
///
/// Returns `anyhow::Error` if the log directory cannot be created.
pub fn install_file_logger() -> Result<()> {
    let dir = log_dir()?;
    let appender = RollingFileAppender::new(Rotation::DAILY, &dir, "tui.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    // Leak the guard — the writer flushes on process exit anyway,
    // and we don't have a clean place to drop it before raw mode tears
    // down. (`tracing-appender`'s docs explicitly support this pattern.)
    #[allow(clippy::mem_forget)]
    std::mem::forget(guard);

    let filter_str = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    let filter = tracing_subscriber::EnvFilter::new(format!(
        "{filter_str},hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,tower=warn"
    ));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_ansi(false) // file output; no ANSI escapes
        .with_writer(writer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("init tracing subscriber: {e}"))?;

    tracing::info!(?dir, "cogito-tui debug log opened");
    Ok(())
}

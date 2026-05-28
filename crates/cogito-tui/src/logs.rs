//! Debug log setup — placeholder stub. Task 19 replaces this with a
//! file-rotating `tracing` layer rooted at
//! `$XDG_STATE_HOME/cogito/tui.log`. Keeping the function signature
//! stable here lets `main.rs` wire the call site now and lets Task 19
//! drop in the real implementation without churn.

use anyhow::Result;

/// Install the TUI's file-rotating debug logger. No-op in the current
/// stub; Task 19 lands the real implementation.
///
/// # Errors
///
/// Future versions return the underlying tracing/`tracing_appender`
/// initialization error. The current stub is infallible.
pub fn install_file_logger() -> Result<()> {
    Ok(())
}

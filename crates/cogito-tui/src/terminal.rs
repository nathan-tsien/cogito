//! Terminal lifecycle. Three layers of defense ensure the terminal
//! always returns to a sane state (raw mode off, alternate screen
//! left, cursor visible) even on panic or SIGTERM (spec §6.1).
//!
//! Layer 1: RAII drop (normal exit path).
//! Layer 2: panic hook installed before raw mode.
//! Layer 3: SIGTERM/SIGHUP handlers spawned at construction.
//!
//! SIGKILL is unhandleable — documented as a known limitation.

use std::io;

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Restore terminal to non-raw mode with the alternate screen left
/// and the cursor visible. Idempotent and infallible — every restore
/// path calls this.
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// RAII guard. Construct once at startup; `Drop` restores on the
/// happy path. The panic hook and signal handlers cover the rest.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enter raw mode + alternate screen. Installs the panic hook
    /// and signal handlers as side effects.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` if `enable_raw_mode` or
    /// `EnterAlternateScreen` fails. The caller should print a
    /// user-facing error and exit non-zero in that case — the panic
    /// hook is not yet installed, so a normal `?` propagation is fine.
    pub fn new() -> Result<Self> {
        Self::install_panic_hook();
        enable_raw_mode().context("enable_raw_mode")?;
        execute!(io::stdout(), EnterAlternateScreen).context("EnterAlternateScreen")?;
        Self::spawn_signal_handlers();
        Ok(Self)
    }

    fn install_panic_hook() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
    }

    fn spawn_signal_handlers() {
        // Best-effort: ignore signal-stream setup failures (e.g.
        // unsupported platforms). The RAII drop + panic hook still
        // cover normal exits.
        #[cfg(unix)]
        tokio::spawn(async {
            use tokio::signal::unix::{SignalKind, signal};
            // Watch SIGTERM and SIGHUP simultaneously.
            let Ok(mut term) = signal(SignalKind::terminate()) else {
                return;
            };
            let Ok(mut hup) = signal(SignalKind::hangup()) else {
                return;
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = hup.recv() => {}
            }
            restore();
            std::process::exit(130); // 128 + SIGINT-style signal exit code
        });
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_idempotent() {
        // Calling restore() twice when raw mode is already off must
        // not panic. We can't enter raw mode in CI (no real TTY), so
        // we only exercise the cleanup path.
        restore();
        restore();
    }
}

//! Parity test: `cogito-tui --list-strategies` produces the same
//! output as `cogito --list-strategies` (when both point at the
//! same `--config`).

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::path::PathBuf;

fn fixture_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cogito-list-strategies.toml")
}

fn fixture_strategies_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn tui_list_strategies_matches_cli() {
    let mut tui = Command::cargo_bin("cogito-tui").unwrap();
    let tui_out = tui
        .arg("--config")
        .arg(fixture_config())
        .arg("--list-strategies")
        // Point HOME at an empty dir so user-scope scanning is a no-op.
        .env("HOME", tempfile::tempdir().unwrap().path())
        .env("XDG_CONFIG_HOME", tempfile::tempdir().unwrap().path())
        .current_dir(fixture_strategies_dir())
        .output()
        .unwrap();
    assert!(
        tui_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&tui_out.stderr)
    );
    let tui_stdout = String::from_utf8_lossy(&tui_out.stdout).into_owned();

    let mut cli = Command::cargo_bin("cogito").unwrap();
    let cli_out = cli
        .arg("chat")
        .arg("--config")
        .arg(fixture_config())
        .arg("--list-strategies")
        .env("HOME", tempfile::tempdir().unwrap().path())
        .env("XDG_CONFIG_HOME", tempfile::tempdir().unwrap().path())
        .current_dir(fixture_strategies_dir())
        .output()
        .unwrap();
    assert!(
        cli_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&cli_out.stderr)
    );
    let cli_stdout = String::from_utf8_lossy(&cli_out.stdout).into_owned();

    assert_eq!(
        tui_stdout, cli_stdout,
        "TUI and CLI --list-strategies must produce identical output"
    );
    assert!(
        tui_stdout.contains("coder"),
        "expected 'coder' in output: {tui_stdout}"
    );
}

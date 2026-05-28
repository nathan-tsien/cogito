//! cogito-tui binary entrypoint. Phase 1 stub: parses args and exits
//! OK so the workspace compiles. Real entrypoint lands in Task 22.

use anyhow::Result;
use clap::Parser;
use cogito_tui::cli::TuiArgs;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let _args = TuiArgs::parse();
    Ok(())
}

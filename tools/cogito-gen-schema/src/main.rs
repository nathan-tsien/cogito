//! Generate JSON Schema for cogito-protocol public types.
//!
//! Invoked via `just gen-schema`. CI runs `just gen-schema-check` (this
//! tool with `--check`) to enforce drift-free committed schema files.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cogito-gen-schema", about)]
struct Args {
    /// Output path for the generated JSON Schema.
    #[arg(long)]
    output: PathBuf,

    /// If set, compare generated schema against the file at `--output`
    /// and exit non-zero if they differ. Does not write.
    #[arg(long, default_value_t = false)]
    check: bool,
}

fn main() -> Result<()> {
    let _args = Args::parse();
    // Real implementation lands in Task 11 after types are defined.
    anyhow::bail!("not yet implemented — see Plan 2 Task 11")
}

//! Generate JSON Schema for cogito-protocol public types.
//!
//! Invoked via `just gen-schema`. CI runs `just gen-schema-check` to
//! enforce drift-free committed schema files.

// Dev-time CLI tool: writing the success line to stderr is the
// expected UX, and `tracing` is overkill for a one-shot binary.
#![allow(clippy::print_stderr)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cogito_protocol::ConversationEvent;
use schemars::schema_for;

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
    let args = Args::parse();

    let schema = schema_for!(ConversationEvent);
    let generated = serde_json::to_string_pretty(&schema).context("serializing schema to JSON")?;
    // Trailing newline so the file plays well with text-editing tools.
    let generated = format!("{generated}\n");

    if args.check {
        let existing = std::fs::read_to_string(&args.output)
            .with_context(|| format!("reading {}", args.output.display()))?;
        if existing != generated {
            bail!(
                "schema drift detected: {} differs from generated output. \
                 Run `just gen-schema` and commit.",
                args.output.display()
            );
        }
        Ok(())
    } else {
        if let Some(parent) = args.output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir -p {}", parent.display()))?;
        }
        std::fs::write(&args.output, generated.as_bytes())
            .with_context(|| format!("writing {}", args.output.display()))?;
        eprintln!("wrote {}", args.output.display());
        Ok(())
    }
}

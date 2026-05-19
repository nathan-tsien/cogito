//! Write the canonical sample JSONL fixture to its checked-in path.
//!
//! Run via `cargo run -p cogito-test-fixtures --bin write-sample`. The
//! emitted file is `fixtures/sessions/sample-v1.jsonl` next to this
//! crate's Cargo.toml and is committed alongside the source. The
//! companion `fixture_roundtrip` integration test enforces that the
//! checked-in bytes match the in-code builder.

#![allow(clippy::print_stderr)]
// Justification: this is a one-shot developer binary, not a library or
// long-running service. Using `eprintln!` for a single confirmation
// line keeps the script self-contained.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

fn main() -> Result<()> {
    let bytes = cogito_test_fixtures::fixtures::canonical_sample_jsonl();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/sample-v1.jsonl");
    let parent = out
        .parent()
        .ok_or_else(|| anyhow!("output path {} has no parent directory", out.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating fixture dir {}", parent.display()))?;
    std::fs::write(&out, &bytes)
        .with_context(|| format!("writing fixture file {}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

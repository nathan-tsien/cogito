//! Write the canonical sample JSONL fixtures to their checked-in paths.
//!
//! Run via `cargo run -p cogito-test-fixtures --bin write-sample`. The
//! emitted files are `fixtures/sessions/sample-v1.jsonl` (all original
//! variants) and `fixtures/sessions/sample-skill-v1.jsonl` (Sprint 7 skill
//! loader fixture), both committed alongside the source. The companion
//! `fixture_roundtrip` integration test enforces that the checked-in bytes
//! match the in-code builders.

#![allow(clippy::print_stderr)]
// Justification: this is a one-shot developer binary, not a library or
// long-running service. Using `eprintln!` for a single confirmation
// line keeps the script self-contained.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions");
    write_fixture(
        &root.join("sample-v1.jsonl"),
        &cogito_test_fixtures::fixtures::canonical_sample_jsonl(),
    )?;
    write_fixture(
        &root.join("sample-skill-v1.jsonl"),
        &cogito_test_fixtures::fixtures::canonical_skill_jsonl(),
    )?;
    Ok(())
}

fn write_fixture(out: &Path, bytes: &[u8]) -> Result<()> {
    let parent = out
        .parent()
        .ok_or_else(|| anyhow!("output path {} has no parent directory", out.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating fixture dir {}", parent.display()))?;
    std::fs::write(out, bytes)
        .with_context(|| format!("writing fixture file {}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

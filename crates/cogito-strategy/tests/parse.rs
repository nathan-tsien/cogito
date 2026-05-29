//! File-fixture integration tests for `parse_strategy_file`.
//!
//! Drives the parser directly against the fixtures in `tests/fixtures/`
//! to cover happy paths (minimal, full, file-ref body) and every
//! `LoadError` variant the parser can produce.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use cogito_strategy::{LoadError, parse_strategy_file};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn parses_minimal_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_minimal.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_minimal");
    assert_eq!(
        parsed.strategy.system_prompt.trim(),
        "You are a helpful assistant."
    );
    assert!(parsed.provider.is_none());
}

#[test]
fn parses_full_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_full.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_full");
    assert_eq!(parsed.provider.as_deref(), Some("anthropic-default"));
    assert_eq!(parsed.strategy.model_params.model, "claude-opus-4-7");
    assert_eq!(parsed.strategy.model_params.temperature, Some(0.3));
    assert_eq!(parsed.strategy.max_turns, 50);
    assert!(
        parsed.strategy.system_prompt.contains("horizontal rule"),
        "body must survive a body-level `---` line without truncation"
    );
}

#[test]
fn parses_file_ref_strategy() {
    let parsed = parse_strategy_file(&fixture("valid_file_ref.md")).unwrap();
    assert_eq!(parsed.strategy.name, "valid_file_ref");
    assert_eq!(
        parsed.strategy.system_prompt.trim(),
        "You are loaded from a referenced file."
    );
}

#[test]
fn rejects_missing_frontmatter() {
    let err = parse_strategy_file(&fixture("malformed_no_frontmatter.md")).unwrap_err();
    assert!(matches!(err, LoadError::Frontmatter { .. }));
}

#[test]
fn rejects_missing_name_field() {
    let err = parse_strategy_file(&fixture("malformed_no_name.md")).unwrap_err();
    assert!(matches!(err, LoadError::Parse { .. }));
}

#[test]
fn rejects_filename_mismatch() {
    let err = parse_strategy_file(&fixture("mismatched_filename.md")).unwrap_err();
    assert!(matches!(err, LoadError::NameMismatch { .. }));
}

#[test]
fn rejects_empty_prompt() {
    let err = parse_strategy_file(&fixture("empty_prompt.md")).unwrap_err();
    assert!(matches!(err, LoadError::EmptyPrompt { .. }));
}

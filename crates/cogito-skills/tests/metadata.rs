//! Integration tests for `cogito_skills::metadata::parse_skill_md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_skills::metadata::{ParseError, parse_skill_md};

const VALID: &str = r"---
name: invoice-parser
description: Parses invoices into structured JSON.
version: 0.1.0
---

# Invoice parser

Body content here.
";

#[test]
fn parses_required_fields() {
    let p = parse_skill_md(VALID).unwrap();
    assert_eq!(p.name, "invoice-parser");
    assert_eq!(p.description, "Parses invoices into structured JSON.");
    assert_eq!(p.version, Some("0.1.0".into()));
    assert!(p.body.starts_with("# Invoice parser"));
}

#[test]
fn defaults_for_optional_flags() {
    let p = parse_skill_md(VALID).unwrap();
    assert!(!p.disable_model_invocation);
    assert!(p.user_invocable);
}

#[test]
fn rejects_missing_frontmatter() {
    let err = parse_skill_md("no frontmatter here").unwrap_err();
    assert!(matches!(err, ParseError::MissingFrontmatter));
}

#[test]
fn rejects_missing_name() {
    let s = "---\ndescription: x\n---\nbody";
    assert!(matches!(
        parse_skill_md(s).unwrap_err(),
        ParseError::MissingField(_)
    ));
}

#[test]
fn rejects_invalid_name_chars() {
    let s = "---\nname: \"foo bar!\"\ndescription: x\n---\nbody";
    let err = parse_skill_md(s).unwrap_err();
    assert!(matches!(err, ParseError::InvalidName(_)));
}

#[test]
fn description_oversize_is_capped() {
    let long = "x".repeat(2048);
    let s = format!("---\nname: foo\ndescription: \"{long}\"\n---\nbody");
    let p = parse_skill_md(&s).unwrap();
    assert!(p.description.len() <= cogito_skills::metadata::DESCRIPTION_CAP_CHARS);
    // Last char should be the ellipsis sentinel marker.
    assert!(p.description.ends_with('…'));
}

#[test]
fn parses_disable_model_invocation() {
    let s = "---\nname: foo\ndescription: x\ndisable-model-invocation: true\n---\nbody";
    let p = parse_skill_md(s).unwrap();
    assert!(p.disable_model_invocation);
}

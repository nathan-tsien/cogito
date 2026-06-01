//! Integration tests for the `cogito_protocol::skill` value-types
//! (`SkillSource` serde round-trip + struct construction smoke tests).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillSource};
use std::path::PathBuf;

#[test]
fn skill_source_serde_roundtrip_repo() {
    let src = SkillSource::Repo {
        dir: PathBuf::from("/tmp/.cogito/skills/foo"),
    };
    let json = serde_json::to_string(&src).unwrap();
    let parsed: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, src);
    assert!(json.contains("\"kind\":\"repo\""));
}

#[test]
fn skill_source_serde_roundtrip_user() {
    let src = SkillSource::User;
    let json = serde_json::to_string(&src).unwrap();
    assert_eq!(json, r#"{"kind":"user"}"#);
}

#[test]
fn skill_source_serde_roundtrip_plugin() {
    let src = SkillSource::Plugin {
        plugin_id: "acme-tools".into(),
    };
    let json = serde_json::to_string(&src).unwrap();
    let parsed: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, src);
}

#[test]
fn skill_metadata_is_constructible() {
    let m = SkillMetadata {
        name: "invoice-parser".into(),
        description: "Parses invoices".into(),
        source: SkillSource::User,
        disable_model_invocation: false,
        user_invocable: true,
        version: Some("0.1.0".into()),
    };
    assert_eq!(m.name, "invoice-parser");
}

#[test]
fn skill_content_is_constructible() {
    let c = SkillContent {
        name: "x".into(),
        source: SkillSource::User,
        body: "# heading".into(),
        root: None,
    };
    assert_eq!(c.body, "# heading");
}

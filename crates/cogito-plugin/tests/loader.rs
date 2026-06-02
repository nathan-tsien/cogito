#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;

use cogito_plugin::PluginManifest;

#[test]
fn parses_toml_manifest_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("code-review");
    fs::create_dir_all(plugin_dir.join(".cogito-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".cogito-plugin/plugin.toml"),
        "[plugin]\nid = \"code-review\"\nversion = \"0.1.0\"\ndescription = \"x\"\n",
    )
    .unwrap();

    let m = PluginManifest::load_from_dir(&plugin_dir).unwrap();
    assert_eq!(m.id, "code-review");
    assert_eq!(m.version.as_deref(), Some("0.1.0"));
    assert_eq!(m.skills_dir, "skills");
    assert_eq!(m.mcp_file, "mcp.toml");
}

#[test]
fn rejects_id_with_invalid_chars() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("bad");
    fs::create_dir_all(plugin_dir.join(".cogito-plugin")).unwrap();
    // Uppercase + underscore violate the `[a-z0-9-]+` rule (ADR-0021 §1).
    fs::write(
        plugin_dir.join(".cogito-plugin/plugin.toml"),
        "[plugin]\nid = \"Code_Review\"\n",
    )
    .unwrap();

    let err = PluginManifest::load_from_dir(&plugin_dir).unwrap_err();
    assert!(matches!(err, cogito_plugin::PluginError::InvalidId { .. }));
}

#[test]
fn rejects_empty_id() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("empty");
    fs::create_dir_all(plugin_dir.join(".cogito-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".cogito-plugin/plugin.toml"),
        "[plugin]\nid = \"\"\n",
    )
    .unwrap();

    let err = PluginManifest::load_from_dir(&plugin_dir).unwrap_err();
    assert!(matches!(err, cogito_plugin::PluginError::InvalidId { .. }));
}

#[test]
fn falls_back_to_claude_json_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("legacy");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin/plugin.json"),
        "{ \"name\": \"legacy\", \"version\": \"2.0.0\" }",
    )
    .unwrap();

    let m = PluginManifest::load_from_dir(&plugin_dir).unwrap();
    assert_eq!(m.id, "legacy");
    assert_eq!(m.skills_dir, "skills");
}

use cogito_plugin::{ArtifactOverride, PluginEntry, PluginSet};

fn write_plugin(root: &std::path::Path, id: &str, with_mcp: bool, skill: Option<&str>) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join(".cogito-plugin")).unwrap();
    fs::write(
        dir.join(".cogito-plugin/plugin.toml"),
        format!("[plugin]\nid = \"{id}\"\n"),
    )
    .unwrap();
    if let Some(skill_name) = skill {
        let sdir = dir.join("skills").join(skill_name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            format!("---\nname: {skill_name}\ndescription: d\n---\nbody\n"),
        )
        .unwrap();
    }
    if with_mcp {
        fs::write(
            dir.join("mcp.toml"),
            "[[mcp_servers]]\nname = \"github\"\ntransport = \"stdio\"\ncommand = \"echo\"\n",
        )
        .unwrap();
    }
}

#[test]
fn loads_namespaces_and_keeps_skill_root() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "code-review", true, Some("review-rust"));
    let entries = vec![PluginEntry {
        path: "code-review".into(),
        enabled: true,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert_eq!(c.skill_roots.len(), 1);
    assert_eq!(c.skill_roots[0].plugin_id, "code-review");
    assert_eq!(c.mcp_servers.len(), 1);
    assert_eq!(c.mcp_servers[0].name, "code-review:github");
}

#[test]
fn disabled_plugin_contributes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", true, Some("s1"));
    let entries = vec![PluginEntry {
        path: "p1".into(),
        enabled: false,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert!(c.skill_roots.is_empty());
    assert!(c.mcp_servers.is_empty());
}

#[test]
fn artifact_override_disables_one_mcp_server() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", true, Some("s1"));
    let entries = vec![PluginEntry {
        path: "p1".into(),
        enabled: true,
        artifact_overrides: vec![ArtifactOverride {
            plugin: "p1".into(),
            kind: "mcp".into(),
            name: "github".into(),
            enabled: false,
        }],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();
    assert!(c.mcp_servers.is_empty());
    assert_eq!(c.skill_roots.len(), 1);
}

#[test]
fn duplicate_plugin_id_is_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(&tmp.path().join("a"), "dup", false, None);
    write_plugin(&tmp.path().join("b"), "dup", false, None);
    let entries = vec![
        PluginEntry {
            path: "a/dup".into(),
            enabled: true,
            artifact_overrides: vec![],
        },
        PluginEntry {
            path: "b/dup".into(),
            enabled: true,
            artifact_overrides: vec![],
        },
    ];
    let err = PluginSet::load(&entries, tmp.path()).unwrap_err();
    assert!(matches!(
        err,
        cogito_plugin::PluginError::DuplicateId { .. }
    ));
}

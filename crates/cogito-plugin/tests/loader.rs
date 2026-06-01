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

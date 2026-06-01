#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! ADR-0021 acceptance: a local plugin contributes a skill and an MCP
//! server; both are namespaced and foldable. The skill is verified
//! end-to-end through the real `SkillRegistry`. A mid-session "add a
//! second plugin" recomposition (the input `update_session` would receive)
//! is verified at the contribution level.

use std::fs;

use cogito_plugin::{PluginEntry, PluginSet};
use cogito_protocol::skill::SkillProvider;

fn write_plugin(root: &std::path::Path, id: &str, server: &str, skill: &str) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join(".cogito-plugin")).unwrap();
    fs::write(
        dir.join(".cogito-plugin/plugin.toml"),
        format!("[plugin]\nid = \"{id}\"\n"),
    )
    .unwrap();
    let sdir = dir.join("skills").join(skill);
    fs::create_dir_all(&sdir).unwrap();
    fs::write(
        sdir.join("SKILL.md"),
        format!("---\nname: {skill}\ndescription: d\n---\nbody\n"),
    )
    .unwrap();
    fs::write(
        dir.join("mcp.toml"),
        format!(
            "[[mcp_servers]]\nname = \"{server}\"\ntransport = \"stdio\"\ncommand = \"echo\"\n"
        ),
    )
    .unwrap();
}

#[test]
fn plugin_skill_and_mcp_are_contributed_and_namespaced() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "code-review", "github", "review-rust");

    let entries = vec![PluginEntry {
        path: "code-review".into(),
        enabled: true,
        artifact_overrides: vec![],
    }];
    let c = PluginSet::load(&entries, tmp.path()).unwrap();

    let mcp_names: Vec<_> = c.mcp_servers.iter().map(|s| s.name.clone()).collect();
    assert_eq!(mcp_names, vec!["code-review:github"]);

    // Skill reachable end-to-end through the real SkillRegistry (by value).
    let cfg = cogito_skills::discovery::ScanConfig {
        workspace_root: None,
        user_dir: None,
        include_system: false,
        plugin_roots: c.skill_roots,
    };
    let registry = cogito_skills::SkillRegistry::scan(cfg).unwrap();
    assert!(registry.is_registered("code-review:review-rust"));
}

#[test]
fn mid_session_add_appends_second_plugin_mcp() {
    let tmp = tempfile::tempdir().unwrap();
    write_plugin(tmp.path(), "p1", "srv1", "s1");
    write_plugin(tmp.path(), "p2", "srv2", "s2");

    let c1 = PluginSet::load(
        &[PluginEntry {
            path: "p1".into(),
            enabled: true,
            artifact_overrides: vec![],
        }],
        tmp.path(),
    )
    .unwrap();
    let c2 = PluginSet::load(
        &[
            PluginEntry {
                path: "p1".into(),
                enabled: true,
                artifact_overrides: vec![],
            },
            PluginEntry {
                path: "p2".into(),
                enabled: true,
                artifact_overrides: vec![],
            },
        ],
        tmp.path(),
    )
    .unwrap();

    let before: Vec<_> = c1.mcp_servers.iter().map(|s| s.name.clone()).collect();
    let after: Vec<_> = c2.mcp_servers.iter().map(|s| s.name.clone()).collect();
    assert_eq!(before, vec!["p1:srv1"]);
    assert_eq!(after, vec!["p1:srv1", "p2:srv2"]);
}

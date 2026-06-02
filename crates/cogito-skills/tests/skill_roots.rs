//! `SkillProvider::skill_roots()` for `SkillRegistry` (ADR-0032): returns each
//! registered skill's on-disk directory, deduped and sorted, so the Runtime can
//! inject them into `ExecCtx.skill_roots` for the read-class file tools.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Once;

use cogito_protocol::skill::SkillProvider;
use cogito_skills::{ScanConfig, SkillRegistry};

static FIXTURE_INIT: Once = Once::new();

fn fixtures() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    FIXTURE_INIT.call_once(|| {
        let git_dir = root.join(".git");
        std::fs::create_dir_all(&git_dir).expect("create .git fixture dir");
        let head = git_dir.join("HEAD");
        if !head.exists() {
            std::fs::write(&head, "ref: refs/heads/main\n").expect("write .git/HEAD fixture");
        }
    });
    root
}

#[test]
fn skill_roots_returns_each_skill_directory_sorted() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        include_system: false,
        ..Default::default()
    })
    .unwrap();

    let roots = reg.skill_roots();

    let skills_dir = fixtures().join(".cogito").join("skills");
    let mut expected = vec![skills_dir.join("repo-bar"), skills_dir.join("repo-foo")];
    expected.sort();

    assert_eq!(
        roots, expected,
        "skill_roots must list each skill's own dir"
    );
}

#[test]
fn skill_roots_are_deduped() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        include_system: false,
        ..Default::default()
    })
    .unwrap();

    let roots = reg.skill_roots();
    let mut sorted = roots.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        roots.len(),
        sorted.len(),
        "skill_roots must not contain duplicates"
    );
}

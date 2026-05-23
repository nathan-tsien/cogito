//! Integration tests for `cogito_skills::discovery::discover_skills`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Once;

use cogito_protocol::skill::SkillSource;
use cogito_skills::discovery::{ScanConfig, discover_skills};

static FIXTURE_INIT: Once = Once::new();

fn fixtures() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    // Materialize the `.git/HEAD` walk-up marker at test time: git itself
    // refuses to track paths inside a `.git/` directory, so the fixture
    // can't ship the file in version control.
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
fn discovers_repo_and_user_scopes() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let names: Vec<&str> = found.iter().map(|s| s.parsed.name.as_str()).collect();
    assert!(names.contains(&"repo-foo"));
    assert!(names.contains(&"repo-bar"));
    assert!(names.contains(&"user-baz"));
}

#[test]
fn repo_scope_carries_source_repo() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: None,
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let foo = found.iter().find(|s| s.parsed.name == "repo-foo").unwrap();
    matches!(foo.source, SkillSource::Repo { .. });
}

#[test]
fn user_scope_carries_source_user() {
    let cfg = ScanConfig {
        workspace_root: None,
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
    };
    let found = discover_skills(&cfg).unwrap();
    let baz = found.iter().find(|s| s.parsed.name == "user-baz").unwrap();
    assert!(matches!(baz.source, SkillSource::User));
}

#[test]
fn missing_user_dir_is_not_an_error() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(PathBuf::from("/does/not/exist/cogito-skills-test")),
        include_system: false,
    };
    let _ = discover_skills(&cfg).expect("missing user dir is OK");
}

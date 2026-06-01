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
        ..Default::default()
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
        ..Default::default()
    };
    let found = discover_skills(&cfg).unwrap();
    let foo = found.iter().find(|s| s.parsed.name == "repo-foo").unwrap();
    assert!(matches!(foo.source, SkillSource::Repo { .. }));
}

#[test]
fn user_scope_carries_source_user() {
    let cfg = ScanConfig {
        workspace_root: None,
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
        ..Default::default()
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
        ..Default::default()
    };
    let _ = discover_skills(&cfg).expect("missing user dir is OK");
}

use cogito_protocol::skill::SkillProvider;
use cogito_skills::{SkillRegistry, SkillRegistryError};

#[test]
fn registry_build_succeeds() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
        ..Default::default()
    })
    .unwrap();
    assert!(reg.is_registered("repo-foo"));
    assert!(reg.is_registered("user-baz"));
    assert!(!reg.is_registered("nonexistent"));
}

#[test]
fn registry_get_returns_body() {
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: None,
        include_system: false,
        ..Default::default()
    })
    .unwrap();
    let content = reg.get("repo-foo").unwrap();
    assert!(content.body.starts_with("foo body"));
}

#[test]
fn registry_get_carries_skill_own_directory_as_root() {
    // ADR-0029: SkillContent.root must point at the skill's OWN directory
    // (the folder containing SKILL.md), not the workspace root, so the
    // model can resolve relative references in the body (scripts/, etc.).
    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: None,
        include_system: false,
        ..Default::default()
    })
    .unwrap();
    let content = reg.get("repo-foo").unwrap();
    let expected = fixtures().join(".cogito").join("skills").join("repo-foo");
    assert_eq!(content.root, Some(expected));
}

#[test]
fn duplicate_name_in_same_dir_is_fatal() {
    use std::fs;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let skills = tmp.path().join(".cogito").join("skills");
    fs::create_dir_all(skills.join("dup-a")).unwrap();
    fs::create_dir_all(skills.join("dup-b")).unwrap();
    fs::write(
        skills.join("dup-a").join("SKILL.md"),
        "---\nname: dup\ndescription: a\n---\nbody-a",
    )
    .unwrap();
    fs::write(
        skills.join("dup-b").join("SKILL.md"),
        "---\nname: dup\ndescription: b\n---\nbody-b",
    )
    .unwrap();
    // Plant a .git/ so the walk-up stops here:
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let err = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(tmp.path().to_path_buf()),
        user_dir: None,
        include_system: false,
        ..Default::default()
    })
    .unwrap_err();
    assert!(matches!(err, SkillRegistryError::DuplicateName { .. }));
}

#[test]
fn cross_scope_repo_wins_over_user() {
    use std::fs;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let repo_skills = tmp
        .path()
        .join("repo")
        .join(".cogito")
        .join("skills")
        .join("dual");
    let user_skills = tmp
        .path()
        .join("user")
        .join(".cogito")
        .join("skills")
        .join("dual");
    fs::create_dir_all(&repo_skills).unwrap();
    fs::create_dir_all(&user_skills).unwrap();
    fs::write(
        repo_skills.join("SKILL.md"),
        "---\nname: dual\ndescription: from-repo\n---\nrepo body",
    )
    .unwrap();
    fs::write(
        user_skills.join("SKILL.md"),
        "---\nname: dual\ndescription: from-user\n---\nuser body",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("repo").join(".git")).unwrap();

    let reg = SkillRegistry::scan(ScanConfig {
        workspace_root: Some(tmp.path().join("repo")),
        user_dir: Some(tmp.path().join("user").join(".cogito").join("skills")),
        include_system: false,
        ..Default::default()
    })
    .unwrap();
    let dual = reg.get("dual").unwrap();
    assert!(dual.body.starts_with("repo body"));
}

#[test]
fn registry_list_is_sorted_by_name() {
    let cfg = ScanConfig {
        workspace_root: Some(fixtures()),
        user_dir: Some(fixtures().join("user-home").join(".cogito").join("skills")),
        include_system: false,
        ..Default::default()
    };
    let reg = SkillRegistry::scan(cfg).unwrap();
    let names: Vec<String> = reg.list().into_iter().map(|m| m.name).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "list() must be sorted alphabetically");
    assert!(names.len() >= 2, "test depends on at least 2 fixtures");
}

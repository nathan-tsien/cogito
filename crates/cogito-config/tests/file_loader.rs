//! Tests for `FileConfigLoader`: search path, file-not-found, `deny_unknown_fields`,
//! reserved-section tolerance. Uses temp-env for env mutation; `ENV_LOCK`
//! serializes parallel tests since temp-env mutates global env state.
//!
//! Note about `std::env::set_current_dir`: it is safe (not `unsafe`) in std,
//! but it still mutates process-global state. The `ENV_LOCK` mutex covers
//! parallel-test races. If a test panics between `set_current_dir(new)` and
//! `set_current_dir(prev)`, the cwd remains changed — accepted risk for
//! Sprint 4.5 per the plan; tests are expected to pass cleanly.

#![cfg(feature = "file")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_config::{ConfigLoader, FileConfigLoader};
use tempfile::tempdir;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

const SCRUBBED_KEYS: [(&str, Option<&str>); 2] =
    [("COGITO_CONFIG", None), ("XDG_CONFIG_HOME", None)];

#[tokio::test]
async fn no_path_no_env_no_local_returns_empty_partial() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    let xdg = dir.path().to_path_buf();
    let dir_path = dir.path().to_path_buf();

    temp_env::async_with_vars(
        [
            ("COGITO_CONFIG", None::<std::path::PathBuf>),
            ("XDG_CONFIG_HOME", Some(xdg)),
        ],
        async move {
            // Run inside a tempdir to also avoid picking up the workspace's
            // ./cogito.toml (if any).
            let prev = std::env::current_dir().unwrap();
            std::env::set_current_dir(&dir_path).unwrap();

            let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
            let partial = loader.load().await.expect("ok");
            assert!(partial.runtime.is_none());
            assert!(partial.providers.is_none());

            std::env::set_current_dir(prev).unwrap();
        },
    )
    .await;
}

#[tokio::test]
async fn explicit_path_wins() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    let path = dir.path().join("custom.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "/from/explicit"
        "#,
    )
    .unwrap();

    temp_env::async_with_vars(SCRUBBED_KEYS, async move {
        let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
        let partial = loader.load().await.expect("ok");
        let rt = partial.runtime.expect("runtime");
        assert_eq!(
            rt.session_root.as_deref(),
            Some(std::path::Path::new("/from/explicit"))
        );
    })
    .await;
}

#[tokio::test]
async fn cogito_config_env_var_used() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    let path = dir.path().join("env.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            default_model = "from-env-var-path"
        "#,
    )
    .unwrap();

    let path_str = path.clone();
    temp_env::async_with_vars([("COGITO_CONFIG", Some(path_str))], async move {
        let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
        let partial = loader.load().await.expect("ok");
        assert_eq!(
            partial.runtime.unwrap().default_model.as_deref(),
            Some("from-env-var-path")
        );
    })
    .await;
}

#[tokio::test]
async fn local_cogito_toml_used_when_no_explicit_or_env() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("cogito.toml"),
        r#"
            [runtime]
            default_provider = "local"
        "#,
    )
    .unwrap();
    let dir_path = dir.path().to_path_buf();

    temp_env::async_with_vars(SCRUBBED_KEYS, async move {
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir_path).unwrap();

        let loader = FileConfigLoader::resolve::<&str>(None).expect("ok");
        let partial = loader.load().await.expect("ok");
        assert_eq!(
            partial.runtime.unwrap().default_provider.as_deref(),
            Some("local")
        );

        std::env::set_current_dir(prev).unwrap();
    })
    .await;
}

#[tokio::test]
async fn reserved_top_level_section_does_not_error() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            session_root = "./s"

            [[plugins]]
            name = "future"
        "#,
    )
    .unwrap();

    temp_env::async_with_vars(SCRUBBED_KEYS, async move {
        let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
        let partial = loader.load().await.expect("ok");
        assert_eq!(
            partial.runtime.unwrap().session_root.as_deref(),
            Some(std::path::Path::new("./s"))
        );
    })
    .await;
}

#[tokio::test]
async fn unknown_inner_field_errors() {
    let _g = ENV_LOCK.lock().await;
    let dir = tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(
        &path,
        r#"
            [runtime]
            bogus_field = "x"
        "#,
    )
    .unwrap();

    temp_env::async_with_vars(SCRUBBED_KEYS, async move {
        let loader = FileConfigLoader::resolve(Some(&path)).expect("ok");
        let err = loader.load().await.unwrap_err();
        assert!(err.to_string().contains("bogus_field"));
    })
    .await;
}

#[test]
fn parses_skills_section() {
    use cogito_config::types::SkillsConfig;
    let toml_text = r#"
[skills]
enabled = true
user_dir = "/tmp/.cogito/skills"
include_system = false
"#;
    let parsed: cogito_config::types::RuntimeConfigPartial = toml::from_str(toml_text).unwrap();
    let skills: SkillsConfig = parsed.skills.unwrap();
    assert!(skills.enabled);
    assert_eq!(skills.user_dir.as_deref(), Some("/tmp/.cogito/skills"));
    assert!(!skills.include_system);
}

#[test]
fn skills_section_optional() {
    let parsed: cogito_config::types::RuntimeConfigPartial =
        toml::from_str("[provider.default]\nkind = \"anthropic\"").unwrap();
    assert!(parsed.skills.is_none());
}

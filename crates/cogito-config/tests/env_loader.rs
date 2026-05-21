//! Integration tests for `EnvConfigLoader`. Uses temp-env to scope
//! environment mutations to a single test body; `ENV_LOCK` serializes
//! against test parallelism. The lock is `tokio::sync::Mutex` because
//! its guard is held across an `await` boundary (clippy
//! `await_holding_lock` would reject a `std::sync::Mutex` here).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_config::{ConfigLoader, EnvConfigLoader};
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

const ALL_KEYS: [&str; 4] = [
    "COGITO_SESSION_ROOT",
    "COGITO_DEFAULT_PROVIDER",
    "COGITO_DEFAULT_MODEL",
    "COGITO_STRATEGIES_DIR",
];

fn unset_all_pairs() -> Vec<(&'static str, Option<&'static str>)> {
    ALL_KEYS.iter().map(|k| (*k, None::<&str>)).collect()
}

#[tokio::test]
async fn empty_env_yields_empty_partial() {
    let _g = ENV_LOCK.lock().await;
    temp_env::async_with_vars(unset_all_pairs(), async {
        let loader = EnvConfigLoader;
        let p = loader.load().await.expect("ok");
        assert!(p.runtime.is_none());
        assert!(p.providers.is_none());
    })
    .await;
}

#[tokio::test]
async fn cogito_session_root_sets_field() {
    let _g = ENV_LOCK.lock().await;
    let mut vars = unset_all_pairs();
    vars[0] = ("COGITO_SESSION_ROOT", Some("/tmp/cogito-sess"));
    temp_env::async_with_vars(vars, async {
        let p = EnvConfigLoader.load().await.expect("ok");
        let rt = p.runtime.expect("runtime present");
        assert_eq!(
            rt.session_root.as_deref(),
            Some(std::path::Path::new("/tmp/cogito-sess"))
        );
        assert!(rt.default_provider.is_none());
        assert!(rt.default_model.is_none());
        assert!(rt.strategies_dir.is_none());
    })
    .await;
}

#[tokio::test]
async fn all_cogito_vars_set() {
    let _g = ENV_LOCK.lock().await;
    temp_env::async_with_vars(
        [
            ("COGITO_SESSION_ROOT", Some("./s")),
            ("COGITO_DEFAULT_PROVIDER", Some("anthropic-prod")),
            ("COGITO_DEFAULT_MODEL", Some("claude-opus-4-7")),
            ("COGITO_STRATEGIES_DIR", Some("./strats")),
        ],
        async {
            let p = EnvConfigLoader.load().await.expect("ok");
            let rt = p.runtime.expect("runtime present");
            assert_eq!(
                rt.session_root.as_deref(),
                Some(std::path::Path::new("./s"))
            );
            assert_eq!(rt.default_provider.as_deref(), Some("anthropic-prod"));
            assert_eq!(rt.default_model.as_deref(), Some("claude-opus-4-7"));
            assert_eq!(
                rt.strategies_dir.as_deref(),
                Some(std::path::Path::new("./strats"))
            );
        },
    )
    .await;
}

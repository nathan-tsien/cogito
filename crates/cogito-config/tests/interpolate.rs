//! Tests for `${VAR}` / `${VAR:-default}` interpolation. Uses temp-env
//! to scope env mutations to a single test body; `ENV_LOCK` serializes
//! against test parallelism since `temp-env` mutates process env.
//!
//! All tests here are synchronous, so a `std::sync::Mutex<()>` is
//! sufficient — no guard is ever held across an `await` boundary.

#![cfg(feature = "file")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use cogito_config::interpolate::interpolate_value;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn expand_simple_var() {
    let _g = ENV_LOCK.lock().unwrap();
    temp_env::with_vars([("COGITO_TEST_VAR", Some("hello"))], || {
        let raw: toml::Value =
            toml::from_str(r#"key = "prefix-${COGITO_TEST_VAR}-suffix""#).unwrap();
        let expanded = interpolate_value(raw).expect("ok");
        assert_eq!(expanded["key"].as_str(), Some("prefix-hello-suffix"));
    });
}

#[test]
fn default_used_when_unset() {
    let _g = ENV_LOCK.lock().unwrap();
    temp_env::with_vars([("COGITO_NO_SUCH_VAR", None::<&str>)], || {
        let raw: toml::Value =
            toml::from_str(r#"url = "${COGITO_NO_SUCH_VAR:-https://api.example.com}""#).unwrap();
        let expanded = interpolate_value(raw).expect("ok");
        assert_eq!(expanded["url"].as_str(), Some("https://api.example.com"));
    });
}

#[test]
fn missing_var_no_default_errors() {
    let _g = ENV_LOCK.lock().unwrap();
    temp_env::with_vars([("COGITO_REQUIRED", None::<&str>)], || {
        let raw: toml::Value = toml::from_str(r#"k = "${COGITO_REQUIRED}""#).unwrap();
        let err = interpolate_value(raw).unwrap_err();
        assert!(err.to_string().contains("COGITO_REQUIRED"));
    });
}

#[test]
fn no_substitution_for_non_strings() {
    let _g = ENV_LOCK.lock().unwrap();
    let raw: toml::Value = toml::from_str("n = 42\nb = true").unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    assert_eq!(expanded["n"].as_integer(), Some(42));
    assert_eq!(expanded["b"].as_bool(), Some(true));
}

#[test]
fn nested_tables_and_arrays() {
    let _g = ENV_LOCK.lock().unwrap();
    temp_env::with_vars([("COGITO_KEY_A", Some("from-env"))], || {
        let raw: toml::Value = toml::from_str(
            r#"
                [section]
                inner = "${COGITO_KEY_A}"
                [[items]]
                field = "literal"
                [[items]]
                field = "${COGITO_KEY_A:-fallback}"
            "#,
        )
        .unwrap();
        let expanded = interpolate_value(raw).expect("ok");
        assert_eq!(expanded["section"]["inner"].as_str(), Some("from-env"));
        assert_eq!(expanded["items"][0]["field"].as_str(), Some("literal"));
        assert_eq!(expanded["items"][1]["field"].as_str(), Some("from-env"));
    });
}

#[test]
fn literal_dollar_passes_through() {
    let _g = ENV_LOCK.lock().unwrap();
    let raw: toml::Value = toml::from_str(r#"k = "$5 cup of coffee""#).unwrap();
    let expanded = interpolate_value(raw).expect("ok");
    // Lone `$` not followed by `{` is left untouched.
    assert_eq!(expanded["k"].as_str(), Some("$5 cup of coffee"));
}

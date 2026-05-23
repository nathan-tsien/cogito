//! Slash-command parser tests for `/skill <name> [text]` in the CLI REPL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat::{SlashError, parse_slash_skill};
use cogito_protocol::turn_trigger::TurnTrigger;

fn assert_skill(t: &TurnTrigger, names: &[&str], user_text: Option<&str>) {
    if let TurnTrigger::SkillActivation {
        names: got_names,
        user_text: got_text,
    } = t
    {
        let expected: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(got_names, &expected);
        assert_eq!(got_text.as_deref(), user_text);
    } else {
        panic!("expected SkillActivation, got {t:?}");
    }
}

fn registered(names: &[&'static str]) -> impl Fn(&str) -> bool {
    let owned: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    move |n| owned.iter().any(|m| m == n)
}

/// All registered names are user-invocable. Use this for tests that
/// don't care about the `user-invocable` flag.
fn all_invocable(_n: &str) -> bool {
    true
}

#[test]
fn plain_text_returns_user_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("hello world", &r, &all_invocable).unwrap();
    assert!(matches!(t, TurnTrigger::UserText(s) if s == "hello world"));
}

#[test]
fn single_skill_no_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo", &r, &all_invocable).unwrap();
    assert_skill(&t, &["foo"], None);
}

#[test]
fn single_skill_with_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo do this thing", &r, &all_invocable).unwrap();
    assert_skill(&t, &["foo"], Some("do this thing"));
}

#[test]
fn multiple_skills() {
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo bar do X", &r, &all_invocable).unwrap();
    assert_skill(&t, &["foo", "bar"], Some("do X"));
}

#[test]
fn unknown_skill_errors() {
    let r = registered(&["foo"]);
    let err = parse_slash_skill("/skill unknown", &r, &all_invocable).unwrap_err();
    assert!(matches!(err, SlashError::UnknownSkill(ref n) if n == "unknown"));
}

#[test]
fn unknown_after_known_treated_as_text_start() {
    // /skill foo unknown bar -> activate ["foo"], user_text = "unknown bar"
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo unknown bar", &r, &all_invocable).unwrap();
    assert_skill(&t, &["foo"], Some("unknown bar"));
}

#[test]
fn non_user_invocable_first_token_errors() {
    // "blocked" is registered but its frontmatter set user-invocable: false.
    let r = registered(&["foo", "blocked"]);
    let invocable = |n: &str| n != "blocked";
    let err = parse_slash_skill("/skill blocked", &r, &invocable).unwrap_err();
    assert!(matches!(err, SlashError::NotUserInvocable(ref n) if n == "blocked"));
}

#[test]
fn non_user_invocable_first_token_errors_even_with_trailing_text() {
    // Trailing user text doesn't rescue a blocked first token.
    let r = registered(&["blocked"]);
    let invocable = |n: &str| n != "blocked";
    let err = parse_slash_skill("/skill blocked please run", &r, &invocable).unwrap_err();
    assert!(matches!(err, SlashError::NotUserInvocable(ref n) if n == "blocked"));
}

#[test]
fn non_user_invocable_after_invocable_starts_user_text() {
    // "/skill foo blocked rest" — "blocked" terminates name scan and
    // becomes part of user_text. The accepted "foo" is still activated.
    let r = registered(&["foo", "blocked"]);
    let invocable = |n: &str| n != "blocked";
    let t = parse_slash_skill("/skill foo blocked rest", &r, &invocable).unwrap();
    assert_skill(&t, &["foo"], Some("blocked rest"));
}

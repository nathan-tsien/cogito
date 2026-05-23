//! Slash-command parser tests for `/skill <name> [text]` in the CLI REPL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_cli::chat::parse_slash_skill;
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

#[test]
fn plain_text_returns_user_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("hello world", &r).unwrap();
    assert!(matches!(t, TurnTrigger::UserText(s) if s == "hello world"));
}

#[test]
fn single_skill_no_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo", &r).unwrap();
    assert_skill(&t, &["foo"], None);
}

#[test]
fn single_skill_with_text() {
    let r = registered(&["foo"]);
    let t = parse_slash_skill("/skill foo do this thing", &r).unwrap();
    assert_skill(&t, &["foo"], Some("do this thing"));
}

#[test]
fn multiple_skills() {
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo bar do X", &r).unwrap();
    assert_skill(&t, &["foo", "bar"], Some("do X"));
}

#[test]
fn unknown_skill_errors() {
    let r = registered(&["foo"]);
    let err = parse_slash_skill("/skill unknown", &r).unwrap_err();
    assert!(err.to_string().contains("unknown"));
}

#[test]
fn unknown_after_known_treated_as_text_start() {
    // /skill foo unknown bar -> activate ["foo"], user_text = "unknown bar"
    let r = registered(&["foo", "bar"]);
    let t = parse_slash_skill("/skill foo unknown bar", &r).unwrap();
    assert_skill(&t, &["foo"], Some("unknown bar"));
}

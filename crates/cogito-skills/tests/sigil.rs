//! Integration tests for `cogito_skills::sigil::find_sigils_outside_code`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_skills::sigil::{FenceState, SigilHit, find_sigils_outside_code};

fn names(hits: Vec<SigilHit>) -> Vec<String> {
    hits.into_iter().map(|h| h.name).collect()
}

#[test]
fn finds_single_sigil() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "use $invoice-parser please");
    assert_eq!(names(hits), vec!["invoice-parser"]);
}

#[test]
fn ignores_sigil_in_fenced_code() {
    let mut s = FenceState::default();
    let text = "regular text\n```rust\nlet x = $foo;\n```\nback to text";
    let hits = find_sigils_outside_code(&mut s, text);
    assert!(names(hits).is_empty());
}

#[test]
fn ignores_sigil_in_inline_backticks() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "the `$foo` example");
    assert!(names(hits).is_empty());
}

#[test]
fn allows_sigil_with_colon_for_plugin_ns() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "$acme:linter please");
    assert_eq!(names(hits), vec!["acme:linter"]);
}

#[test]
fn streaming_fence_state_persists_across_chunks() {
    let mut s = FenceState::default();
    let _ = find_sigils_outside_code(&mut s, "intro\n```\n");
    // Now inside a fence — $foo on a separate chunk must NOT match.
    let hits = find_sigils_outside_code(&mut s, "let x = $foo;\n");
    assert!(names(hits).is_empty());
    let hits = find_sigils_outside_code(&mut s, "```\nafter fence $bar end");
    assert_eq!(names(hits), vec!["bar"]);
}

#[test]
fn rejects_digit_starting_sigil() {
    let mut s = FenceState::default();
    let hits = find_sigils_outside_code(&mut s, "amount $123");
    assert!(names(hits).is_empty(), "digits cannot start a sigil");
}

#[test]
fn caps_name_length_at_64() {
    let mut s = FenceState::default();
    let long = "a".repeat(80);
    let input = format!("$valid-{long}"); // total > 64
    let hits = find_sigils_outside_code(&mut s, &input);
    // Regex caps body to 63 chars after the leading letter; what's
    // matched is `valid-` + first 57 'a's.
    let only = hits.first().expect("expected one match");
    assert!(only.name.len() <= 64);
}

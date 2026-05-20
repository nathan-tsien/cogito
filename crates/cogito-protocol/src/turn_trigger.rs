//! `TurnTrigger` — what caused a new turn to start. Single source of
//! truth for "what triggered this turn", open-by-extension via
//! `#[non_exhaustive]` per ADR-0007 track B (additive variants do NOT bump
//! `schema_version`). See ADR-0016 for the design rationale and the
//! v0.2 / v0.3 / v0.6 migration plan.

use serde::{Deserialize, Serialize};

/// What caused a new turn to start. Open-by-extension via
/// `#[non_exhaustive]` per ADR-0007 track B: future variants are additive
/// and do NOT bump `schema_version`.
///
/// v0.1 ships exactly one variant. A single-variant `#[non_exhaustive]`
/// enum is intentional: it locks the *shape* of the abstraction so that
/// future variants are additive (Skill, Hook, multimodal user content),
/// even though the enum looks like overkill today.
///
/// Reserved variants (DO NOT add to the enum until the matching
/// consumer lands — adding a variant before its handler exists creates
/// a dead-code path that drifts unverified):
///
/// - `UserContent(Vec<ContentBlock>)` — lands with the v0.2 multimedia
///   ADR + `ContentBlock::{Image, Audio}`. Projection: the per-session
///   loop writes `TurnStarted.user_input = blocks` verbatim.
/// - `SkillInvocation { skill_id: String, args: serde_json::Value }` —
///   lands with the post-v0.3 Skills initiative. Projection: the loop
///   writes `TurnStarted.origin = Skill { skill_id }` and derives
///   `user_input` from `args`.
/// - `HookFired { hook_id: String, payload: serde_json::Value }` —
///   lands with the post-v0.6 Hooks initiative beyond H09's policy
///   gate. Projection: the loop writes
///   `TurnStarted.origin = Hook { hook_id }` and derives `user_input`
///   from `payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnTrigger {
    /// User-typed plain text. The overwhelmingly common case for v0.1.
    UserText(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_serde_roundtrip() -> serde_json::Result<()> {
        let original = TurnTrigger::UserText("hello".into());
        let json = serde_json::to_string(&original)?;
        assert_eq!(json, r#"{"kind":"user_text","data":"hello"}"#);
        let parsed: TurnTrigger = serde_json::from_str(&json)?;
        assert_eq!(parsed, original);
        Ok(())
    }

    #[test]
    fn unknown_kind_fails_to_deserialize() {
        // Until v0.2 lands `UserContent` (or v0.3 lands `SkillInvocation`),
        // unknown `kind` values must fail loudly: TurnTrigger is wire-internal
        // between SessionHandle and the per-session loop, NOT a persisted
        // event-log payload that needs forward-tolerance per ADR-0007.
        let unknown = r#"{"kind":"skill_invocation","data":{"skill_id":"foo"}}"#;
        let result: Result<TurnTrigger, _> = serde_json::from_str(unknown);
        assert!(
            result.is_err(),
            "expected unknown variant to error; got {result:?}"
        );
    }
}

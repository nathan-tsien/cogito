//! `TurnTrigger` — what caused a new turn to start. Single source of
//! truth for "what triggered this turn", open-by-extension via
//! `#[non_exhaustive]` per ADR-0007 track B (additive variants do NOT bump
//! `schema_version`). See ADR-0016 for the design rationale and the
//! v0.2 / v0.3 / v0.6 migration plan.

use serde::{Deserialize, Serialize};

/// What caused a new turn to start. Open-by-extension via
/// `#[non_exhaustive]` per ADR-0007 track B.
///
/// Reserved variants (DO NOT add to the enum until the matching
/// consumer lands):
///
/// - `UserContent(Vec<ContentBlock>)` — lands with v0.2 multimedia ADR.
/// - `HookFired { hook_id, payload }` — lands with post-v0.6 Hook trigger work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnTrigger {
    /// User-typed plain text. The overwhelmingly common case for v0.1.
    UserText(String),

    /// User invoked one or more skills via `/skill <name>` (optionally with
    /// trailing text). `user_text` is the leftover after slash parsing
    /// (`None` when the user typed only `/skill foo`). Both fields can be
    /// non-empty simultaneously (`/skill foo do X`).
    SkillActivation {
        /// Skill names to activate (Repo/User bare names or `<plugin_id>:<name>`).
        names: Vec<String>,
        /// Optional trailing user text that becomes `TurnStarted.user_input`.
        user_text: Option<String>,
    },
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
        let unknown = r#"{"kind":"hook_fired","data":{"hook_id":"x"}}"#;
        let result: Result<TurnTrigger, _> = serde_json::from_str(unknown);
        assert!(
            result.is_err(),
            "expected unknown variant to error; got {result:?}"
        );
    }

    #[test]
    fn skill_activation_serde_roundtrip_no_text() -> serde_json::Result<()> {
        let trigger = TurnTrigger::SkillActivation {
            names: vec!["foo".into(), "bar".into()],
            user_text: None,
        };
        let json = serde_json::to_string(&trigger)?;
        let parsed: TurnTrigger = serde_json::from_str(&json)?;
        assert_eq!(parsed, trigger);
        assert!(json.contains("\"kind\":\"skill_activation\""));
        Ok(())
    }

    #[test]
    fn skill_activation_serde_roundtrip_with_text() -> serde_json::Result<()> {
        let trigger = TurnTrigger::SkillActivation {
            names: vec!["foo".into()],
            user_text: Some("do X".into()),
        };
        let json = serde_json::to_string(&trigger)?;
        let parsed: TurnTrigger = serde_json::from_str(&json)?;
        assert_eq!(parsed, trigger);
        Ok(())
    }
}

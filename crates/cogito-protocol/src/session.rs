//! Session-level metadata recorded once per session as the first
//! `ConversationEvent::SessionStarted` payload.
//!
//! Most fields are optional pass-through metadata supplied by the
//! consumer. Cogito performs no validation or auth on these — they
//! are preserved verbatim for the `SaaS` catalog use case (ADR-0007).

use serde::{Deserialize, Serialize};

/// Session-level metadata. All fields except `cogito_version` are
/// optional / consumer-supplied.
///
/// Note: `Eq` is deliberately not derived because the `extra` field
/// carries `serde_json::Value`, which does not implement `Eq`. This
/// mirrors the rationale in [`crate::content::ContentBlock`].
///
/// `JsonSchema` is derived for schema-gen (Plan 2 Task 11). The `extra`
/// field, a `serde_json::Map<String, serde_json::Value>`, leans on
/// schemars's built-in impls (open object of arbitrary JSON values).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct SessionMeta {
    /// Cogito library version that created this session.
    pub cogito_version: String,

    /// Strategy name (from `HarnessStrategy::name`) selected for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,

    /// Model identifier intended for this session (e.g. `"claude-sonnet-4-6"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Optional consumer-supplied user identifier. Cogito does no auth
    /// on this field — opaque pass-through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    /// Optional consumer-supplied tenant identifier. Cogito propagates
    /// only; enforcement is the consumer's responsibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,

    /// Parent session id when this session is a subagent (ADR-0011).
    /// `Some` => this is a delegated child; `None` => top-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<crate::ids::SessionId>,

    /// The parent turn's `delegate` tool-call id that spawned this child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,

    /// Subagent nesting depth (0 = top-level, 1 = first delegate, ...).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub subagent_depth: u32,

    /// Opaque consumer-supplied metadata; preserved verbatim.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Serde skip predicate: omit `subagent_depth` when it is the 0 default.
/// Takes `&u32` because serde's `skip_serializing_if` requires a `fn(&T) -> bool`
/// signature (hence the `trivially_copy_pass_by_ref` allow).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_meta_roundtrips_with_none_fields_omitted() -> serde_json::Result<()> {
        let meta = SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta)?;
        // Only cogito_version should be serialized; Option fields and empty
        // extra map are skipped.
        assert_eq!(json, r#"{"cogito_version":"0.1.0"}"#);
        let back: SessionMeta = serde_json::from_str(&json)?;
        assert_eq!(meta, back);
        Ok(())
    }

    #[test]
    fn full_meta_roundtrips() -> serde_json::Result<()> {
        let mut extra = serde_json::Map::new();
        extra.insert("source".into(), serde_json::json!("web"));
        let meta = SessionMeta {
            cogito_version: "0.1.0".into(),
            strategy: Some("default".into()),
            model: Some("claude-sonnet-4-6".into()),
            user_id: Some("u_42".into()),
            tenant_id: Some("acme".into()),
            parent_session_id: Some(crate::ids::SessionId::new()),
            parent_call_id: Some("c0".into()),
            subagent_depth: 2,
            extra,
            ..Default::default()
        };
        let json = serde_json::to_string(&meta)?;
        let back: SessionMeta = serde_json::from_str(&json)?;
        assert_eq!(meta, back);
        Ok(())
    }

    #[test]
    fn unknown_fields_in_json_do_not_panic() -> serde_json::Result<()> {
        // Forward-compat: a v0.2 writer may add a field we don't know.
        // serde defaults to ignoring unknowns (no `deny_unknown_fields`).
        let json = r#"{"cogito_version":"0.2.0","brand_new_field":42}"#;
        let meta: SessionMeta = serde_json::from_str(json)?;
        assert_eq!(
            meta.cogito_version, "0.2.0",
            "known field must still decode despite unknown sibling"
        );
        Ok(())
    }

    #[test]
    fn subagent_meta_roundtrips_and_defaults() -> serde_json::Result<()> {
        // Default top-level: new fields absent / zero.
        let base = SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        };
        assert_eq!(base.subagent_depth, 0);
        assert!(base.parent_session_id.is_none());
        let json = serde_json::to_string(&base)?;
        // depth 0 + None parents are omitted.
        assert_eq!(json, r#"{"cogito_version":"0.1.0"}"#);

        // Child: fields populated and round-trip.
        let child = SessionMeta {
            cogito_version: "0.1.0".into(),
            strategy: Some("reviewer".into()),
            parent_session_id: Some(crate::ids::SessionId::new()),
            parent_call_id: Some("c1".into()),
            subagent_depth: 1,
            ..Default::default()
        };
        let back: SessionMeta = serde_json::from_str(&serde_json::to_string(&child)?)?;
        assert_eq!(back, child);
        Ok(())
    }
}

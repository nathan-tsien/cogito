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
/// `JsonSchema` is deliberately not derived in v0.1: the wider
/// schema-gen cascade is owned by Plan 2 Task 11, which will add the
/// derive uniformly across the protocol surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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

    /// Opaque consumer-supplied metadata; preserved verbatim.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
            extra,
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
}

//! `activate_skill` — primary skill-activation channel (ADR-0042). Given a
//! skill name, returns the skill's full SKILL.md body (rendered identically
//! to the sigil/slash injection path) so the model loads instructions via a
//! native tool call rather than a prose sigil. Unknown or
//! `disable-model-invocation` skills return structured errors.

use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::skill::{SkillProvider, render_skill_block};
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Loads a skill body on model request. Holds the injected `SkillProvider`.
#[derive(Clone)]
pub struct ActivateSkill {
    provider: Arc<dyn SkillProvider>,
}

impl ActivateSkill {
    /// Construct from the runtime's skill provider.
    #[must_use]
    pub fn new(provider: Arc<dyn SkillProvider>) -> Self {
        Self { provider }
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    name: String,
}

#[async_trait]
impl BuiltinTool for ActivateSkill {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "activate_skill".into(),
            description: "Load a skill's full instructions into the conversation. Call this before acting whenever a skill listed in the Skills section is relevant to the task. Returns the skill's complete SKILL.md body.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name exactly as listed in the Skills section."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, _ctx: ExecCtx) -> ToolResult {
        let Args { name } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("activate_skill args: {e}"),
                    retryable: false,
                };
            }
        };
        let Some(meta) = self.provider.get_metadata(&name) else {
            let mut available: Vec<String> =
                self.provider.list().into_iter().map(|m| m.name).collect();
            available.sort();
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!(
                    "unknown skill '{name}'; available: {}",
                    available.join(", ")
                ),
                retryable: false,
            };
        };
        if meta.disable_model_invocation {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!(
                    "skill '{name}' is user-invocable only; ask the user to run /skill {name}"
                ),
                retryable: false,
            };
        }
        let Some(content) = self.provider.get(&name) else {
            // Registered in metadata but body unavailable — treat as failure.
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("skill '{name}' has no loadable body"),
                retryable: false,
            };
        };
        ToolResult::text(render_skill_block(&content))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // tests
mod tests {
    use std::sync::Arc;

    use cogito_protocol::ExecCtx;
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};
    use cogito_protocol::tool::{ToolErrorKind, ToolProvider, ToolResult};

    use super::ActivateSkill;
    use crate::provider::{BuiltinTool, BuiltinToolProvider};

    struct FakeProvider {
        metas: Vec<SkillMetadata>,
    }
    impl SkillProvider for FakeProvider {
        fn list(&self) -> Vec<SkillMetadata> {
            self.metas.clone()
        }
        fn get(&self, name: &str) -> Option<SkillContent> {
            self.metas
                .iter()
                .find(|m| m.name == name)
                .map(|m| SkillContent {
                    name: m.name.clone(),
                    source: m.source.clone(),
                    body: format!("BODY for {name}"),
                    root: None,
                })
        }
        fn is_registered(&self, name: &str) -> bool {
            self.metas.iter().any(|m| m.name == name)
        }
    }

    fn meta(name: &str, disable_model: bool) -> SkillMetadata {
        SkillMetadata {
            name: name.into(),
            description: "d".into(),
            source: SkillSource::User,
            disable_model_invocation: disable_model,
            user_invocable: true,
            version: None,
        }
    }

    fn tool() -> ActivateSkill {
        ActivateSkill::new(Arc::new(FakeProvider {
            metas: vec![meta("brainstorming", false), meta("locked", true)],
        }))
    }

    /// `ExecCtx` has no `Default`; construct a minimal open-ended context.
    fn ctx() -> ExecCtx {
        ExecCtx::open_ended(SessionId::new(), TurnId::new())
    }

    #[tokio::test]
    async fn returns_rendered_body_for_known_skill() {
        let r = tool()
            .invoke(serde_json::json!({"name": "brainstorming"}), ctx())
            .await;
        match r {
            ToolResult::Output(blocks) => {
                let s = blocks[0].as_str().unwrap();
                assert!(s.contains("BODY for brainstorming"));
                assert!(s.contains(r#"<skill name="brainstorming""#));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_skill_errors_with_available_list() {
        let r = tool()
            .invoke(serde_json::json!({"name": "nope"}), ctx())
            .await;
        match r {
            ToolResult::Error {
                kind,
                message,
                retryable,
            } => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
                assert!(!retryable);
                assert!(
                    message.contains("brainstorming"),
                    "lists available: {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_args_error_is_invalid_args() {
        let r = tool().invoke(serde_json::json!({"wrong": 1}), ctx()).await;
        assert!(matches!(
            r,
            ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn disable_model_invocation_skill_is_refused() {
        let r = tool()
            .invoke(serde_json::json!({"name": "locked"}), ctx())
            .await;
        match r {
            ToolResult::Error { kind, message, .. } => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
                assert!(message.contains("/skill"), "guides user channel: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// Assert that `BuiltinToolProvider` exposes `activate_skill` in its
    /// descriptor list when the tool is registered. This mirrors the wiring
    /// applied in cogito-cli and cogito-tui when a `SkillProvider` is
    /// present.
    #[test]
    fn activate_skill_is_listed_when_registered() {
        let provider = Arc::new(FakeProvider {
            metas: vec![meta("brainstorming", false)],
        });
        let builtin = BuiltinToolProvider::builder()
            .with_tool(Arc::new(ActivateSkill::new(provider)))
            .build();
        let descriptors = builtin.list();
        let names: Vec<&str> = descriptors.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"activate_skill"),
            "expected activate_skill in {names:?}"
        );
    }
}

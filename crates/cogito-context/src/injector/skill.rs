//! `SkillInjector` — `SystemPromptInjector` impl for the Skill loader.
//!
//! Spec: `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md` §7.

use std::sync::Arc;

use async_trait::async_trait;

use cogito_protocol::context::{ContextError, InjectionInput, SystemPromptInjector};
use cogito_protocol::ids::EventId;
use cogito_protocol::skill::SkillProvider;
use cogito_protocol::store::EventRecorder;

/// Per-skill description character cap for the registry block.
const DESCRIPTION_CAP_CHARS: usize = 1024;

/// `SystemPromptInjector` impl powered by a `SkillProvider`.
#[derive(Clone)]
pub struct SkillInjector {
    provider: Arc<dyn SkillProvider>,
    description_cap_chars: usize,
}

impl SkillInjector {
    /// Construct with default char cap.
    #[must_use]
    pub fn new(provider: Arc<dyn SkillProvider>) -> Self {
        Self {
            provider,
            description_cap_chars: DESCRIPTION_CAP_CHARS,
        }
    }
}

#[async_trait]
impl SystemPromptInjector for SkillInjector {
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError> {
        // Task 12 will fill in candidate collection + dedupe.
        // For Task 11 we emit only the registry block — no activations.
        let suffix = build_registry_block(&*self.provider, self.description_cap_chars);
        let event_id = EventRecorder::record_system_prompt_injected(
            input.recorder,
            input.turn_id,
            suffix,
            Vec::new(),
            "skill",
        )
        .await?;
        Ok(event_id)
    }

    fn id(&self) -> &'static str {
        "skill"
    }
}

fn build_registry_block(provider: &dyn SkillProvider, cap_chars: usize) -> String {
    let metas = provider.list();
    if metas.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Available Skills\n");
    for m in metas {
        let desc = if m.description.chars().count() > cap_chars {
            let mut t: String = m
                .description
                .chars()
                .take(cap_chars.saturating_sub(1))
                .collect();
            t.push('…');
            t
        } else {
            m.description
        };
        out.push_str("- ");
        out.push_str(&m.name);
        out.push_str(": ");
        out.push_str(&desc);
        out.push('\n');
    }
    out
}

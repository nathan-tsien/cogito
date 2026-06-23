//! `SkillInjector` — `SystemPromptInjector` impl for the Skill loader.
//!
//! Spec: `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md` §7.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use cogito_protocol::context::{ContextError, InjectionInput, SystemPromptInjector};
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::ids::{EventId, TurnId};
use cogito_protocol::skill::{
    SkillActivationChannel, SkillProvider, SkillSource, render_skill_block,
};
use cogito_protocol::store::EventRecorder;

/// Per-skill description character cap for the registry block.
const DESCRIPTION_CAP_CHARS: usize = 1024;

/// Max skills listed in the index before truncation (ADR-0042 §6).
const MAX_LISTED_SKILLS: usize = 50;
/// Max total characters in the index block before truncation (ADR-0042 §6).
const MAX_INDEX_CHARS: usize = 8192;

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
        // Idempotency: if a SystemPromptInjected for this turn already exists,
        // return the existing event_id and emit nothing new (resume hit).
        if let Some(eid) = find_existing_injection(input.history, input.turn_id) {
            return Ok(eid);
        }

        // Step 1: collect user-channel names from current turn's TurnStarted.
        let user_names = collect_user_channel(input.history, input.turn_id);

        // Step 2: collect model-channel names from previous turn(s) text.
        let model_names = collect_model_channel(input.history, input.turn_id, &*self.provider);

        // Step 3: dedupe against prior SkillActivated events.
        let prior = collect_prior_activations(input.history);

        let mut seen_this_turn: HashSet<String> = HashSet::new();
        let mut contributors: Vec<String> = Vec::new();
        let mut to_inject: Vec<String> = Vec::new();

        let candidates = user_names
            .into_iter()
            .map(|n| (n, SkillActivationChannel::UserSlash))
            .chain(
                model_names
                    .into_iter()
                    .map(|n| (n, SkillActivationChannel::ModelSigil)),
            );
        for (name, channel) in candidates {
            if prior.contains(&name) {
                continue;
            }
            if !seen_this_turn.insert(name.clone()) {
                continue;
            }
            let Some(content) = self.provider.get(&name) else {
                continue;
            };
            EventRecorder::record_skill_activated(
                input.recorder,
                input.turn_id,
                name.clone(),
                content.source.clone(),
                channel,
            )
            .await?;
            contributors.push(name.clone());
            to_inject.push(name);
        }

        // Step 4: build suffix.
        let registry = build_registry_block(&*self.provider, self.description_cap_chars);
        let bodies = build_body_blocks(&*self.provider, &to_inject);
        let suffix = if registry.is_empty() && bodies.is_empty() {
            String::new()
        } else {
            format!("{registry}{bodies}")
        };

        let event_id = EventRecorder::record_system_prompt_injected(
            input.recorder,
            input.turn_id,
            suffix,
            contributors,
            "skill",
        )
        .await?;
        Ok(event_id)
    }

    fn id(&self) -> &'static str {
        "skill"
    }
}

fn find_existing_injection(history: &[ConversationEvent], turn_id: TurnId) -> Option<EventId> {
    for ev in history {
        if ev.turn_id == Some(turn_id)
            && let EventPayload::SystemPromptInjected { .. } = &ev.payload
        {
            return Some(ev.event_id);
        }
    }
    None
}

fn collect_user_channel(history: &[ConversationEvent], turn_id: TurnId) -> Vec<String> {
    for ev in history {
        if ev.turn_id == Some(turn_id)
            && let EventPayload::TurnStarted {
                activate_skills, ..
            } = &ev.payload
        {
            return activate_skills.clone();
        }
    }
    Vec::new()
}

fn collect_model_channel(
    history: &[ConversationEvent],
    current_turn: TurnId,
    provider: &dyn SkillProvider,
) -> Vec<String> {
    // Sigil scanner lives in cogito-protocol so both Brain (H06) and Hands
    // (this injector) can share the same impl without crossing ADR-0004.
    use cogito_protocol::sigil::{FenceState, find_sigils_outside_code};

    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut hit_current = false;
    for ev in history {
        if ev.turn_id == Some(current_turn) {
            hit_current = true;
        }
        if hit_current {
            continue;
        }
        if let EventPayload::AssistantMessageAppended { text, .. } = &ev.payload {
            // Each AssistantMessageAppended carries one completed text
            // block; H06 resets fence state at TextBlockCompleted, so the
            // projection here must reset per event to match.
            let mut state = FenceState::default();
            for hit in find_sigils_outside_code(&mut state, text) {
                if !provider.is_registered(&hit.name) {
                    continue;
                }
                // Honor SKILL.md `disable-model-invocation: true` — the
                // skill is still listed in the registry block but the
                // sigil channel cannot activate it.
                if provider
                    .get_metadata(&hit.name)
                    .is_some_and(|m| m.disable_model_invocation)
                {
                    continue;
                }
                if seen.insert(hit.name.clone()) {
                    names.push(hit.name);
                }
            }
        }
    }
    names
}

fn collect_prior_activations(history: &[ConversationEvent]) -> HashSet<String> {
    let mut out = HashSet::new();
    for ev in history {
        if let EventPayload::SkillActivated { skill_name, .. } = &ev.payload {
            out.insert(skill_name.clone());
        }
    }
    out
}

/// Sort key giving scope precedence Repo > User > Plugin > System.
fn scope_rank(s: &SkillSource) -> u8 {
    match s {
        SkillSource::Repo { .. } => 0,
        SkillSource::User => 1,
        SkillSource::Plugin { .. } => 2,
        SkillSource::System => 3,
        _ => 4,
    }
}

fn scope_header(s: &SkillSource) -> &'static str {
    match s {
        SkillSource::Repo { .. } => "### From this repository",
        SkillSource::User => "### User",
        SkillSource::Plugin { .. } => "### Plugins",
        SkillSource::System => "### Built-in",
        _ => "### Other",
    }
}

fn build_registry_block(provider: &dyn SkillProvider, cap_chars: usize) -> String {
    let mut metas = provider.list();
    if metas.is_empty() {
        return String::new();
    }
    // Stable sort by scope precedence; preserves discovery order within a scope.
    metas.sort_by_key(|m| scope_rank(&m.source));

    let total = metas.len();
    let dropped = total.saturating_sub(MAX_LISTED_SKILLS);
    if dropped > 0 {
        metas.truncate(MAX_LISTED_SKILLS);
        tracing::warn!(
            dropped,
            total,
            limit = MAX_LISTED_SKILLS,
            "skill index truncated: more skills than the listing cap"
        );
    }

    let mut out = String::from(
        "## Skills (mandatory)\n\
         Before responding, scan the skills below. If any skill is relevant to \
         the user's task — even partially — you MUST load it first by calling the \
         `activate_skill` tool with its name. (If you cannot call tools, write \
         `$<name>` instead.) Loading injects the skill's full instructions.\n\n",
    );

    let mut last_rank: Option<u8> = None;
    for m in metas {
        let rank = scope_rank(&m.source);
        if last_rank != Some(rank) {
            out.push_str(scope_header(&m.source));
            out.push('\n');
            last_rank = Some(rank);
        }
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

    if out.len() > MAX_INDEX_CHARS {
        tracing::warn!(
            chars = out.len(),
            limit = MAX_INDEX_CHARS,
            "skill index block exceeds the char cap; downstream context budget may truncate it"
        );
    }
    out
}

fn build_body_blocks(provider: &dyn SkillProvider, names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    for name in names {
        let Some(content) = provider.get(name) else {
            continue;
        };
        out.push_str(&render_skill_block(&content));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod registry_block_tests {
    use cogito_protocol::skill::{SkillContent, SkillMetadata, SkillProvider, SkillSource};

    use super::{DESCRIPTION_CAP_CHARS, build_registry_block};

    struct P(Vec<SkillMetadata>);
    impl SkillProvider for P {
        fn list(&self) -> Vec<SkillMetadata> {
            self.0.clone()
        }
        fn get(&self, _: &str) -> Option<SkillContent> {
            None
        }
        fn is_registered(&self, n: &str) -> bool {
            self.0.iter().any(|m| m.name == n)
        }
    }
    fn m(name: &str, src: SkillSource) -> SkillMetadata {
        SkillMetadata {
            name: name.into(),
            description: "desc".into(),
            source: src,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }
    }

    #[test]
    fn includes_mandatory_instruction_and_tool_name() {
        let p = P(vec![m("a", SkillSource::User)]);
        let out = build_registry_block(&p, DESCRIPTION_CAP_CHARS);
        assert!(out.contains("## Skills (mandatory)"));
        assert!(out.contains("you MUST"));
        assert!(out.contains("activate_skill"));
        assert!(out.contains("$<name>"), "mentions sigil fallback");
    }

    #[test]
    fn orders_repo_before_user_before_plugin() {
        let p = P(vec![
            m("u", SkillSource::User),
            m("r", SkillSource::Repo { dir: "/x".into() }),
            m(
                "p",
                SkillSource::Plugin {
                    plugin_id: "acme".into(),
                },
            ),
        ]);
        let out = build_registry_block(&p, DESCRIPTION_CAP_CHARS);
        let ri = out.find("- r:").unwrap();
        let ui = out.find("- u:").unwrap();
        let pi = out.find("- p:").unwrap();
        assert!(ri < ui && ui < pi, "repo<user<plugin in:\n{out}");
    }

    #[test]
    fn empty_provider_yields_empty_block() {
        let p = P(vec![]);
        assert!(build_registry_block(&p, DESCRIPTION_CAP_CHARS).is_empty());
    }
}

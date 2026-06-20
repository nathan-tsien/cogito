//! `SkillInjector` — `SystemPromptInjector` impl for the Skill loader.
//!
//! Spec: `docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md` §7.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use cogito_protocol::context::{ContextError, InjectionInput, SystemPromptInjector};
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::ids::{EventId, TurnId};
use cogito_protocol::skill::{SkillActivationChannel, SkillProvider, SkillSource};
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

fn build_body_blocks(provider: &dyn SkillProvider, names: &[String]) -> String {
    use std::fmt::Write as _;

    if names.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    for name in names {
        let Some(content) = provider.get(name) else {
            continue;
        };
        let source_kind = match content.source {
            SkillSource::Repo { .. } => "repo",
            SkillSource::User => "user",
            SkillSource::Plugin { .. } => "plugin",
            SkillSource::System => "system",
            // `SkillSource` is `#[non_exhaustive]`; future variants render as
            // "unknown" until explicit support lands.
            _ => "unknown",
        };
        // Writing into a `String` via `fmt::Write` is infallible; the
        // `Result` can be safely discarded.
        //
        // ADR-0029: when the skill has an on-disk bundle, surface its root
        // directory as a `root="..."` attribute plus a one-line resolution
        // hint, so the model can locate bundled files (`scripts/`,
        // `references/`, `assets/`) referenced relatively in the body.
        // Skills with no on-disk bundle (`root: None`) emit no path.
        //
        // TODO(ADR-0029): the path is interpolated into the pseudo-XML
        // attribute unescaped. Operator-authored skill dirs are trusted in
        // v0.1, but a directory name containing `"`, `>`, or a newline would
        // break the tag and inject text into the system prompt. Escape (or
        // reject at discovery) before skill roots become tenant-controlled
        // in the SaaS profile (Phase 3).
        match content.root.as_deref().map(std::path::Path::display) {
            Some(root) => {
                let _ = write!(
                    out,
                    "\n<skill name=\"{name}\" source=\"{source_kind}\" root=\"{root}\">\n"
                );
                let _ = writeln!(
                    out,
                    "Bundled files for this skill live under the root path above; \
                     resolve any relative path in the instructions below against it."
                );
            }
            None => {
                let _ = write!(out, "\n<skill name=\"{name}\" source=\"{source_kind}\">\n");
            }
        }
        out.push_str(&content.body);
        out.push_str("\n</skill>\n");
    }
    out
}

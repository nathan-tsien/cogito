//! In-process slash command dispatch. v0.1 supports `/skill <name>`
//! only (mirrors `cogito-cli`'s `parse_slash_skill`). Unknown
//! commands push an `[error] unknown command: /foo` notice without
//! going through the model.
//!
//! The parser is intentionally re-implemented here rather than
//! re-exported from `cogito-cli` because the CLI's variant accepts a
//! callback (`F: Fn(...)`) suited to its REPL loop; the TUI just
//! needs the parsed result.

use crate::app::App;

/// Parsed slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/skill <name>` — activate a named skill for the next turn.
    Skill {
        /// Skill name (the token after `/skill `).
        name: String,
    },
    /// An unrecognized `/foo` command.
    Unknown {
        /// The raw text the user typed (including the leading `/`).
        raw: String,
    },
}

/// Parse a slash command. Returns `None` if the input doesn't begin
/// with `/`; the caller should treat that case as a normal user
/// message.
#[must_use]
pub fn parse(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match head {
        "/skill" if !rest.is_empty() => Some(SlashCommand::Skill { name: rest.into() }),
        _ => Some(SlashCommand::Unknown {
            raw: trimmed.into(),
        }),
    }
}

/// Dispatch a parsed slash command against the App. Returns the text
/// (if any) that the App should submit to the model in lieu of a
/// user message — `Some(text)` for skill activation (`/skill foo`
/// becomes the prompt `Activate skill: foo`), `None` for unknowns
/// (already rendered as a notice).
pub fn dispatch(app: &mut App, cmd: SlashCommand) -> Option<String> {
    match cmd {
        SlashCommand::Skill { name } => {
            app.chat.push_notice(format!("[skill] activating: {name}"));
            // The CLI's parse_slash_skill formats the message as
            // "Activate skill: <name>". We mirror that here so the
            // model sees the same prompt.
            Some(format!("Activate skill: {name}"))
        }
        SlashCommand::Unknown { raw } => {
            app.chat
                .push_notice(format!("[error] unknown command: {raw}. Try /skill <name>"));
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_with_name() {
        let r = parse("/skill foo").unwrap();
        assert_eq!(r, SlashCommand::Skill { name: "foo".into() });
    }

    #[test]
    fn parse_skill_without_name_is_unknown() {
        let r = parse("/skill").unwrap();
        assert!(matches!(r, SlashCommand::Unknown { .. }));
    }

    #[test]
    fn parse_unknown_command_returns_unknown() {
        let r = parse("/strategy planner").unwrap();
        assert!(matches!(r, SlashCommand::Unknown { .. }));
    }

    #[test]
    fn parse_non_slash_returns_none() {
        assert!(parse("hello").is_none());
    }

    #[test]
    fn dispatch_skill_returns_activation_prompt() {
        let (mut app, _td) = crate::app::tests::app_for_pure_test();
        let out = dispatch(&mut app, SlashCommand::Skill { name: "foo".into() });
        assert_eq!(out, Some("Activate skill: foo".into()));
        assert_eq!(app.chat.lines.len(), 1);
    }

    #[test]
    fn dispatch_unknown_pushes_error_notice_and_no_prompt() {
        let (mut app, _td) = crate::app::tests::app_for_pure_test();
        let out = dispatch(&mut app, SlashCommand::Unknown { raw: "/foo".into() });
        assert!(out.is_none());
        assert_eq!(app.chat.lines.len(), 1);
        match &app.chat.lines[0] {
            crate::render_model::ChatLine::SystemNotice(s) => {
                assert!(s.contains("unknown command: /foo"));
            }
            _ => unreachable!(),
        }
    }
}

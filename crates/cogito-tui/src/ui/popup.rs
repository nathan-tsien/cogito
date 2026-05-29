//! Slash-command discovery popup. Shown when the input buffer starts
//! with `/`. Prefix-matches against the v0.1 command list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

/// Static catalogue of slash commands available in Sprint 9b. New
/// commands (e.g. `/strategy`, `/help`) land in later sprints.
pub const COMMANDS: &[(&str, &str)] = &[("/skill", "activate a skill by name")];

/// Return commands matching the user's typed prefix (case-insensitive).
#[must_use]
pub fn matches(prefix: &str) -> Vec<(&'static str, &'static str)> {
    let p = prefix.to_lowercase();
    COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&p))
        .copied()
        .collect()
}

/// Render the popup centered above the input bar. `parent` is the
/// full frame area; the popup lays out as a small overlay in the
/// bottom-center.
pub fn render(f: &mut Frame, parent: Rect, prefix: &str) {
    let items: Vec<ListItem<'static>> = matches(prefix)
        .into_iter()
        .map(|(cmd, desc)| {
            ListItem::new(Line::from(vec![
                Span::styled(cmd, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled(desc, Style::default().add_modifier(Modifier::DIM)),
            ]))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "(no matching commands; Esc to dismiss)",
            Style::default().add_modifier(Modifier::DIM),
        )]))]
    } else {
        items
    };

    // border = 2 rows; use try_from to avoid cast_possible_truncation lint.
    let height = (u16::try_from(items.len()).unwrap_or(u16::MAX) + 2).min(parent.height);
    let width = 60.min(parent.width.saturating_sub(4));
    let x = parent.x + (parent.width.saturating_sub(width)) / 2;
    let y = parent
        .y
        .saturating_add(parent.height)
        .saturating_sub(height + 6); // 6 = approx input height; popup hovers above
    let area = Rect {
        x,
        y: y.max(parent.y),
        width,
        height,
    };

    let block = Block::default().borders(Borders::ALL).title("commands");
    let list = List::new(items).block(block);
    f.render_widget(Clear, area);
    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_prefix_skill_returns_skill_entry() {
        let m = matches("/sk");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, "/skill");
    }

    #[test]
    fn matches_bare_slash_returns_all() {
        let m = matches("/");
        assert_eq!(m.len(), COMMANDS.len());
    }

    #[test]
    fn matches_unknown_returns_empty() {
        let m = matches("/zz");
        assert!(m.is_empty());
    }

    #[test]
    fn matches_is_case_insensitive() {
        let m = matches("/SK");
        assert_eq!(m.len(), 1);
    }
}

//! Key -> App action dispatcher. Implements the spec's focus model
//! (decision Q9 = B: implicit focus, no mode toggle):
//!
//! - Typing characters -> input widget
//! - Enter / Shift+Enter -> input widget (send or newline)
//! - PgUp/PgDn -> chat scrollback (no focus required)
//! - Ctrl-Up/Down -> tool-tree selection cursor
//! - Ctrl-Enter -> toggle expansion of selected node
//! - Ctrl-T -> toggle tools pane visibility
//! - Ctrl-C -> cancel turn (with double-tap exit)
//! - Ctrl-D on empty input -> exit
//! - Esc -> dismiss popup (if shown); otherwise no-op
//!
//! The dispatcher returns an `Action` describing what the event loop
//! should do (send message, toggle pane, expand node, quit, ...).
//! Side effects that require async (`cancel_turn`, `submit_user_text`,
//! lazy tool-result lookup) happen in the event loop, not here.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, CTRL_C_EXIT_WINDOW};
use crate::ui::input::InputOutcome;

/// What the event loop should do as a result of one key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// No async side effect required; state has already been mutated.
    None,
    /// Submit the given message as the next user turn.
    SubmitUser(String),
    /// Submit the given slash command for in-process dispatch.
    SubmitSlash(String),
    /// Cancel the current turn (call `SessionHandle::cancel_turn`).
    CancelTurn,
    /// Toggle expansion of `path` — if expanding, also trigger lazy
    /// result lookup via the store.
    ExpandNode {
        /// Tree path being toggled.
        path: crate::render_model::TreePath,
        /// `true` if this transitions to expanded; `false` if
        /// transitioning back to collapsed.
        now_expanded: bool,
    },
    /// Quit the event loop.
    Quit,
}

/// Apply a key event to App state and return the deferred Action (if
/// any) that the event loop must perform asynchronously.
pub fn dispatch(app: &mut App, key: KeyEvent) -> Action {
    // Esc dismisses popups first; any other key while popup is open
    // still goes to the input.
    if matches!(key.code, KeyCode::Esc) && app.popup.is_some() {
        app.popup = None;
        return Action::None;
    }

    // Ctrl-T toggles tools pane.
    if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.show_tools = !app.show_tools;
        return Action::None;
    }

    // Ctrl-C: cancel-or-exit double-tap.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return handle_ctrl_c(app);
    }

    // Ctrl-D: exit if buffer is empty.
    if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if app.input.first_char().is_none() {
            return Action::Quit;
        }
        return Action::None;
    }

    // PgUp/PgDn scroll the chat (always; no focus mode).
    if matches!(key.code, KeyCode::PageUp) {
        app.chat.scroll_offset = app.chat.scroll_offset.saturating_add(5);
        return Action::None;
    }
    if matches!(key.code, KeyCode::PageDown) {
        app.chat.scroll_offset = app.chat.scroll_offset.saturating_sub(5);
        return Action::None;
    }

    // Ctrl-Up / Ctrl-Down navigate tool tree.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Up | KeyCode::Down)
    {
        navigate_tree(app, key.code);
        return Action::None;
    }

    // Ctrl-Enter expands selected node.
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        return expand_selected(app);
    }

    // Default: route to the input widget.
    let outcome = app.input.on_key(key);
    app.refresh_popup();
    match outcome {
        InputOutcome::Consumed => Action::None,
        InputOutcome::Submit(text) => {
            if text.starts_with('/') {
                Action::SubmitSlash(text)
            } else {
                Action::SubmitUser(text)
            }
        }
    }
}

fn handle_ctrl_c(app: &mut App) -> Action {
    // Three states:
    //   1. Turn running -> cancel + arm 2s double-tap window.
    //   2. Turn idle + arm active -> exit.
    //   3. Turn idle + no arm -> arm + hint.
    if app.turn_in_progress {
        app.cancel_seen_at = Some(Instant::now());
        return Action::CancelTurn;
    }
    if let Some(t) = app.cancel_seen_at
        && t.elapsed() < CTRL_C_EXIT_WINDOW
    {
        return Action::Quit;
    }
    app.cancel_seen_at = Some(Instant::now());
    app.chat
        .push_notice("[hint] Press Ctrl-C again to exit, or Ctrl-D on empty input");
    Action::None
}

fn navigate_tree(app: &mut App, code: KeyCode) {
    if app.tools.turns.is_empty() {
        return;
    }
    let cur = app.selected;
    let next = match (cur, code) {
        (None, _) => Some((0, 0)),
        (Some((t, n)), KeyCode::Down) => {
            let group_len = app.tools.turns.get(t).map_or(0, |g| g.nodes.len());
            if n + 1 < group_len {
                Some((t, n + 1))
            } else if t + 1 < app.tools.turns.len() {
                Some((t + 1, 0))
            } else {
                Some((t, n))
            }
        }
        (Some((t, n)), KeyCode::Up) => {
            if n > 0 {
                Some((t, n - 1))
            } else if t > 0 {
                let prev_t = t - 1;
                let prev_len = app.tools.turns[prev_t].nodes.len();
                Some((prev_t, prev_len.saturating_sub(1)))
            } else {
                Some((t, n))
            }
        }
        _ => cur,
    };
    app.selected = next;
}

fn expand_selected(app: &mut App) -> Action {
    let Some(path) = app.selected else {
        return Action::None;
    };
    // Only allow expansion of finished nodes.
    let finished = app
        .tools
        .turns
        .get(path.0)
        .and_then(|g| g.nodes.get(path.1))
        .is_some_and(|n| n.status.is_finished());
    if !finished {
        return Action::None;
    }
    let now_expanded = if app.expanded.contains(&path) {
        app.expanded.remove(&path);
        false
    } else {
        app.expanded.insert(path);
        true
    };
    Action::ExpandNode { path, now_expanded }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cogito_protocol::stream::StreamEvent;
    use serde_json::json;

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn fresh_app() -> (crate::app::App, tempfile::TempDir) {
        crate::app::tests::app_for_pure_test()
    }

    #[test]
    fn ctrl_t_toggles_show_tools() {
        let (mut app, _td) = fresh_app();
        assert!(app.show_tools);
        let a = dispatch(&mut app, k(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::None);
        assert!(!app.show_tools);
    }

    #[test]
    fn ctrl_c_during_turn_returns_cancel() {
        let (mut app, _td) = fresh_app();
        app.turn_in_progress = true;
        let a = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::CancelTurn);
        assert!(app.cancel_seen_at.is_some());
    }

    #[test]
    fn ctrl_c_twice_when_idle_returns_quit() {
        let (mut app, _td) = fresh_app();
        app.turn_in_progress = false;
        let first = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(first, Action::None);
        let second = dispatch(&mut app, k(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(second, Action::Quit);
    }

    #[test]
    fn ctrl_d_on_empty_buffer_quits() {
        let (mut app, _td) = fresh_app();
        let a = dispatch(&mut app, k(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn ctrl_d_with_text_does_not_quit() {
        let (mut app, _td) = fresh_app();
        dispatch(&mut app, k(KeyCode::Char('h'), KeyModifiers::NONE));
        let a = dispatch(&mut app, k(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_ne!(a, Action::Quit);
    }

    #[test]
    fn enter_with_text_returns_submit_user() {
        let (mut app, _td) = fresh_app();
        dispatch(&mut app, k(KeyCode::Char('h'), KeyModifiers::NONE));
        dispatch(&mut app, k(KeyCode::Char('i'), KeyModifiers::NONE));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, Action::SubmitUser("hi".into()));
    }

    #[test]
    fn enter_with_slash_returns_submit_slash() {
        let (mut app, _td) = fresh_app();
        for ch in "/skill foo".chars() {
            dispatch(&mut app, k(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, Action::SubmitSlash("/skill foo".into()));
    }

    #[test]
    fn pgup_increases_scroll_offset() {
        let (mut app, _td) = fresh_app();
        dispatch(&mut app, k(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.chat.scroll_offset, 5);
    }

    #[test]
    fn ctrl_down_initializes_selection() {
        let (mut app, _td) = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        dispatch(&mut app, k(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(app.selected, Some((0, 0)));
    }

    #[test]
    fn ctrl_enter_on_finished_node_expands() {
        let (mut app, _td) = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        app.tools.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c".into(),
            ok: true,
            error_message: None,
        });
        app.selected = Some((0, 0));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(matches!(
            a,
            Action::ExpandNode {
                path: (0, 0),
                now_expanded: true,
                ..
            }
        ));
        assert!(app.expanded.contains(&(0, 0)));
    }

    #[test]
    fn ctrl_enter_on_running_node_is_noop() {
        let (mut app, _td) = fresh_app();
        app.tools.on_event(&StreamEvent::TurnStarted);
        app.tools.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        app.selected = Some((0, 0));
        let a = dispatch(&mut app, k(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(a, Action::None);
        assert!(!app.expanded.contains(&(0, 0)));
    }

    #[test]
    fn esc_dismisses_popup() {
        let (mut app, _td) = fresh_app();
        app.popup = Some(crate::app::Popup::SlashMenu { prefix: "/".into() });
        dispatch(&mut app, k(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.popup.is_none());
    }
}

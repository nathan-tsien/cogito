//! Top-level App state. Single source of truth for every pane.
//!
//! `App` is reconstructible from the JSONL log alone (AGENTS.md
//! rule 3): on `--session <id>` startup, the resume module translates
//! `ConversationEvent`s into `StreamEvent`s and replays them through
//! `apply_stream_event` before the live event loop starts.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cogito_protocol::ConversationStore;
use cogito_protocol::stream::StreamEvent;

use crate::render_model::{ChatModel, ToolTreeModel, TreePath};
use crate::ui::input::InputWidget;
use crate::ui::status::StatusData;

/// Window after a Ctrl-C that already initiated a turn cancellation
/// during which a second Ctrl-C is interpreted as "exit now". Mirrors
/// cogito-cli::chat::CTRL_C_EXIT_WINDOW.
pub const CTRL_C_EXIT_WINDOW: Duration = Duration::from_secs(2);

/// Active popup state. Sprint 9b has just the slash command menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// Slash discovery menu; `prefix` is the input buffer's leading
    /// substring (e.g. `/`, `/s`, `/sk`).
    SlashMenu {
        /// The leading slash-token typed so far (currently just the
        /// first character; richer prefix matching is a v0.2 follow-up).
        prefix: String,
    },
}

/// Top-level App state.
pub struct App {
    /// Session handle that drives the underlying Runtime.
    pub handle: cogito_core::runtime::SessionHandle,
    /// Strategy registry — used for `/strategy` listing (popup),
    /// never for mid-session swap.
    pub registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry>,
    /// Store handle for lazy tool-result lookup (spec §5.3 α.1).
    pub store: Arc<dyn ConversationStore>,
    /// Session id (as a string for status display).
    pub session_id_str: String,
    /// Cwd of the session's JSONL file — used only if the store needs
    /// re-opening.
    pub session_root: Option<PathBuf>,

    /// Chat scrollback model.
    pub chat: ChatModel,
    /// Tool-tree model.
    pub tools: ToolTreeModel,
    /// Currently selected node in the tree.
    pub selected: Option<TreePath>,
    /// Set of expanded nodes (subset of `TreePath`).
    pub expanded: HashSet<TreePath>,

    /// Multi-line input buffer.
    pub input: InputWidget,

    /// Whether the tools pane is visible (`Ctrl-T` toggle).
    pub show_tools: bool,
    /// Active popup, if any.
    pub popup: Option<Popup>,

    /// Status payload (rebuilt every render).
    pub strategy_name: String,
    /// Model id in effect for the session.
    pub model_id: String,
    /// Completed turn counter; increments on `TurnCompleted`.
    pub turn_count: u32,
    /// `true` between `TurnStarted` and `TurnCompleted/Failed/Cancelled/Paused`.
    pub turn_in_progress: bool,

    /// First Ctrl-C of a double-tap window. `None` = next Ctrl-C
    /// cancels the turn (or shows a hint if idle).
    pub cancel_seen_at: Option<Instant>,

    /// Set to `true` by the keymap to end the event loop.
    pub should_quit: bool,
}

impl App {
    /// Apply a `StreamEvent` to both models and update lifecycle flags.
    pub fn apply_stream_event(&mut self, ev: &StreamEvent) {
        self.chat.on_event(ev);
        self.tools.on_event(ev);
        match ev {
            StreamEvent::TurnStarted => self.turn_in_progress = true,
            StreamEvent::TurnCompleted => {
                self.turn_in_progress = false;
                self.turn_count = self.turn_count.saturating_add(1);
            }
            StreamEvent::TurnFailed { .. }
            | StreamEvent::TurnCancelled
            | StreamEvent::TurnPaused => self.turn_in_progress = false,
            _ => {}
        }
    }

    /// Build the status bar payload from current state.
    #[must_use]
    pub fn status_data(&self) -> StatusData {
        StatusData {
            strategy: self.strategy_name.clone(),
            model: self.model_id.clone(),
            session_id: self.session_id_str.clone(),
            turn_count: self.turn_count,
            tools_visible: self.show_tools,
        }
    }

    /// Update popup state based on the input buffer's first character.
    /// Call this after any `on_key` that modifies the input.
    ///
    /// For richer prefix matching we'd want the whole first line; v0.1
    /// keeps it simple — the popup matches just the leading slash token
    /// (one character of context).
    pub fn refresh_popup(&mut self) {
        let first = self.input.first_char();
        self.popup = match first {
            Some('/') => Some(Popup::SlashMenu {
                prefix: String::from('/'),
            }),
            _ => None,
        };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use cogito_store_jsonl::JsonlStore;
    use cogito_test_fixtures::strategy::MapStrategyRegistry;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tempfile::TempDir;

    /// Minimal App builder for unit tests that don't need a real
    /// `SessionHandle`, registry, or store. The tests exercise pure
    /// App methods that never invoke methods on those fields; the
    /// stubs only have to be valid trait objects / structs.
    ///
    /// Returns the `App` together with its owning `TempDir` so the
    /// JSONL store's directory outlives the test scope.
    pub(crate) fn app_for_pure_test() -> (App, TempDir) {
        let tempdir = tempfile::tempdir().unwrap();
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tempdir.path()));
        let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> =
            Arc::new(MapStrategyRegistry::default());
        let handle = cogito_core::runtime::SessionHandle::test_stub();
        let app = App {
            handle,
            registry,
            store,
            session_id_str: "01abc".into(),
            session_root: None,
            chat: ChatModel::new(),
            tools: ToolTreeModel::new(),
            selected: None,
            expanded: HashSet::new(),
            input: InputWidget::new(),
            show_tools: true,
            popup: None,
            strategy_name: "default".into(),
            model_id: "model-x".into(),
            turn_count: 0,
            turn_in_progress: false,
            cancel_seen_at: None,
            should_quit: false,
        };
        (app, tempdir)
    }

    #[test]
    fn turn_started_sets_turn_in_progress() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        assert!(app.turn_in_progress);
    }

    #[test]
    fn turn_completed_clears_flag_and_increments_counter() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        app.apply_stream_event(&StreamEvent::TurnCompleted);
        assert!(!app.turn_in_progress);
        assert_eq!(app.turn_count, 1);
    }

    #[test]
    fn turn_failed_clears_flag_but_does_not_increment() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        app.apply_stream_event(&StreamEvent::TurnFailed { reason: "x".into() });
        assert!(!app.turn_in_progress);
        assert_eq!(app.turn_count, 0);
    }

    #[test]
    fn refresh_popup_shows_slash_menu_when_slash_typed() {
        let (mut app, _td) = app_for_pure_test();
        app.input
            .on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.refresh_popup();
        assert!(matches!(app.popup, Some(Popup::SlashMenu { .. })));
    }

    #[test]
    fn refresh_popup_clears_when_input_is_not_slash() {
        let (mut app, _td) = app_for_pure_test();
        app.input
            .on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.refresh_popup();
        assert!(app.popup.is_none());
    }

    #[test]
    fn status_data_mirrors_app_state() {
        let (app, _td) = app_for_pure_test();
        let s = app.status_data();
        assert_eq!(s.strategy, "default");
        assert_eq!(s.model, "model-x");
        assert!(s.tools_visible);
    }
}

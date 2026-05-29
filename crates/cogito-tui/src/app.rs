//! Top-level App state. Single source of truth for every pane.
//!
//! `App` is reconstructible from the JSONL log alone (AGENTS.md
//! rule 3): on `--session <id>` startup, the resume module translates
//! `ConversationEvent`s into `StreamEvent`s and replays them through
//! `apply_stream_event` before the live event loop starts.

use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cogito_protocol::ConversationStore;
use cogito_protocol::ids::SessionId;
use cogito_protocol::stream::StreamEvent;
use futures::StreamExt as _;

use crate::render_model::{ChatModel, ToolTreeModel, TreePath};
use crate::ui::input::InputWidget;

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
    /// True between `TurnStarted | ToolDispatchEnded` and the next
    /// content event for this turn. Drives the `∴ ⠋` thinking spinner
    /// (spec §"Thinking spinner").
    pub current_turn_thinking: bool,

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
            StreamEvent::TurnStarted => {
                self.turn_in_progress = true;
                self.current_turn_thinking = true;
            }
            StreamEvent::TurnCompleted => {
                self.turn_in_progress = false;
                self.current_turn_thinking = false;
                self.turn_count = self.turn_count.saturating_add(1);
            }
            StreamEvent::TurnFailed { .. }
            | StreamEvent::TurnCancelled
            | StreamEvent::TurnPaused => {
                self.turn_in_progress = false;
                self.current_turn_thinking = false;
            }
            StreamEvent::TextDelta { .. }
            | StreamEvent::ThinkingDelta { .. }
            | StreamEvent::ToolDispatchStarted { .. } => {
                self.current_turn_thinking = false;
            }
            StreamEvent::ToolDispatchEnded { .. } => {
                // Spinner reappears between tool end and next content.
                if self.turn_in_progress {
                    self.current_turn_thinking = true;
                }
            }
            _ => {}
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

    /// Populate the result preview for one tool node by re-reading the
    /// session store. Idempotent — does nothing if the node is still
    /// running or already has a preview. Called by the event loop in
    /// response to `Action::ExpandNode { now_expanded: true }`.
    ///
    /// JSONL backend reads from a local file (sub-ms for typical
    /// sessions), so blocking the UI for the duration of the call is
    /// acceptable. On store error a `[warning]` notice is pushed and
    /// the preview stays empty (the expanded panel renders the
    /// "(loading result...)" placeholder).
    pub async fn populate_result_preview(&mut self, path: crate::render_model::TreePath) {
        let needs_lookup = self
            .tools
            .turns
            .get(path.0)
            .and_then(|g| g.nodes.get(path.1))
            .is_some_and(|n| n.status.is_finished() && n.result_preview.is_none());
        if !needs_lookup {
            return;
        }
        let call_id = match self
            .tools
            .turns
            .get(path.0)
            .and_then(|g| g.nodes.get(path.1))
        {
            Some(n) => n.call_id.clone(),
            None => return,
        };
        // SessionHandle does not expose its `session_id` publicly; parse
        // the canonical ULID string we already keep on the App for the
        // status bar. This is the same value the runtime built from on
        // session open, so a parse failure here means programmer error.
        let session_id = match SessionId::from_str(&self.session_id_str) {
            Ok(id) => id,
            Err(err) => {
                self.chat
                    .push_notice(format!("[warning] invalid session id: {err}"));
                return;
            }
        };
        // Collect the replay stream into a Vec. JSONL replay is local
        // file IO; bounded by the on-disk log size.
        let events: Vec<cogito_protocol::ConversationEvent> = match self
            .store
            .replay(session_id, 0)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(e) => e,
            Err(err) => {
                self.chat
                    .push_notice(format!("[warning] could not read session: {err}"));
                return;
            }
        };
        if let Some(preview) = crate::resume::extract_tool_result(&events, &call_id)
            && let Some(node) = self.tools.find_node_mut(&call_id)
        {
            node.result_preview = Some(preview);
        }
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
            popup: None,
            strategy_name: "default".into(),
            model_id: "model-x".into(),
            turn_count: 0,
            turn_in_progress: false,
            current_turn_thinking: false,
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
    fn turn_started_sets_thinking_and_content_clears_it() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        assert!(app.current_turn_thinking);
        app.apply_stream_event(&StreamEvent::TextDelta { chunk: "hi".into() });
        assert!(!app.current_turn_thinking);
    }

    #[test]
    fn tool_dispatch_ended_rearms_thinking_until_next_content() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        // First content clears the spinner.
        app.apply_stream_event(&StreamEvent::TextDelta {
            chunk: "calling a tool".into(),
        });
        assert!(!app.current_turn_thinking);
        app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: serde_json::json!({}),
        });
        assert!(!app.current_turn_thinking);
        // Tool end while the turn is still running re-arms the spinner.
        app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        });
        assert!(app.current_turn_thinking);
        // Next content event clears it again.
        app.apply_stream_event(&StreamEvent::TextDelta {
            chunk: "done".into(),
        });
        assert!(!app.current_turn_thinking);
    }

    #[test]
    fn turn_terminal_events_clear_thinking() {
        let (mut app, _td) = app_for_pure_test();
        app.apply_stream_event(&StreamEvent::TurnStarted);
        assert!(app.current_turn_thinking);
        app.apply_stream_event(&StreamEvent::TurnCompleted);
        assert!(!app.current_turn_thinking);

        // TurnCancelled also clears it.
        app.apply_stream_event(&StreamEvent::TurnStarted);
        assert!(app.current_turn_thinking);
        app.apply_stream_event(&StreamEvent::TurnCancelled);
        assert!(!app.current_turn_thinking);
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

    #[tokio::test(flavor = "current_thread")]
    async fn populate_result_preview_is_noop_for_running_node() {
        let (mut app, _td) = app_for_pure_test();
        app.tools
            .on_event(&cogito_protocol::stream::StreamEvent::TurnStarted);
        app.tools
            .on_event(&cogito_protocol::stream::StreamEvent::ToolDispatchStarted {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
            });
        // Running; populate must not touch the store.
        app.populate_result_preview((0, 0)).await;
        // The node is still without a preview.
        assert!(app.tools.turns[0].nodes[0].result_preview.is_none());
    }
}

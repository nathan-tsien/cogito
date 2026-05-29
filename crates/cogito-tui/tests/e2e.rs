//! End-to-end test driving the TUI's App through a fake event channel,
//! a `MockModelGateway`, and a `TestBackend`-rendered Terminal. Verifies
//! the full data flow: keystroke -> submit -> model stream -> `ChatModel`
//! mutation -> frame render.
//!
//! This bypasses `event_loop::run` (which owns a real
//! `Terminal<CrosstermBackend>`) and instead exercises the surface
//! piece-by-piece. The dispatcher + App + render pipeline are the
//! interesting parts; the select! glue is trivial.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::keymap::{Action, dispatch};
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::{RenderInputs, render};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

/// Construct an App suitable for E2E tests — stubs out `SessionHandle`
/// (we never call into it) and runs the dispatcher + render pipeline.
///
/// Returns the App together with its owning `TempDir` so the JSONL
/// store's directory outlives the test scope.
fn e2e_app() -> (App, TempDir) {
    let tempdir = tempfile::tempdir().unwrap();
    let store: Arc<dyn cogito_protocol::ConversationStore> =
        Arc::new(cogito_store_jsonl::JsonlStore::new(tempdir.path()));
    let registry: Arc<dyn cogito_protocol::strategy_registry::StrategyRegistry> =
        Arc::new(cogito_test_fixtures::strategy::MapStrategyRegistry::default());
    let handle = cogito_core::runtime::SessionHandle::test_stub();
    let app = App {
        handle,
        registry,
        store,
        session_id_str: "01TEST".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        popup: None,
        strategy_name: "test".into(),
        model_id: "model-x".into(),
        turn_count: 0,
        turn_in_progress: false,
        current_turn_thinking: false,
        cancel_seen_at: None,
        should_quit: false,
    };
    (app, tempdir)
}

/// Render the App into an 80x24 `TestBackend` and return the buffer as
/// a flat string (one row per line). `Buffer` does not implement
/// `Display` in ratatui 0.29, so cell symbols are collected directly.
fn draw(app: &App) -> String {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render(
                f,
                &RenderInputs {
                    chat: &app.chat,
                    tools: &app.tools,
                    selected: app.selected,
                    expanded: &app.expanded,
                    input: &app.input,
                    turn_thinking: app.current_turn_thinking,
                    spinner_tick: 0,
                    popup_prefix: None,
                },
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let width = buf.area().width as usize;
    let cells = buf.content();
    cells
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn typing_and_model_response_render_into_chat() {
    let (mut app, _td) = e2e_app();
    // Simulate user typing "hi" + Enter.
    for ch in "hi".chars() {
        let _ = dispatch(&mut app, key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let action = dispatch(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, Action::SubmitUser("hi".into()));
    // Bypass the (stubbed) session and apply the user prompt manually,
    // mirroring what the event loop does on Action::SubmitUser.
    app.chat.push_user_prompt("hi".into());

    // Simulate model response.
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "hello!".into(),
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);

    let out = draw(&app);
    assert!(out.contains("▸  hi"), "user prompt missing:\n{out}");
    assert!(out.contains("∴  hello!"), "agent text missing:\n{out}");
    assert_eq!(app.turn_count, 1);
    assert!(!app.turn_in_progress);
}

#[test]
fn layout_has_no_tools_pane() {
    let (app, _td) = e2e_app();
    let out = draw(&app);
    assert!(
        !out.contains("tools "),
        "tools pane should be absent:\n{out}"
    );
}

#[test]
fn ctrl_c_during_streaming_emits_cancel_action() {
    let (mut app, _td) = e2e_app();
    app.turn_in_progress = true;
    let a = dispatch(&mut app, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(a, Action::CancelTurn);
}

#[test]
fn slash_unknown_command_renders_error_notice() {
    let (mut app, _td) = e2e_app();
    for ch in "/foo".chars() {
        let _ = dispatch(&mut app, key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let action = dispatch(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, Action::SubmitSlash("/foo".into()));
    // Mirror what the event loop's handle_action does for Action::SubmitSlash.
    let cmd = cogito_tui::slash::parse("/foo").unwrap();
    let prompt = cogito_tui::slash::dispatch(&mut app, cmd);
    assert!(prompt.is_none());
    let out = draw(&app);
    assert!(
        out.contains("unknown command"),
        "missing error notice:\n{out}"
    );
}

#[test]
fn tool_lifecycle_renders_inline() {
    let (mut app, _td) = e2e_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c1".into(),
        tool_name: "read_file".into(),
        args: serde_json::json!({"path": "a.rs"}),
    });
    app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c1".into(),
        ok: true,
        error_message: None,
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);
    let out = draw(&app);
    assert!(out.contains("read_file"), "chat lacking tool name:\n{out}");
    assert!(out.contains('✓'), "chat lacking completed glyph:\n{out}");
}

#[test]
fn typing_thinking_response_shows_spinner_then_clears() {
    let (mut app, _td) = e2e_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    // No content yet -> spinner present.
    let out1 = draw(&app);
    assert!(out1.contains("∴ ⠋"), "spinner missing pre-content:\n{out1}");
    app.apply_stream_event(&StreamEvent::TextDelta { chunk: "hi".into() });
    let out2 = draw(&app);
    assert!(
        !out2.contains("∴ ⠋"),
        "spinner should clear on content:\n{out2}"
    );
    assert!(out2.contains("∴  hi"), "content missing:\n{out2}");
}

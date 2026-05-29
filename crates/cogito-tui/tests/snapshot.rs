//! Visual-state snapshot tests using ratatui's `TestBackend`. These
//! complement the per-widget tests already in the source files by
//! asserting the full composed frame in canonical states.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::{RenderInputs, render};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

/// Construct a minimal App for snapshot tests.
///
/// Returns `(App, TempDir)` so the JSONL store's backing directory
/// outlives the test scope.
fn app() -> (App, TempDir) {
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
        session_id_str: "01abcdef".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        popup: None,
        strategy_name: "coder".into(),
        model_id: "claude-opus-4-7".into(),
        turn_count: 0,
        turn_in_progress: false,
        current_turn_thinking: false,
        cancel_seen_at: None,
        should_quit: false,
    };
    (app, tempdir)
}

/// Render the App into an 80x20 `TestBackend` and return the buffer as
/// a flat string (one row per line). `Buffer` does not implement
/// `Display` in ratatui 0.29, so cell symbols are collected directly.
fn draw(app: &App, popup_prefix: Option<&str>) -> String {
    let backend = TestBackend::new(80, 20);
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
                    popup_prefix,
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

#[test]
fn empty_state_renders_no_tools_pane_and_no_status_bar() {
    let (app, _td) = app();
    let out = draw(&app, None);
    assert!(
        !out.contains("tools "),
        "tools pane should be absent:\n{out}"
    );
    assert!(
        !out.contains("strategy:"),
        "status bar should be absent:\n{out}"
    );
    assert!(
        !out.contains("turn:"),
        "turn counter should be absent:\n{out}"
    );
}

#[test]
fn single_text_turn_renders_user_and_agent_lines() {
    let (mut app, _td) = app();
    app.chat.push_user_prompt("who are you?".into());
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "I am cogito.".into(),
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);
    let out = draw(&app, None);
    assert!(out.contains("▸  who are you?"));
    assert!(out.contains("∴  I am cogito."));
}

#[test]
fn popup_overlays_when_prefix_set() {
    let (app, _td) = app();
    let out = draw(&app, Some("/"));
    assert!(out.contains("commands"));
    assert!(out.contains("/skill"));
}

#[test]
fn chat_uses_full_width() {
    let (mut app, _td) = app();
    app.chat.push_user_prompt("test".into());
    let out = draw(&app, None);
    assert!(out.contains("▸  test"));
    // No tool pane separator at 30% mark.
}

#[test]
fn mcp_banner_lines_render_at_top_of_chat() {
    let (mut app, _td) = app();
    app.chat
        .push_notice("[mcp] \u{2713} filesystem ready (4 tools)".to_string());
    let out = draw(&app, None);
    assert!(
        out.contains("filesystem ready"),
        "mcp banner missing:\n{out}"
    );
}

#[test]
fn startup_banner_renders_three_lines_with_sigil_and_identity() {
    let (mut app, _td) = app();
    // Mimic what runtime_build does at build time: Art -> push_banner, Meta -> push_notice.
    for line in cogito_tui::ui::banner::startup_lines("opus-4.7", "coder", "01abcdefghij") {
        match line {
            cogito_tui::ui::banner::BannerLine::Art(s) => app.chat.push_banner(s),
            cogito_tui::ui::banner::BannerLine::Meta(s) => app.chat.push_notice(s),
        }
    }
    let out = draw(&app, None);
    assert!(
        out.contains("\u{2234}\u{2234}\u{2234}"),
        "sigil missing:\n{out}"
    );
    assert!(out.contains("cogito"));
    assert!(out.contains("opus-4.7"));
    assert!(out.contains("coder"));
    assert!(out.contains("01abcdef"));
    assert!(!out.contains("01abcdefghij"));
}

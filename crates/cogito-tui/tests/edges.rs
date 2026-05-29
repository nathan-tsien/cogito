//! Edge cases from spec §7.6:
//!   - Terminal resize mid-stream
//!   - Extremely long single-line input (10k chars)
//!   - Unicode in tool args (CJK + emoji)
//!   - Deep tool tree (50+ calls in one turn)

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;

use cogito_protocol::stream::StreamEvent;
use cogito_tui::app::App;
use cogito_tui::keymap::dispatch;
use cogito_tui::render_model::{ChatModel, ToolTreeModel};
use cogito_tui::ui::input::InputWidget;
use cogito_tui::ui::{RenderInputs, render};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

/// Construct a minimal App for edge-case tests.
///
/// Returns `(App, TempDir)` so the JSONL store's backing directory
/// outlives the test scope.
fn fresh_app() -> (App, TempDir) {
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
        session_id_str: "01".into(),
        session_root: None,
        chat: ChatModel::new(),
        tools: ToolTreeModel::new(),
        selected: None,
        expanded: HashSet::new(),
        input: InputWidget::new(),
        popup: None,
        strategy_name: "x".into(),
        model_id: "m".into(),
        turn_count: 0,
        turn_in_progress: false,
        current_turn_thinking: false,
        cancel_seen_at: None,
        should_quit: false,
    };
    (app, tempdir)
}

/// Render the App into a `TestBackend` of the given dimensions and
/// return the buffer as a flat string.
///
/// `Buffer` does not implement `Display` in ratatui 0.29, so cell
/// symbols are collected directly from the buffer's content.
fn draw(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
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

#[test]
fn resize_mid_stream_does_not_lose_content() {
    let (mut app, _td) = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::TextDelta {
        chunk: "before resize".into(),
    });
    // Render at one size, then another. Content must appear in both.
    let small = draw(&app, 40, 10);
    assert!(small.contains("∴  before resize"), "small render:\n{small}");
    let big = draw(&app, 120, 30);
    assert!(big.contains("∴  before resize"), "big render:\n{big}");
}

#[test]
fn extremely_long_input_does_not_panic() {
    let (mut app, _td) = fresh_app();
    // 10k-char paste via successive char events. tui-textarea must
    // accept; render must not crash.
    let blob = "x".repeat(10_000);
    for ch in blob.chars() {
        let _ = dispatch(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    let out = draw(&app, 80, 24);
    // We don't assert the full buffer round-trips visually — the
    // assertion is "no panic, frame renders". The 'x' rune should
    // appear at least once.
    assert!(out.contains('x'), "expected at least one rendered 'x'");
}

#[test]
fn unicode_in_tool_args_renders_without_corruption() {
    let (mut app, _td) = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c1".into(),
        tool_name: "q".into(),
        args: serde_json::json!({"keyword": "深圳 🌟"}),
    });
    app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c1".into(),
        ok: true,
        error_message: None,
    });
    app.apply_stream_event(&StreamEvent::TurnCompleted);
    // Tools render inline now; the args are only shown when the tool
    // block is expanded. Expand the (only) tool block via Alt+1.
    dispatch(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
    );
    let out = draw(&app, 80, 24);
    // Wide characters (CJK, emoji) each occupy two terminal cells. The
    // cell-symbol collector puts a space in the trailing placeholder
    // cell, so "深圳" appears as "深 圳" in the flat buffer string.
    // Assert each character individually rather than the combined string.
    assert!(out.contains('深'), "CJK character '深' missing:\n{out}");
    assert!(out.contains('圳'), "CJK character '圳' missing:\n{out}");
    assert!(out.contains("🌟"), "emoji missing:\n{out}");
}

#[test]
fn deep_tool_tree_renders_without_panic() {
    let (mut app, _td) = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    for i in 0..60 {
        let call_id = format!("c{i}");
        app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
            call_id: call_id.clone(),
            tool_name: format!("t{i}"),
            args: serde_json::json!({}),
        });
        app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
            call_id,
            ok: true,
            error_message: None,
        });
    }
    assert_eq!(app.tools.total_nodes(), 60);
    let out = draw(&app, 80, 24);
    // 60 inline tool blocks render as 60 chat lines. Even at a 24-row
    // terminal, the first node must be visible and the render must not
    // have panicked. No turn header any more (single-column layout).
    assert!(out.contains("t0"), "first node missing:\n{out}");
}

#[test]
fn quick_expand_via_digit_one_works_after_tool_completes() {
    let (mut app, _td) = fresh_app();
    app.apply_stream_event(&StreamEvent::TurnStarted);
    app.apply_stream_event(&StreamEvent::ToolDispatchStarted {
        call_id: "c".into(),
        tool_name: "read_file".into(),
        args: serde_json::json!({"path": "x.rs"}),
    });
    app.apply_stream_event(&StreamEvent::ToolDispatchEnded {
        call_id: "c".into(),
        ok: true,
        error_message: None,
    });
    // Quick-expand the most recent tool block. Bare digits now route to
    // the input as text; quick-expand is Alt+1..9.
    dispatch(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
    );
    let out = draw(&app, 80, 24);
    assert!(out.contains('▾'), "expansion glyph missing:\n{out}");
    assert!(out.contains("args"), "args row missing:\n{out}");
    assert!(out.contains("path"), "args content missing:\n{out}");
}

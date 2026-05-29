//! Event loop — single tokio task that multiplexes:
//!   1. crossterm keyboard events (via `EventStream`)
//!   2. `StreamEvent` broadcast from `SessionHandle::subscribe()`
//!   3. 33ms redraw tick (<=30 FPS)
//!
//! Drawing happens only on tick. Key handling and stream-event
//! handling mutate App state without redrawing.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CrosstermEvent, EventStream};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::App;
use crate::keymap::{Action, dispatch};
use crate::slash;
use crate::ui::{RenderInputs, render};

/// Drive the TUI to completion. Returns when the user quits, the
/// session closes, or a fatal error occurs.
///
/// The terminal must already be in raw mode (the caller owns the
/// `TerminalGuard`).
///
/// # Errors
///
/// Returns I/O errors from `Terminal::draw` or `CrosstermBackend`.
pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("constructing CrosstermBackend")?;
    let mut crossterm_events = EventStream::new();
    let mut stream_events = BroadcastStream::new(app.handle.subscribe());
    let mut redraw_tick = tokio::time::interval(Duration::from_millis(33));
    redraw_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut spinner_tick: u64 = 0;

    // Initial draw so the user sees a frame immediately.
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
                    spinner_tick,
                    popup_prefix: popup_prefix(app.popup.as_ref()).as_deref(),
                },
            );
        })
        .context("initial draw")?;

    loop {
        tokio::select! {
            maybe_key = crossterm_events.next() => {
                // Resize events trigger a redraw on the next tick; no
                // app-state change required for them.
                if let Some(Ok(CrosstermEvent::Key(key))) = maybe_key {
                    let action = dispatch(app, key);
                    handle_action(app, action).await?;
                }
                if app.should_quit { break; }
            }
            maybe_ev = stream_events.next() => {
                match maybe_ev {
                    Some(Ok(ev)) => app.apply_stream_event(&ev),
                    Some(Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n))) => {
                        app.chat.push_notice(format!(
                            "[warning] {n} events dropped (slow render); live view will catch up"
                        ));
                    }
                    None => {
                        // Broadcast closed -> session ended. Push a notice and quit gracefully.
                        app.chat.push_notice("[fatal] session closed");
                        break;
                    }
                }
            }
            _ = redraw_tick.tick() => {
                spinner_tick = spinner_tick.wrapping_add(1);
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
                                spinner_tick,
                                popup_prefix: popup_prefix(app.popup.as_ref()).as_deref(),
                            },
                        );
                    })
                    .context("draw on tick")?;
            }
        }
    }
    Ok(())
}

fn popup_prefix(popup: Option<&crate::app::Popup>) -> Option<String> {
    popup.map(|p| match p {
        crate::app::Popup::SlashMenu { prefix } => prefix.clone(),
    })
}

async fn handle_action(app: &mut App, action: Action) -> Result<()> {
    match action {
        Action::None
        | Action::ExpandNode {
            now_expanded: false,
            ..
        }
        | Action::ExpandAllInLatestMessage
        | Action::CollapseAllInLatestMessage => Ok(()),
        Action::Quit => {
            app.should_quit = true;
            Ok(())
        }
        Action::CancelTurn => {
            if let Err(err) = app.handle.cancel_turn().await {
                app.chat
                    .push_notice(format!("[warning] cancel failed: {err}"));
            }
            Ok(())
        }
        Action::SubmitUser(text) => {
            app.chat.push_user_prompt(text.clone());
            if let Err(err) = app.handle.submit_user_text(text).await {
                app.chat
                    .push_notice(format!("[error] failed to send: {err}"));
            }
            Ok(())
        }
        Action::SubmitSlash(raw) => {
            let parsed = slash::parse(&raw);
            if let Some(cmd) = parsed
                && let Some(prompt) = slash::dispatch(app, cmd)
                && let Err(err) = app.handle.submit_user_text(prompt).await
            {
                app.chat
                    .push_notice(format!("[error] failed to send: {err}"));
            }
            Ok(())
        }
        Action::ExpandNode {
            path,
            now_expanded: true,
        } => {
            app.populate_result_preview(path).await;
            Ok(())
        }
    }
}

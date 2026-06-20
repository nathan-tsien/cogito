//! Resume support. On startup with `--session <id>`, the JSONL log
//! is read in full and translated into the equivalent `StreamEvent`
//! sequence. The App's `apply_stream_event` then drives the same
//! `ChatModel` + `ToolTreeModel` paths used at live time — no separate
//! "replay" code path in the models (spec §4.6 invariant).

use std::sync::Arc;

use anyhow::Result;
use cogito_protocol::{
    ConversationEvent, ConversationStore, EventPayload, ids::SessionId, stream::StreamEvent,
};
use futures::StreamExt as _;

/// Initial state derived from a session log. `Fresh` for a brand-new
/// session (no replay); `Replayed` for resumes (translated events to
/// drive into App at startup).
pub enum InitialState {
    /// New session, nothing to replay.
    Fresh,
    /// Resumed session — translated stream of events to apply to the
    /// App's models before entering the live loop.
    Replayed {
        /// Events in arrival order.
        stream_events: Vec<StreamEvent>,
    },
}

/// Translate a persisted `ConversationEvent` stream into a stream of
/// `StreamEvent`s suitable for driving `ChatModel` + `ToolTreeModel`.
///
/// The mapping is coarse (one logical block = one synthetic
/// `TextDelta` with the whole text), since the persisted log is in
/// completed-block form, not delta form. This is intentional: replay
/// shows the user the finished content, not a re-played token-by-token
/// stream.
#[must_use]
pub fn translate_events(events: &[ConversationEvent]) -> Vec<StreamEvent> {
    let mut out: Vec<StreamEvent> = Vec::new();
    let mut in_turn = false;
    for ev in events {
        match &ev.payload {
            EventPayload::TurnStarted { .. } => {
                if in_turn {
                    out.push(StreamEvent::TurnCompleted {
                        // Replay reconstruction does not thread the terminal
                        // stop_reason (ADR-0040); read the adjacent
                        // ModelCallCompleted if the flag is needed on replay.
                        stop_reason: None,
                        subagent_call_id: None,
                        // This synthetic close belongs to the previous turn,
                        // whose id is not threaded through this loop.
                        turn_id: None,
                    });
                }
                out.push(StreamEvent::TurnStarted {
                    subagent_call_id: None,
                    turn_id: ev.turn_id,
                });
                in_turn = true;
            }
            EventPayload::TurnCompleted { .. } => {
                out.push(StreamEvent::TurnCompleted {
                    stop_reason: None,
                    subagent_call_id: None,
                    turn_id: ev.turn_id,
                });
                in_turn = false;
            }
            EventPayload::TurnFailed { reason } => {
                out.push(StreamEvent::TurnFailed {
                    reason: format!("{reason:?}"),
                    subagent_call_id: None,
                    turn_id: ev.turn_id,
                });
                in_turn = false;
            }
            EventPayload::AssistantMessageAppended { text, .. } => {
                if !text.is_empty() {
                    out.push(StreamEvent::TextDelta {
                        chunk: text.clone(),
                        subagent_call_id: None,
                        turn_id: ev.turn_id,
                        message_id: None,
                    });
                }
            }
            EventPayload::ThinkingBlockRecorded { text, .. } => {
                if !text.is_empty() {
                    out.push(StreamEvent::ThinkingDelta {
                        chunk: text.clone(),
                        turn_id: ev.turn_id,
                        message_id: None,
                    });
                }
            }
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
                ..
            } => {
                out.push(StreamEvent::ToolDispatchStarted {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                    turn_id: ev.turn_id,
                    message_id: None,
                });
                // We don't know the end status without scanning forward for the
                // matching ToolResultRecorded; emit a synthetic "ok" marker. If
                // a ToolResultRecorded is encountered later in translate_events,
                // the arm below corrects the status.
                out.push(StreamEvent::ToolDispatchEnded {
                    call_id: call_id.clone(),
                    ok: true,
                    error_message: None,
                    turn_id: ev.turn_id,
                    message_id: None,
                });
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                // Re-emit a corrective end event whose ok bit reflects whether
                // the result is an error. The first synthetic end was ok=true;
                // for an errored result we push a second end to flip the status
                // in ToolTreeModel. ChatModel ignores ToolDispatchEnded
                // entirely, so this adds no chat line.
                if matches!(result, cogito_protocol::ToolResult::Error { .. }) {
                    out.push(StreamEvent::ToolDispatchEnded {
                        call_id: call_id.clone(),
                        ok: false,
                        error_message: None,
                        turn_id: ev.turn_id,
                        message_id: None,
                    });
                }
            }
            _ => {}
        }
    }
    if in_turn {
        out.push(StreamEvent::TurnCompleted {
            stop_reason: None,
            subagent_call_id: None,
            // Loop ended without an explicit TurnCompleted event; no
            // event-scoped turn id is in scope for this synthetic close.
            turn_id: None,
        });
    }
    out
}

/// Read the session log and produce an `InitialState`. Errors propagate
/// — caller should print them and exit non-zero before entering raw mode.
///
/// # Errors
///
/// Returns an error if the session log cannot be read from the store.
pub async fn load_initial_state(
    store: &Arc<dyn ConversationStore>,
    session_id: &SessionId,
    is_new_session: bool,
) -> Result<InitialState> {
    if is_new_session {
        return Ok(InitialState::Fresh);
    }
    // Collect all events from the store. replay(session_id, 0) returns events
    // with seq > 0, which skips the SessionStarted event (seq=0). That event
    // carries no TUI-relevant content and falls into the wildcard arm of
    // translate_events, so this is acceptable for replay purposes.
    let events: Vec<ConversationEvent> = store
        .replay(*session_id, 0)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("store replay error: {e}"))?;
    let stream_events = translate_events(&events);
    Ok(InitialState::Replayed { stream_events })
}

// -- Lazy tool-result lookup (spec §5.3 α.1) ----------------------------

/// Walk a `ConversationEvent` log and find the result text for one
/// `call_id`. Returns the first `EventPayload::ToolResultRecorded`
/// whose `call_id` matches, rendered as a single string.
///
/// For `ToolResult::Output(values)` the JSON string values are joined
/// with `\n`; non-string values are JSON-encoded so the user sees
/// something. `ToolResult::Error { message, .. }` returns the message.
/// Returns `Some("<no text content>")` for empty content, and `None`
/// when no matching event is found.
#[must_use]
pub fn extract_tool_result(events: &[ConversationEvent], call_id: &str) -> Option<String> {
    for ev in events {
        if let EventPayload::ToolResultRecorded {
            call_id: cid,
            result,
        } = &ev.payload
            && cid == call_id
        {
            let text = render_tool_result(result);
            if text.is_empty() {
                return Some("<no text content>".into());
            }
            return Some(text);
        }
    }
    None
}

/// Render a `ToolResult` as a single human-readable string for the
/// tool-tree preview pane. v0.1 `Output` carries `Vec<serde_json::Value>`;
/// string values are emitted as-is, non-string values are JSON-encoded.
///
/// `ToolResult` is `#[non_exhaustive]`, so a wildcard arm guards against
/// future variants (e.g. a v0.2 `Streaming` classification).
fn render_tool_result(result: &cogito_protocol::ToolResult) -> String {
    match result {
        cogito_protocol::ToolResult::Output(values) => values
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        cogito_protocol::ToolResult::Error { message, .. } => message.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::{
        SCHEMA_VERSION,
        ids::{EventId, SessionId},
        turn::TurnOutcome,
    };

    fn ev(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: None,
            seq: 0,
            ts: chrono::Utc::now(),
            payload,
        }
    }

    #[test]
    fn empty_log_translates_to_empty_stream() {
        assert!(translate_events(&[]).is_empty());
    }

    #[test]
    fn turn_started_and_completed_translate_directly() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                user_input: vec![],
                activate_skills: vec![],
            }),
            ev(EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            }),
        ];
        let s = translate_events(&log);
        assert!(matches!(s[0], StreamEvent::TurnStarted { .. }));
        assert!(matches!(s[1], StreamEvent::TurnCompleted { .. }));
    }

    #[test]
    fn text_block_yields_text_delta() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                user_input: vec![],
                activate_skills: vec![],
            }),
            ev(EventPayload::AssistantMessageAppended {
                text: "hello".into(),
                message_id: None,
            }),
            ev(EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            }),
        ];
        let s = translate_events(&log);
        match &s[1] {
            StreamEvent::TextDelta { chunk, .. } => assert_eq!(chunk, "hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_yields_dispatch_started_and_ended() {
        let log = vec![
            ev(EventPayload::TurnStarted {
                user_input: vec![],
                activate_skills: vec![],
            }),
            ev(EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                message_id: None,
            }),
            ev(EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            }),
        ];
        let s = translate_events(&log);
        assert!(s.iter().any(|e| matches!(
            e,
            StreamEvent::ToolDispatchStarted { call_id, .. } if call_id == "c1"
        )));
        assert!(s.iter().any(|e| matches!(
            e,
            StreamEvent::ToolDispatchEnded { call_id, ok: true, .. } if call_id == "c1"
        )));
    }

    #[test]
    fn unterminated_turn_emits_synthetic_completed() {
        // A log that ends mid-turn (e.g. crash during streaming)
        // must still leave the chat in a coherent state.
        let log = vec![ev(EventPayload::TurnStarted {
            user_input: vec![],
            activate_skills: vec![],
        })];
        let s = translate_events(&log);
        assert!(
            s.last()
                .is_some_and(|e| matches!(e, StreamEvent::TurnCompleted { .. }))
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod extract_tests {
    use super::*;
    use cogito_protocol::{
        SCHEMA_VERSION,
        ids::{EventId, SessionId},
        tool::ToolResult,
    };

    fn ev(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: None,
            seq: 0,
            ts: chrono::Utc::now(),
            payload,
        }
    }

    #[test]
    fn extract_returns_text_content() {
        let log = vec![ev(EventPayload::ToolResultRecorded {
            call_id: "c1".into(),
            result: ToolResult::text("file contents"),
        })];
        let r = extract_tool_result(&log, "c1");
        assert_eq!(r, Some("file contents".into()));
    }

    #[test]
    fn extract_returns_none_when_call_id_not_found() {
        let log = vec![ev(EventPayload::ToolResultRecorded {
            call_id: "c1".into(),
            result: ToolResult::text("anything"),
        })];
        assert_eq!(extract_tool_result(&log, "c-other"), None);
    }

    #[test]
    fn extract_empty_content_returns_placeholder() {
        let log = vec![ev(EventPayload::ToolResultRecorded {
            call_id: "c1".into(),
            result: ToolResult::Output(vec![]),
        })];
        assert_eq!(
            extract_tool_result(&log, "c1"),
            Some("<no text content>".into())
        );
    }
}

//! `StreamEvent` serde stability tests.

use cogito_protocol::stream::StreamEvent;

#[test]
fn text_delta_round_trips() -> serde_json::Result<()> {
    let event = StreamEvent::TextDelta {
        chunk: "Hello ".into(),
    };
    let json = serde_json::to_string(&event)?;
    let back: StreamEvent = serde_json::from_str(&json)?;
    assert_eq!(event, back);
    Ok(())
}

#[test]
fn lifecycle_events_round_trip() -> serde_json::Result<()> {
    let events = [
        StreamEvent::TurnStarted,
        StreamEvent::TurnPaused,
        StreamEvent::TurnResumed,
        StreamEvent::TurnCancelled,
        StreamEvent::TurnCompleted,
        StreamEvent::TurnFailed {
            reason: "model gateway timeout".into(),
        },
        StreamEvent::ToolDispatchStarted {
            call_id: "call_1".into(),
            tool_name: "read_file".into(),
        },
        StreamEvent::ToolDispatchEnded {
            call_id: "call_1".into(),
            ok: true,
        },
    ];
    for e in events {
        let json = serde_json::to_string(&e)?;
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(e, back);
    }
    Ok(())
}

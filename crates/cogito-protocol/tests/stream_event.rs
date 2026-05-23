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
            args: serde_json::json!({"path": "src/main.rs"}),
        },
        StreamEvent::ToolDispatchEnded {
            call_id: "call_1".into(),
            ok: true,
            error_message: None,
        },
        StreamEvent::ToolDispatchEnded {
            call_id: "call_2".into(),
            ok: false,
            error_message: Some("permission denied".into()),
        },
    ];
    for e in events {
        let json = serde_json::to_string(&e)?;
        let back: StreamEvent = serde_json::from_str(&json)?;
        assert_eq!(e, back);
    }
    Ok(())
}

#[test]
#[allow(clippy::unwrap_used)]
fn skill_activation_requested_serde_roundtrip() {
    use cogito_protocol::stream::StreamEvent;
    let ev = StreamEvent::SkillActivationRequested {
        skill_name: "invoice-parser".into(),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains("\"kind\":\"skill_activation_requested\""));
    let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, ev);
}

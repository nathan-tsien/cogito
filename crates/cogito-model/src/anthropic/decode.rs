//! SSE event → `ModelEvent`. Adapter buffers per-block partial text + JSON
//! and emits sealed `*Completed` events on `content_block_stop`.

use std::collections::HashMap;

use cogito_protocol::gateway::{ModelError, ModelEvent, StopReason, Usage};

use super::wire::{
    SseContentBlockDelta, SseContentBlockStart, SseEvent, SseMessageDelta, SseUsage,
};
use crate::error::wire;

/// Per-stream decoder state. One instance per `stream()` call.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    /// Accumulated text per text block.
    text_buf: HashMap<u32, String>,
    /// Accumulated partial JSON per `tool_use` block.
    tool_args_buf: HashMap<u32, ToolUseAccum>,
    /// Final usage from `message_delta`.
    usage: Usage,
    /// Final stop reason.
    stop_reason: Option<StopReason>,
}

#[derive(Debug, Clone)]
struct ToolUseAccum {
    call_id: String,
    tool_name: String,
    partial_json: String,
}

impl Decoder {
    /// Create a fresh decoder for a new streaming call.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Translate one SSE event into zero or more `ModelEvent`s.
    ///
    /// `Ping` and unknown event types yield an empty vector.
    pub(crate) fn translate(
        &mut self,
        sse: SseEvent,
    ) -> Result<Vec<ModelEvent>, ModelError> {
        match sse {
            SseEvent::MessageStart { message } => {
                self.usage = into_usage(message.usage);
                Ok(vec![])
            }
            SseEvent::Ping => Ok(vec![]),
            SseEvent::ContentBlockStart { index, content_block } => {
                match content_block {
                    SseContentBlockStart::Text { text } => {
                        self.text_buf.insert(index, text);
                        Ok(vec![])
                    }
                    SseContentBlockStart::ToolUse { id, name, input } => {
                        // `input` is typically `{}`; partial_json deltas follow.
                        let starting_json =
                            if input.is_null() || input == serde_json::json!({}) {
                                String::new()
                            } else {
                                input.to_string()
                            };
                        self.tool_args_buf.insert(
                            index,
                            ToolUseAccum {
                                call_id: id.clone(),
                                tool_name: name.clone(),
                                partial_json: starting_json,
                            },
                        );
                        Ok(vec![ModelEvent::ToolUseStarted {
                            block_index: index,
                            call_id: id,
                            tool_name: name,
                        }])
                    }
                }
            }
            SseEvent::ContentBlockDelta { index, delta } => match delta {
                SseContentBlockDelta::TextDelta { text } => {
                    self.text_buf.entry(index).or_default().push_str(&text);
                    Ok(vec![ModelEvent::TextDelta {
                        block_index: index,
                        chunk: text,
                    }])
                }
                SseContentBlockDelta::InputJsonDelta { partial_json } => {
                    if let Some(acc) = self.tool_args_buf.get_mut(&index) {
                        acc.partial_json.push_str(&partial_json);
                    }
                    Ok(vec![])
                }
            },
            SseEvent::ContentBlockStop { index } => {
                if let Some(text) = self.text_buf.remove(&index) {
                    return Ok(vec![ModelEvent::TextBlockCompleted {
                        block_index: index,
                        text,
                    }]);
                }
                if let Some(acc) = self.tool_args_buf.remove(&index) {
                    let parsed: serde_json::Value = if acc.partial_json.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(&acc.partial_json)
                            .unwrap_or(serde_json::Value::Null)
                    };
                    return Ok(vec![ModelEvent::ToolUseCompleted {
                        block_index: index,
                        call_id: acc.call_id,
                        tool_name: acc.tool_name,
                        args: parsed,
                    }]);
                }
                Ok(vec![])
            }
            SseEvent::MessageDelta { delta, usage } => {
                let SseMessageDelta { stop_reason } = delta;
                if let Some(s) = stop_reason {
                    self.stop_reason = Some(parse_stop_reason(&s));
                }
                // Anthropic sends a cumulative usage update here.
                self.usage = into_usage(usage);
                Ok(vec![])
            }
            SseEvent::MessageStop => {
                let stop_reason = self.stop_reason.unwrap_or(StopReason::EndTurn);
                let usage = std::mem::take(&mut self.usage);
                Ok(vec![ModelEvent::MessageCompleted { stop_reason, usage }])
            }
            SseEvent::Error { error } => Err(wire::decode(format!(
                "anthropic SSE error: {}",
                error.message
            ))),
        }
    }
}

fn into_usage(u: SseUsage) -> Usage {
    Usage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
    }
}

fn parse_stop_reason(s: &str) -> StopReason {
    match s {
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        // "end_turn" and any unknown values default to EndTurn.
        _ => StopReason::EndTurn,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    #[allow(clippy::wildcard_imports)]
    use super::*;
    use crate::anthropic::wire::{SseContentBlockStart, SseMessageStart, SseUsage};

    #[test]
    fn translate_message_start_yields_no_events() {
        let mut dec = Decoder::new();
        let events = dec
            .translate(SseEvent::MessageStart {
                message: SseMessageStart {
                    usage: SseUsage {
                        input_tokens: 10,
                        output_tokens: 0,
                    },
                },
            })
            .expect("translate succeeds");
        assert!(events.is_empty());
        assert_eq!(dec.usage.input_tokens, 10);
    }

    #[test]
    fn translate_text_block_accumulates_and_completes() {
        let mut dec = Decoder::new();

        // Start a text block at index 0.
        let started = dec
            .translate(SseEvent::ContentBlockStart {
                index: 0,
                content_block: SseContentBlockStart::Text {
                    text: String::new(),
                },
            })
            .expect("start ok");
        assert!(started.is_empty());

        // Two deltas.
        let d1 = dec
            .translate(SseEvent::ContentBlockDelta {
                index: 0,
                delta: crate::anthropic::wire::SseContentBlockDelta::TextDelta {
                    text: "Hello".into(),
                },
            })
            .expect("delta 1 ok");
        assert_eq!(d1.len(), 1);
        assert!(matches!(
            &d1[0],
            ModelEvent::TextDelta { chunk, .. } if chunk == "Hello"
        ));

        dec.translate(SseEvent::ContentBlockDelta {
            index: 0,
            delta: crate::anthropic::wire::SseContentBlockDelta::TextDelta {
                text: ", world!".into(),
            },
        })
        .expect("delta 2 ok");

        // Stop seals the block.
        let stopped = dec
            .translate(SseEvent::ContentBlockStop { index: 0 })
            .expect("stop ok");
        assert_eq!(stopped.len(), 1);
        assert!(matches!(
            &stopped[0],
            ModelEvent::TextBlockCompleted { text, .. } if text == "Hello, world!"
        ));
    }
}

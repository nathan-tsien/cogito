//! Stream chunk → `ModelEvent` for `OpenAI` Chat Completions.
//!
//! Key challenge: `OpenAI` does not emit per-tool-call boundary events.
//! All `tool_calls` accumulate via `delta.tool_calls[i]` until
//! `finish_reason: "tool_calls"` fires; the decoder emits one
//! `ToolUseStarted` per new call id+name pair, accumulates partial
//! `arguments`, and emits one `ToolUseCompleted` per buffered call when
//! `finish_reason` arrives.
//!
//! Text accumulation: deltas are forwarded as `TextDelta` events immediately
//! for live-streaming UI, and also buffered so that the final
//! `TextBlockCompleted` carries the complete text without requiring H06 to
//! maintain its own accumulator.

use std::collections::BTreeMap;

use cogito_protocol::gateway::{ModelError, ModelEvent, StopReason, Usage};

use super::wire::{Choice, StreamChunk};

/// Per-stream decoder state.  One instance per `stream()` call.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    /// Accumulated text for block 0 (the single text block).
    text_buf: String,
    /// Whether the text block has been opened (first content delta seen).
    text_started: bool,
    /// Tool-call buffer keyed by stream-level index.
    /// `block_index` is synthesised: text is always 0; tools are 1..N in
    /// stream-index order.
    tool_calls: BTreeMap<u32, ToolCallBuf>,
    /// Next `block_index` to assign for `tool_use` blocks (starts at 1).
    next_tool_block: u32,
}

#[derive(Debug, Default)]
struct ToolCallBuf {
    /// Synthesised `block_index`; `None` until both `call_id` and `name` are
    /// known and `ToolUseStarted` has been emitted.
    block_index: Option<u32>,
    /// Opaque id assigned by the model (present in first fragment).
    call_id: Option<String>,
    /// Tool name (present in first fragment).
    name: Option<String>,
    /// Accumulated partial JSON for the arguments.
    arguments: String,
}

impl Decoder {
    /// Create a fresh decoder for a new streaming call.
    pub(crate) fn new() -> Self {
        Self {
            next_tool_block: 1,
            ..Self::default()
        }
    }

    /// Translate one streaming JSON chunk into zero or more `ModelEvent`s.
    ///
    /// Returns `Result` for API consistency with future fallible decode paths
    /// (e.g. strict argument validation in v0.2+).
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn translate(&mut self, chunk: StreamChunk) -> Result<Vec<ModelEvent>, ModelError> {
        let mut out = Vec::new();
        for choice in chunk.choices {
            self.translate_choice(choice, &mut out);
        }
        Ok(out)
    }

    fn translate_choice(&mut self, choice: Choice, out: &mut Vec<ModelEvent>) {
        let Choice {
            delta,
            finish_reason,
        } = choice;

        // --- Text delta ---
        if let Some(text) = delta.content {
            if !self.text_started {
                self.text_started = true;
            }
            if !text.is_empty() {
                self.text_buf.push_str(&text);
                out.push(ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: text,
                });
            }
        }

        // --- Tool-call fragments ---
        for tc in delta.tool_calls {
            let buf = self.tool_calls.entry(tc.index).or_default();
            if let Some(id) = tc.id {
                buf.call_id = Some(id);
            }
            if let Some(fun) = tc.function {
                if let Some(n) = fun.name {
                    buf.name = Some(n);
                }
                if let Some(a) = fun.arguments {
                    buf.arguments.push_str(&a);
                }
            }
            // Emit `ToolUseStarted` as soon as we know both call_id and name.
            // Guard against empty strings: some providers (e.g. SenseNova) emit
            // extra `tool_calls` delta fragments with index > 0 that carry
            // empty-string id/name — treat those as incomplete and ignore them.
            if buf.block_index.is_none() {
                if let (Some(id), Some(name)) = (buf.call_id.as_ref(), buf.name.as_ref()) {
                    if !id.is_empty() && !name.is_empty() {
                        let block_index = self.next_tool_block;
                        self.next_tool_block += 1;
                        buf.block_index = Some(block_index);
                        out.push(ModelEvent::ToolUseStarted {
                            block_index,
                            call_id: id.clone(),
                            tool_name: name.clone(),
                        });
                    }
                }
            }
        }

        // --- Finish reason seals all buffered blocks ---
        if let Some(reason) = finish_reason {
            // Seal the text block first if any text arrived.
            if self.text_started {
                let text = std::mem::take(&mut self.text_buf);
                out.push(ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text,
                });
                self.text_started = false;
            }

            // Seal every buffered tool call in stream-index order.
            let calls: Vec<_> = std::mem::take(&mut self.tool_calls).into_iter().collect();
            for (_idx, buf) in calls {
                if let (Some(block_index), Some(call_id), Some(name)) =
                    (buf.block_index, buf.call_id, buf.name)
                {
                    let args = if buf.arguments.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(&buf.arguments).unwrap_or(serde_json::Value::Null)
                    };
                    out.push(ModelEvent::ToolUseCompleted {
                        block_index,
                        call_id,
                        tool_name: name,
                        args,
                    });
                }
            }

            // Token usage: `OpenAI`-compat may include it in a non-delta final
            // chunk; v0.1 accepts zeroes here.
            let usage = Usage::default();
            let stop_reason = parse_finish_reason(&reason);
            out.push(ModelEvent::MessageCompleted { stop_reason, usage });
        }
    }
}

/// Map an `OpenAI` `finish_reason` string to a cogito `StopReason`.
///
/// Case-insensitive; `"stop"`, `"end_turn"`, and unknown values fall back
/// to `EndTurn`.
fn parse_finish_reason(s: &str) -> StopReason {
    match s.to_ascii_lowercase().as_str() {
        "tool_calls" | "tool_use" => StopReason::ToolUse,
        "length" | "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        // "stop", "end_turn", and any unrecognised values map to EndTurn.
        _ => StopReason::EndTurn,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::openai_compat::wire::{
        Choice, ChoiceDelta, StreamChunk, ToolCallDelta, ToolCallFunctionDelta,
    };

    fn make_chunk(content: Option<&str>, finish: Option<&str>) -> StreamChunk {
        StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: content.map(String::from),
                    tool_calls: vec![],
                },
                finish_reason: finish.map(String::from),
            }],
        }
    }

    #[test]
    fn text_only_accumulates_and_completes() {
        let mut dec = Decoder::new();

        // First chunk — role hint, empty content.
        let e0 = dec
            .translate(StreamChunk {
                choices: vec![Choice {
                    delta: ChoiceDelta {
                        role: Some("assistant".into()),
                        content: Some(String::new()),
                        tool_calls: vec![],
                    },
                    finish_reason: None,
                }],
            })
            .expect("chunk 0 ok");
        assert!(e0.is_empty(), "empty content should not emit TextDelta");

        let e1 = dec
            .translate(make_chunk(Some("Hello"), None))
            .expect("chunk 1 ok");
        assert_eq!(e1.len(), 1);
        assert!(matches!(&e1[0], ModelEvent::TextDelta { chunk, .. } if chunk == "Hello"));

        let e2 = dec
            .translate(make_chunk(Some(", world!"), None))
            .expect("chunk 2 ok");
        assert_eq!(e2.len(), 1);

        // Finish chunk — seals text block.
        let e3 = dec
            .translate(make_chunk(None, Some("stop")))
            .expect("finish ok");
        assert_eq!(e3.len(), 2, "TextBlockCompleted + MessageCompleted");
        assert!(matches!(
            &e3[0],
            ModelEvent::TextBlockCompleted { text, .. } if text == "Hello, world!"
        ));
        assert!(matches!(
            &e3[1],
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
    }

    #[test]
    fn tool_use_started_on_first_id_and_name() {
        let mut dec = Decoder::new();

        let chunk = StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: None,
                    tool_calls: vec![ToolCallDelta {
                        index: 0,
                        id: Some("call_abc".into()),
                        function: Some(ToolCallFunctionDelta {
                            name: Some("read_file".into()),
                            arguments: Some(String::new()),
                        }),
                    }],
                },
                finish_reason: None,
            }],
        };
        let events = dec.translate(chunk).expect("ok");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ModelEvent::ToolUseStarted { call_id, tool_name, .. }
                if call_id == "call_abc" && tool_name == "read_file"
        ));
    }

    #[test]
    fn tool_use_completed_on_finish_reason_tool_calls() {
        let mut dec = Decoder::new();

        // First fragment — id + name.
        dec.translate(StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: None,
                    tool_calls: vec![ToolCallDelta {
                        index: 0,
                        id: Some("call_x".into()),
                        function: Some(ToolCallFunctionDelta {
                            name: Some("list_dir".into()),
                            arguments: Some("{\"path\":".into()),
                        }),
                    }],
                },
                finish_reason: None,
            }],
        })
        .expect("fragment 1");

        // Second fragment — more args.
        dec.translate(StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: None,
                    tool_calls: vec![ToolCallDelta {
                        index: 0,
                        id: None,
                        function: Some(ToolCallFunctionDelta {
                            name: None,
                            arguments: Some("\"/tmp\"}".into()),
                        }),
                    }],
                },
                finish_reason: None,
            }],
        })
        .expect("fragment 2");

        // Finish — tool_calls reason.
        let finish_events = dec
            .translate(StreamChunk {
                choices: vec![Choice {
                    delta: ChoiceDelta::default(),
                    finish_reason: Some("tool_calls".into()),
                }],
            })
            .expect("finish");

        let completed = finish_events
            .iter()
            .find_map(|e| match e {
                ModelEvent::ToolUseCompleted {
                    call_id,
                    tool_name,
                    args,
                    ..
                } => Some((call_id.clone(), tool_name.clone(), args.clone())),
                _ => None,
            })
            .expect("ToolUseCompleted present");

        assert_eq!(completed.0, "call_x");
        assert_eq!(completed.1, "list_dir");
        assert_eq!(completed.2, serde_json::json!({"path": "/tmp"}));
    }

    #[test]
    fn parse_finish_reason_case_insensitive() {
        assert_eq!(parse_finish_reason("STOP"), StopReason::EndTurn);
        assert_eq!(parse_finish_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(parse_finish_reason("LENGTH"), StopReason::MaxTokens);
        assert_eq!(parse_finish_reason("unknown_value"), StopReason::EndTurn);
    }
}

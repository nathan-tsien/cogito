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
///
/// **Lifecycle:** the gateway calls [`Decoder::translate`] once per
/// SSE chunk (emits live `TextDelta` / `ToolUseStarted` events) and
/// then [`Decoder::finalize`] exactly once when the stream closes
/// (emits the terminal `TextBlockCompleted` / `ToolUseCompleted` /
/// `MessageCompleted` events with the fully-accumulated text).
///
/// Why finalize-on-stream-end instead of seal-on-`finish_reason`: some
/// providers (observed: `SenseNova`) emit `finish_reason: "stop"` on
/// *every* chunk rather than only the final one. Sealing on the first
/// `finish_reason` would (a) cause multi-hundred-fold store write
/// amplification and (b) — worse — truncate the persisted assistant
/// message to whatever text arrived in the first finish-bearing chunk
/// while the live `TextDelta` stream continued to deliver the rest.
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
    /// Latest `finish_reason` seen across all chunks. Consumed by
    /// `finalize` to produce the terminal `MessageCompleted::stop_reason`.
    /// `None` is acceptable — `finalize` falls back to `EndTurn`.
    finish_reason: Option<String>,
    /// Whether `finalize` has already run. Idempotent on repeated calls.
    finalized: bool,
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

        // --- Finish reason — capture only; sealing happens in finalize ---
        // Repeated finish_reason values across chunks are accepted; we keep
        // the latest. The actual `MessageCompleted` event is emitted once
        // by `finalize` when the SSE stream closes (see struct docs).
        if let Some(reason) = finish_reason {
            self.finish_reason = Some(reason);
        }
    }

    /// Emit terminal events once the SSE stream has closed. The gateway
    /// MUST call this exactly once after `sse.next()` returns `None`.
    /// Returns `TextBlockCompleted` (when any text was accumulated),
    /// every buffered `ToolUseCompleted` in stream-index order, then a
    /// single `MessageCompleted`. Idempotent: a second call returns
    /// `Vec::new()`.
    pub(crate) fn finalize(&mut self) -> Vec<ModelEvent> {
        if self.finalized {
            return Vec::new();
        }
        self.finalized = true;

        let mut out = Vec::new();

        // Seal the text block with the full accumulated text.
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
        // chunk; v0.1 accepts zeroes here. Stop reason: latest seen, or
        // `EndTurn` if the server never sent one (e.g. truncated stream).
        let usage = Usage::default();
        let stop_reason = self
            .finish_reason
            .as_deref()
            .map_or(StopReason::EndTurn, parse_finish_reason);
        out.push(ModelEvent::MessageCompleted { stop_reason, usage });

        out
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
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
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
                    reasoning_content: None,
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
                        reasoning_content: None,
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

        // The finish-bearing chunk only records the reason; no seal yet.
        let e3 = dec
            .translate(make_chunk(None, Some("stop")))
            .expect("finish ok");
        assert!(
            e3.is_empty(),
            "finish_reason must not emit terminal events from translate"
        );

        // Stream close → finalize fires terminal events exactly once.
        let final_events = dec.finalize();
        assert_eq!(
            final_events.len(),
            2,
            "TextBlockCompleted + MessageCompleted"
        );
        assert!(matches!(
            &final_events[0],
            ModelEvent::TextBlockCompleted { text, .. } if text == "Hello, world!"
        ));
        assert!(matches!(
            &final_events[1],
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));

        // A second finalize call is a no-op.
        assert!(dec.finalize().is_empty(), "finalize must be idempotent");
    }

    #[test]
    fn tool_use_started_on_first_id_and_name() {
        let mut dec = Decoder::new();

        let chunk = StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: None,
                    reasoning_content: None,
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
                    reasoning_content: None,
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
                    reasoning_content: None,
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

        // Finish — only records the reason. ToolUseCompleted fires on finalize.
        let finish_events = dec
            .translate(StreamChunk {
                choices: vec![Choice {
                    delta: ChoiceDelta::default(),
                    finish_reason: Some("tool_calls".into()),
                }],
            })
            .expect("finish");
        assert!(
            finish_events.is_empty(),
            "ToolUseCompleted must wait for finalize"
        );

        let final_events = dec.finalize();
        let completed = final_events
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

        // MessageCompleted with StopReason::ToolUse.
        assert!(matches!(
            final_events.last(),
            Some(ModelEvent::MessageCompleted {
                stop_reason: StopReason::ToolUse,
                ..
            })
        ));
    }

    #[test]
    fn sensenova_per_chunk_finish_reason_streams_deltas_seals_once_at_finalize() {
        // SenseNova-style: every chunk carries `finish_reason: "stop"`,
        // not just the last one. The decoder MUST:
        // 1. Forward every TextDelta live (UI keeps streaming).
        // 2. Emit zero TextBlockCompleted / MessageCompleted from translate.
        // 3. Emit exactly one TextBlockCompleted carrying the FULL text
        //    and one MessageCompleted at finalize time.
        let mut dec = Decoder::new();

        let chunks = ["Hello", ", ", "world!"];
        let mut delta_count = 0;
        for chunk_text in chunks {
            let events = dec
                .translate(make_chunk(Some(chunk_text), Some("stop")))
                .expect("translate ok");
            for e in events {
                match e {
                    ModelEvent::TextDelta { .. } => delta_count += 1,
                    ModelEvent::TextBlockCompleted { .. } | ModelEvent::MessageCompleted { .. } => {
                        panic!("translate must not emit terminal events")
                    }
                    _ => {}
                }
            }
        }

        // A few extra trailing chunks with finish_reason only — also
        // must not produce terminal events.
        for _ in 0..2 {
            let events = dec
                .translate(make_chunk(None, Some("stop")))
                .expect("translate ok");
            for e in events {
                if matches!(
                    e,
                    ModelEvent::TextBlockCompleted { .. } | ModelEvent::MessageCompleted { .. }
                ) {
                    panic!("translate must not emit terminal events");
                }
            }
        }

        // All three input chunks had non-empty content → 3 deltas forwarded.
        assert_eq!(delta_count, 3, "every chunk's content must be broadcast");

        // Finalize fires the terminal pair with the *full* accumulated text.
        let final_events = dec.finalize();
        assert_eq!(final_events.len(), 2);
        assert!(matches!(
            &final_events[0],
            ModelEvent::TextBlockCompleted { text, .. } if text == "Hello, world!"
        ));
        assert!(matches!(
            &final_events[1],
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
    }

    #[test]
    fn finalize_without_finish_reason_defaults_to_end_turn() {
        // Truncated stream (e.g. network drop after some text but
        // before any finish_reason) still produces a clean seal.
        let mut dec = Decoder::new();
        let _ = dec
            .translate(make_chunk(Some("partial"), None))
            .expect("translate ok");

        let final_events = dec.finalize();
        assert_eq!(final_events.len(), 2);
        assert!(matches!(
            &final_events[0],
            ModelEvent::TextBlockCompleted { text, .. } if text == "partial"
        ));
        assert!(matches!(
            &final_events[1],
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
    }

    #[test]
    fn finalize_with_no_text_still_emits_message_completed() {
        // Empty stream — no content, no finish_reason. finalize still
        // emits MessageCompleted so demux can transition cleanly.
        let mut dec = Decoder::new();
        let final_events = dec.finalize();
        assert_eq!(final_events.len(), 1);
        assert!(matches!(
            &final_events[0],
            ModelEvent::MessageCompleted { .. }
        ));
    }

    #[test]
    fn parse_finish_reason_case_insensitive() {
        assert_eq!(parse_finish_reason("STOP"), StopReason::EndTurn);
        assert_eq!(parse_finish_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(parse_finish_reason("LENGTH"), StopReason::MaxTokens);
        assert_eq!(parse_finish_reason("unknown_value"), StopReason::EndTurn);
    }
}

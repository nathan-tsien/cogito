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

/// Open-tag sentinel for inline reasoning blocks emitted by open-source models
/// (e.g. DeepSeek-R1 raw, `QwQ`, llama.cpp default).
const OPEN_TAG: &str = "<think>";
/// Close-tag sentinel matching `OPEN_TAG`.
const CLOSE_TAG: &str = "</think>";

/// State of the two-state `<think>` tag parser.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ThinkTagState {
    /// Normal text mode — scanning for `<think>`.
    #[default]
    Outside,
    /// Inside a `<think>` block — scanning for `</think>`.
    Inside,
}

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
// Five bools track orthogonal aspects of the decoder's state machine
// (text_started, thinking_started, thinking_sealed, tag_parser_disabled,
// finalized). Collapsing them into a single enum loses the
// independent-axes meaning and would require more code, not less.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Decoder {
    /// Accumulated text for the text block.
    text_buf: String,
    /// Whether the text block has been opened (first content delta seen).
    text_started: bool,
    /// Tool-call buffer keyed by stream-level index.
    /// `block_index` is synthesised: when no thinking is present, text is 0
    /// and tools start at 1; when thinking is present, text shifts to 1 and
    /// tools start at 2 so the content array sorts as
    /// `[Thinking(0), Text(1), ToolUse(2+)]`.
    tool_calls: BTreeMap<u32, ToolCallBuf>,
    /// Next `block_index` to assign for `tool_use` blocks (starts at 1;
    /// bumped to 2 when a `reasoning_content` chunk arrives).
    next_tool_block: u32,
    /// Latest `finish_reason` seen across all chunks. Consumed by
    /// `finalize` to produce the terminal `MessageCompleted::stop_reason`.
    /// `None` is acceptable — `finalize` falls back to `EndTurn`.
    finish_reason: Option<String>,
    /// Whether `finalize` has already run. Idempotent on repeated calls.
    finalized: bool,
    /// Accumulated reasoning text (`DeepSeek` `reasoning_content` path).
    thinking_buf: String,
    /// Whether any `reasoning_content` chunk has been observed.
    thinking_started: bool,
    /// Whether `ThinkingBlockCompleted` has already been emitted for this
    /// stream (i.e. the thinking block has been sealed).
    thinking_sealed: bool,
    /// `block_index` of the thinking block. Always 0 when present.
    thinking_block_index: u32,
    /// `block_index` of the text block. Defaults to 0; bumped to 1 when
    /// thinking is present, so the assistant content array sorts as
    /// `[Thinking, Text, ToolUse, ...]` per ADR-0019 §4.
    text_block_index: u32,
    /// State machine state for inline `<think>` tag parsing.
    tag_state: ThinkTagState,
    /// Pending chunk-boundary buffer for tag matching — text we have
    /// received but cannot yet emit because it might be the prefix of
    /// `<think>` or `</think>`.
    tag_pending: String,
    /// Disables the inline-tag parser when `reasoning_content` was
    /// observed on this stream — the two paths are mutually exclusive.
    tag_parser_disabled: bool,
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

        // --- Reasoning content (DeepSeek/vLLM separate field) ---
        if let Some(reasoning) = delta.reasoning_content {
            if !self.thinking_started {
                self.thinking_started = true;
                // First reasoning chunk: bump text and tool indices up
                // so the assistant message sorts [Thinking(0), Text(1), Tools(2+)]
                // in H06's content reassembly (which sorts by block_index).
                self.text_block_index = 1;
                self.next_tool_block = 2;
                // Disable the inline-tag parser: the two paths are mutually
                // exclusive. A later `<think>` literal in delta.content is
                // just plain text, not a tag boundary.
                self.tag_parser_disabled = true;
            }
            if !reasoning.is_empty() {
                self.thinking_buf.push_str(&reasoning);
                out.push(ModelEvent::ThinkingDelta {
                    block_index: self.thinking_block_index,
                    chunk: reasoning,
                });
            }
        }

        // --- Text delta ---
        if let Some(text) = delta.content {
            // Seal thinking on the first content chunk if Task 13's
            // reasoning_content path was active (mutually exclusive with tag parser).
            if self.thinking_started && !self.thinking_sealed && self.tag_parser_disabled {
                let thinking_text = std::mem::take(&mut self.thinking_buf);
                out.push(ModelEvent::ThinkingBlockCompleted {
                    block_index: self.thinking_block_index,
                    text: thinking_text,
                    provider_opaque: None,
                });
                self.thinking_sealed = true;
            }

            if self.tag_parser_disabled {
                // Plain text path: no tag parsing.
                if !self.text_started {
                    self.text_started = true;
                }
                if !text.is_empty() {
                    self.text_buf.push_str(&text);
                    out.push(ModelEvent::TextDelta {
                        block_index: self.text_block_index,
                        chunk: text,
                    });
                }
            } else if !text.is_empty() {
                self.feed_with_tag_parser(&text, out);
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

    /// Feed one `delta.content` chunk through the `<think>` tag state machine.
    ///
    /// Emits zero or more `ModelEvent`s. Updates `tag_state`, `tag_pending`,
    /// `thinking_started`, `thinking_sealed`, `thinking_buf`, and bumps
    /// `text_block_index` / `next_tool_block` when entering the first thinking block.
    fn feed_with_tag_parser(&mut self, chunk: &str, out: &mut Vec<ModelEvent>) {
        self.tag_pending.push_str(chunk);
        loop {
            match self.tag_state {
                ThinkTagState::Outside => {
                    if let Some(pos) = self.tag_pending.find(OPEN_TAG) {
                        // Emit text before the open tag immediately.
                        let before: String = self.tag_pending.drain(..pos).collect();
                        self.tag_pending.drain(..OPEN_TAG.len());
                        if !before.is_empty() {
                            if !self.text_started {
                                self.text_started = true;
                            }
                            self.text_buf.push_str(&before);
                            out.push(ModelEvent::TextDelta {
                                block_index: self.text_block_index,
                                chunk: before,
                            });
                        }
                        // Entering the first thinking block: bump indices.
                        if !self.thinking_started {
                            self.thinking_started = true;
                            self.text_block_index = 1;
                            self.next_tool_block = 2;
                        }
                        self.tag_state = ThinkTagState::Inside;
                        // Continue the loop to process any content after <think>.
                    } else {
                        // No complete open tag found. Retain from the last `<`
                        // onward (it could be the start of a split `<think>`);
                        // everything before it is safe to emit as plain text.
                        let safe = self
                            .tag_pending
                            .rfind('<')
                            .unwrap_or(self.tag_pending.len());
                        if safe > 0 {
                            let emitted: String = self.tag_pending.drain(..safe).collect();
                            if !self.text_started {
                                self.text_started = true;
                            }
                            self.text_buf.push_str(&emitted);
                            out.push(ModelEvent::TextDelta {
                                block_index: self.text_block_index,
                                chunk: emitted,
                            });
                        }
                        break;
                    }
                }
                ThinkTagState::Inside => {
                    if let Some(pos) = self.tag_pending.find(CLOSE_TAG) {
                        // Emit thinking content before the close tag.
                        let before: String = self.tag_pending.drain(..pos).collect();
                        self.tag_pending.drain(..CLOSE_TAG.len());
                        if !before.is_empty() {
                            self.thinking_buf.push_str(&before);
                            out.push(ModelEvent::ThinkingDelta {
                                block_index: self.thinking_block_index,
                                chunk: before,
                            });
                        }
                        let thinking_text = std::mem::take(&mut self.thinking_buf);
                        out.push(ModelEvent::ThinkingBlockCompleted {
                            block_index: self.thinking_block_index,
                            text: thinking_text,
                            provider_opaque: None,
                        });
                        self.thinking_sealed = true;
                        self.tag_state = ThinkTagState::Outside;
                        // Continue the loop to process any content after </think>.
                    } else {
                        // No complete close tag found. Retain from the last `<`
                        // onward (it could be the start of a split `</think>`);
                        // everything before it is safe to emit as thinking deltas.
                        let safe = self
                            .tag_pending
                            .rfind('<')
                            .unwrap_or(self.tag_pending.len());
                        if safe > 0 {
                            let emitted: String = self.tag_pending.drain(..safe).collect();
                            self.thinking_buf.push_str(&emitted);
                            out.push(ModelEvent::ThinkingDelta {
                                block_index: self.thinking_block_index,
                                chunk: emitted,
                            });
                        }
                        break;
                    }
                }
            }
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

        // Drain leftover pending bytes from the inline-tag parser (e.g.
        // the safe-prefix tail that was not emitted yet). Treats unclosed
        // `<think>` as best-effort: whatever was accumulated becomes a
        // ThinkingBlockCompleted below.
        if !self.tag_pending.is_empty() {
            let leftover = std::mem::take(&mut self.tag_pending);
            match self.tag_state {
                ThinkTagState::Outside => {
                    if !self.text_started {
                        self.text_started = true;
                    }
                    self.text_buf.push_str(&leftover);
                    out.push(ModelEvent::TextDelta {
                        block_index: self.text_block_index,
                        chunk: leftover,
                    });
                }
                ThinkTagState::Inside => {
                    self.thinking_buf.push_str(&leftover);
                    out.push(ModelEvent::ThinkingDelta {
                        block_index: self.thinking_block_index,
                        chunk: leftover,
                    });
                }
            }
        }

        // Seal any unsealed thinking block.
        // Covers two cases:
        // - reasoning_content arrived but no delta.content followed (Task 13).
        // - an unclosed `<think>` was present in delta.content (Task 14).
        if self.thinking_started && !self.thinking_sealed {
            let thinking_text = std::mem::take(&mut self.thinking_buf);
            out.push(ModelEvent::ThinkingBlockCompleted {
                block_index: self.thinking_block_index,
                text: thinking_text,
                provider_opaque: None,
            });
            self.thinking_sealed = true;
        }

        // Seal the text block with the full accumulated text.
        if self.text_started {
            let text = std::mem::take(&mut self.text_buf);
            out.push(ModelEvent::TextBlockCompleted {
                block_index: self.text_block_index,
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

    fn make_chunk_with_reasoning(reasoning: &str, finish: Option<&str>) -> StreamChunk {
        StreamChunk {
            choices: vec![Choice {
                delta: ChoiceDelta {
                    role: None,
                    content: None,
                    reasoning_content: Some(reasoning.to_string()),
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

    #[test]
    fn reasoning_content_then_text_yields_thinking_then_text_blocks() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();

        // Two reasoning_content chunks.
        events.extend(
            decoder
                .translate(make_chunk_with_reasoning("I should ", None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(
            decoder
                .translate(make_chunk_with_reasoning("grep.", None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        // Transition: first content chunk.
        events.extend(
            decoder
                .translate(make_chunk(Some("OK."), None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        // Finish.
        events.extend(
            decoder
                .translate(make_chunk(None, Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        // Expected sequence:
        //   ThinkingDelta("I should "),
        //   ThinkingDelta("grep."),
        //   ThinkingBlockCompleted(text="I should grep.", provider_opaque: None),
        //   TextDelta("OK."),
        //   TextBlockCompleted(text="OK."),
        //   MessageCompleted(stop_reason: EndTurn, ...).
        let kinds: Vec<&'static str> = events
            .iter()
            .map(|e| match e {
                ModelEvent::ThinkingDelta { .. } => "td",
                ModelEvent::ThinkingBlockCompleted { .. } => "tc",
                ModelEvent::TextDelta { .. } => "xd",
                ModelEvent::TextBlockCompleted { .. } => "xc",
                ModelEvent::MessageCompleted { .. } => "mc",
                _ => "??",
            })
            .collect();
        assert_eq!(kinds, vec!["td", "td", "tc", "xd", "xc", "mc"]);

        // ThinkingBlockCompleted carries the full accumulated text and provider_opaque is None.
        let completed = events.iter().find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted {
                text,
                provider_opaque,
                ..
            } => Some((text.clone(), provider_opaque.clone())),
            _ => None,
        });
        #[allow(clippy::panic)]
        let (text, opaque) = completed.unwrap_or_else(|| panic!("ThinkingBlockCompleted missing"));
        assert_eq!(text, "I should grep.");
        assert_eq!(opaque, None);

        // Block indices: thinking is 0, text is 1.
        let think_idx = events.iter().find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted { block_index, .. } => Some(*block_index),
            _ => None,
        });
        let text_idx = events.iter().find_map(|e| match e {
            ModelEvent::TextBlockCompleted { block_index, .. } => Some(*block_index),
            _ => None,
        });
        assert_eq!(think_idx, Some(0));
        assert_eq!(text_idx, Some(1), "text must sort after thinking");
    }

    #[test]
    fn reasoning_content_only_no_text_seals_at_finalize() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();

        events.extend(
            decoder
                .translate(make_chunk_with_reasoning("thinking only", None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(
            decoder
                .translate(make_chunk(None, Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        // Expected: ThinkingDelta, then finalize emits ThinkingBlockCompleted + MessageCompleted.
        let kinds: Vec<&'static str> = events
            .iter()
            .map(|e| match e {
                ModelEvent::ThinkingDelta { .. } => "td",
                ModelEvent::ThinkingBlockCompleted { .. } => "tc",
                ModelEvent::TextBlockCompleted { .. } => "xc",
                ModelEvent::MessageCompleted { .. } => "mc",
                _ => "??",
            })
            .collect();
        assert_eq!(kinds, vec!["td", "tc", "mc"]);
    }

    #[test]
    fn no_reasoning_content_preserves_text_at_block_index_zero() {
        // Backwards compat: when no reasoning_content arrives, text stays at block 0.
        let mut decoder = Decoder::new();
        let _ = decoder
            .translate(make_chunk(Some("just text"), Some("stop")))
            .unwrap_or_else(|e| {
                #[allow(clippy::panic)]
                {
                    panic!("decoder error: {e:?}")
                }
            });
        let final_events = decoder.finalize();
        let text_idx = final_events.iter().find_map(|e| match e {
            ModelEvent::TextBlockCompleted { block_index, .. } => Some(*block_index),
            _ => None,
        });
        assert_eq!(
            text_idx,
            Some(0),
            "no reasoning -> text still at block_index 0 (back compat)"
        );
    }

    #[test]
    fn inline_think_tag_whole_chunk() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        events.extend(
            decoder
                .translate(make_chunk(Some("<think>I should grep.</think>OK."), None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(
            decoder
                .translate(make_chunk(None, Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        let kinds: Vec<&'static str> = events
            .iter()
            .map(|e| match e {
                ModelEvent::ThinkingDelta { .. } => "td",
                ModelEvent::ThinkingBlockCompleted { .. } => "tc",
                ModelEvent::TextDelta { .. } => "xd",
                ModelEvent::TextBlockCompleted { .. } => "xc",
                ModelEvent::MessageCompleted { .. } => "mc",
                _ => "??",
            })
            .collect();
        assert_eq!(kinds, vec!["td", "tc", "xd", "xc", "mc"]);

        let think_text = events.iter().find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(think_text.as_deref(), Some("I should grep."));

        let text_text = events.iter().find_map(|e| match e {
            ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(text_text.as_deref(), Some("OK."));
    }

    #[test]
    fn inline_think_tag_split_across_chunks() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        for piece in ["<thi", "nk>I should grep.</thi", "nk>OK."] {
            events.extend(
                decoder
                    .translate(make_chunk(Some(piece), None))
                    .unwrap_or_else(|e| {
                        #[allow(clippy::panic)]
                        {
                            panic!("decoder error: {e:?}")
                        }
                    }),
            );
        }
        events.extend(
            decoder
                .translate(make_chunk(None, Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        let think_text = events.iter().find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(think_text.as_deref(), Some("I should grep."));

        let text_text = events.iter().find_map(|e| match e {
            ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(text_text.as_deref(), Some("OK."));

        // No spurious TextDelta with partial-tag content (e.g. "<thi" leaking out as text).
        let leak = events
            .iter()
            .any(|e| matches!(e, ModelEvent::TextDelta { chunk, .. } if chunk.contains("<thi")));
        assert!(!leak, "partial open tag leaked into TextDelta");
    }

    #[test]
    fn unclosed_think_tag_at_stream_end_seals_best_effort() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        events.extend(
            decoder
                .translate(make_chunk(Some("<think>I never closed"), Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        let completed = events.iter().find_map(|e| match e {
            ModelEvent::ThinkingBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(
            completed.as_deref(),
            Some("I never closed"),
            "unclosed <think> on stream end must still seal as a thinking block"
        );
    }

    #[test]
    fn reasoning_content_seen_first_disables_tag_parser() {
        // Mutual exclusion: if reasoning_content arrived first, a later
        // <think> literal in delta.content is just plain text, not a tag.
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        events.extend(
            decoder
                .translate(make_chunk_with_reasoning("thinking", None))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        // After reasoning_content path is engaged, delta.content with <think>
        // should be treated as literal text (the reasoning block was already
        // sealed at the transition to content).
        events.extend(
            decoder
                .translate(make_chunk(Some("<think>not a tag</think>"), Some("stop")))
                .unwrap_or_else(|e| {
                    #[allow(clippy::panic)]
                    {
                        panic!("decoder error: {e:?}")
                    }
                }),
        );
        events.extend(decoder.finalize());

        // Exactly one ThinkingBlockCompleted (from the reasoning_content path), not two.
        let think_count = events
            .iter()
            .filter(|e| matches!(e, ModelEvent::ThinkingBlockCompleted { .. }))
            .count();
        assert_eq!(
            think_count, 1,
            "tag parser must not double-emit when reasoning_content was used"
        );

        // Text block contains the literal `<think>...</think>` verbatim.
        let text = events.iter().find_map(|e| match e {
            ModelEvent::TextBlockCompleted { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(text.as_deref(), Some("<think>not a tag</think>"));
    }
}

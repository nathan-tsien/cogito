//! Responses SSE → `ModelEvent` stream.
//!
//! The Responses API emits per-item-id stream events:
//! - `response.output_item.added` introduces a new item (message /
//!   `function_call` / reasoning) with its `id` and metadata.
//! - `response.output_text.{delta,done}` carry the text body of message
//!   items.
//! - `response.reasoning_summary_text.{delta,done}` carry the reasoning
//!   summary body — first-class wire-protocol objects, in contrast to
//!   `openai_compat`'s out-of-band `reasoning_content` field and the
//!   inline `<think>` markers.
//! - `response.function_call_arguments.{delta,done}` carry the argument
//!   JSON for a `function_call` item — tagged by the item's `id`, not
//!   the model's `call_id`.
//! - `response.completed` / `response.failed` close the stream.
//!
//! cogito needs block-indexed events. We synthesize indices in
//! observation order:
//! - Thinking blocks land first per ADR-0019 §4 (assistant content
//!   array sorts `[Thinking, Text, ToolUse...]`); we assign their
//!   indices as they appear.
//! - Text blocks follow.
//! - Tool-use blocks come last.
//!
//! For a single-message stream this collapses to `block_index = 0` for
//! whichever block opens first. For mixed streams the relative ordering
//! follows event arrival, which matches the Responses ordering
//! guarantees.

use std::collections::HashMap;

use async_stream::try_stream;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelError, ModelEvent, StopReason, Usage};
use eventsource_stream::Eventsource;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Client;

use super::OpenAiResponsesConfig;
use super::wire::{ResponseFinal, ResponsesRequest, StreamEvent};
use crate::error::from_reqwest;

/// Map a Responses terminal status / incomplete-details payload to
/// cogito's `StopReason`.
fn classify_stop_reason(response: &ResponseFinal) -> StopReason {
    if let Some(details) = response.incomplete_details.as_ref() {
        if details.reason == "max_output_tokens" || details.reason == "max_tokens" {
            return StopReason::MaxTokens;
        }
    }
    // `incomplete` without a recognized reason still means truncated.
    if response.status.as_deref() == Some("incomplete") {
        return StopReason::MaxTokens;
    }
    StopReason::EndTurn
}

/// Convert wire `Usage` (u32 fields) to protocol `Usage`.
fn convert_usage(u: &super::wire::Usage) -> Usage {
    Usage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
    }
}

/// Per-tool-call state buffered between `ToolUseStarted` and
/// `ToolUseCompleted`.
#[derive(Debug, Clone)]
struct ToolBuf {
    /// Block index assigned at `OutputItemAdded` time.
    block_index: u32,
    /// Model-issued call id (matches `ContentBlock::ToolUse.call_id`).
    call_id: String,
    /// Tool name.
    tool_name: String,
}

/// Per-stream decoder state.
//
// Four bools track orthogonal aspects of the decoder's lifecycle
// (text_started, text_sealed, thinking_started, thinking_sealed).
// Collapsing them loses the independent-axes meaning; matches the
// shape of `openai_compat::decode::Decoder`.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Decoder {
    /// Whether `text_block_index` has been allocated yet.
    text_started: bool,
    /// Synthesised `block_index` for the (single) text block.
    text_block_index: u32,
    /// Accumulated text — used as fallback if `output_text.done` does
    /// not arrive (e.g. truncated stream).
    text_buf: String,
    /// Whether text has been sealed via `output_text.done`.
    text_sealed: bool,

    /// Whether `thinking_block_index` has been allocated yet.
    thinking_started: bool,
    /// Synthesised `block_index` for the (single) thinking block.
    thinking_block_index: u32,
    /// Accumulated reasoning text — used as fallback if
    /// `reasoning_summary_text.done` does not arrive.
    thinking_buf: String,
    /// Whether thinking has been sealed via `reasoning_summary_text.done`.
    thinking_sealed: bool,

    /// `item_id` -> tool-call buffer.
    tool_calls: HashMap<String, ToolBuf>,
    /// Per-tool argument accumulator (only used as fallback if
    /// `function_call_arguments.done` does not arrive). Keyed by
    /// `item_id`.
    tool_args_buf: HashMap<String, String>,

    /// Next free `block_index`.
    next_block_index: u32,
}

impl Decoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Allocate and return the next available `block_index`.
    fn alloc_block_index(&mut self) -> u32 {
        let idx = self.next_block_index;
        self.next_block_index += 1;
        idx
    }

    /// Translate one parsed `StreamEvent` into zero or more `ModelEvent`s.
    ///
    /// Returns `(events, terminal)`. `terminal == true` means the stream
    /// has closed (`Completed` / `Failed`); the caller should not pull
    /// further events.
    pub(crate) fn translate(
        &mut self,
        evt: StreamEvent,
    ) -> Result<(Vec<ModelEvent>, bool), ModelError> {
        let mut out: Vec<ModelEvent> = Vec::new();
        match evt {
            StreamEvent::OutputItemAdded { item } => self.on_item_added(item, &mut out),
            StreamEvent::OutputTextDelta { delta } => self.on_text_delta(delta, &mut out),
            StreamEvent::OutputTextDone { text } => self.on_text_done(text, &mut out),
            StreamEvent::ReasoningSummaryDelta { delta } => {
                self.on_reasoning_delta(delta, &mut out);
            }
            StreamEvent::ReasoningSummaryDone { text } => {
                self.on_reasoning_done(text, &mut out);
            }
            StreamEvent::FunctionCallArgsDelta { item_id, delta } => {
                self.on_args_delta(&item_id, &delta);
            }
            StreamEvent::FunctionCallArgsDone { item_id, arguments } => {
                self.on_args_done(&item_id, &arguments, &mut out)?;
            }
            StreamEvent::Completed { response } => {
                self.on_completed(&response, &mut out);
                return Ok((out, true));
            }
            StreamEvent::Failed { response } => {
                let msg = response
                    .error
                    .map_or_else(|| "Responses stream failed".to_string(), |e| e.message);
                return Err(ModelError::Provider {
                    status: 0,
                    message: msg,
                });
            }
            StreamEvent::Other => {}
        }
        Ok((out, false))
    }

    fn on_item_added(&mut self, item: super::wire::OutputItemHeader, out: &mut Vec<ModelEvent>) {
        if item.kind != "function_call" {
            // For `message` and `reasoning` items we wait until the
            // first delta arrives to allocate the index. This keeps
            // headers without bodies (which the server may emit)
            // from polluting the output with empty blocks.
            return;
        }
        let (Some(call_id), Some(tool_name)) = (item.call_id, item.name) else {
            return;
        };
        if call_id.is_empty() || tool_name.is_empty() {
            return;
        }
        let block_index = self.alloc_block_index();
        self.tool_calls.insert(
            item.id,
            ToolBuf {
                block_index,
                call_id: call_id.clone(),
                tool_name: tool_name.clone(),
            },
        );
        out.push(ModelEvent::ToolUseStarted {
            block_index,
            call_id,
            tool_name,
        });
    }

    fn on_text_delta(&mut self, delta: String, out: &mut Vec<ModelEvent>) {
        if !self.text_started {
            self.text_started = true;
            self.text_block_index = self.alloc_block_index();
        }
        if !delta.is_empty() {
            self.text_buf.push_str(&delta);
            out.push(ModelEvent::TextDelta {
                block_index: self.text_block_index,
                chunk: delta,
            });
        }
    }

    fn on_text_done(&mut self, text: String, out: &mut Vec<ModelEvent>) {
        if !self.text_started {
            // `done` arrived without any prior `delta` — allocate
            // an index now so the sealed block carries one.
            self.text_started = true;
            self.text_block_index = self.alloc_block_index();
        }
        self.text_sealed = true;
        self.text_buf.clear();
        out.push(ModelEvent::TextBlockCompleted {
            block_index: self.text_block_index,
            text,
        });
    }

    fn on_reasoning_delta(&mut self, delta: String, out: &mut Vec<ModelEvent>) {
        if !self.thinking_started {
            self.thinking_started = true;
            self.thinking_block_index = self.alloc_block_index();
        }
        if !delta.is_empty() {
            self.thinking_buf.push_str(&delta);
            out.push(ModelEvent::ThinkingDelta {
                block_index: self.thinking_block_index,
                chunk: delta,
            });
        }
    }

    fn on_reasoning_done(&mut self, text: String, out: &mut Vec<ModelEvent>) {
        if !self.thinking_started {
            self.thinking_started = true;
            self.thinking_block_index = self.alloc_block_index();
        }
        self.thinking_sealed = true;
        self.thinking_buf.clear();
        out.push(ModelEvent::ThinkingBlockCompleted {
            block_index: self.thinking_block_index,
            text,
            // Responses surfaces an `encrypted_content` field on
            // reasoning items in the non-streaming path; the
            // streaming `summary_text.done` event does not carry it.
            // Pass `None` so persistence stays truthful — a follow-up
            // turn that re-feeds this block simply skips the opaque
            // payload.
            provider_opaque: None,
        });
    }

    fn on_args_delta(&mut self, item_id: &str, delta: &str) {
        if self.tool_calls.contains_key(item_id) {
            self.tool_args_buf
                .entry(item_id.to_string())
                .or_default()
                .push_str(delta);
        }
        // No live ToolUseDelta event in cogito's protocol; accumulate
        // silently and emit ToolUseCompleted when sealed.
    }

    fn on_args_done(
        &mut self,
        item_id: &str,
        arguments: &str,
        out: &mut Vec<ModelEvent>,
    ) -> Result<(), ModelError> {
        if let Some(buf) = self.tool_calls.remove(item_id) {
            self.tool_args_buf.remove(item_id);
            let args = if arguments.is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(arguments).map_err(|e| {
                    ModelError::Decode(format!("openai-responses tool arguments JSON: {e}"))
                })?
            };
            out.push(ModelEvent::ToolUseCompleted {
                block_index: buf.block_index,
                call_id: buf.call_id,
                tool_name: buf.tool_name,
                args,
            });
        }
        Ok(())
    }

    fn on_completed(&mut self, response: &ResponseFinal, out: &mut Vec<ModelEvent>) {
        // Seal any text / thinking blocks that received deltas but no
        // done events (defensive — the server should always emit
        // `done`, but mirror openai_compat's robustness here).
        if self.text_started && !self.text_sealed {
            self.text_sealed = true;
            let text = std::mem::take(&mut self.text_buf);
            out.push(ModelEvent::TextBlockCompleted {
                block_index: self.text_block_index,
                text,
            });
        }
        if self.thinking_started && !self.thinking_sealed {
            self.thinking_sealed = true;
            let text = std::mem::take(&mut self.thinking_buf);
            out.push(ModelEvent::ThinkingBlockCompleted {
                block_index: self.thinking_block_index,
                text,
                provider_opaque: None,
            });
        }

        let stop_reason = classify_stop_reason(response);
        let usage = response
            .usage
            .as_ref()
            .map_or_else(Usage::default, convert_usage);
        out.push(ModelEvent::MessageCompleted { stop_reason, usage });
    }
}

/// Open the Responses streaming call and return a `ModelEvent` stream.
///
/// # Errors
///
/// Returns `ModelError::Network` on transport failures, `ModelError::Auth`
/// on 401/403, `ModelError::RateLimited` on 429, and
/// `ModelError::Provider` on other non-2xx responses. Per-chunk decode
/// failures arrive as `Err` items inside the returned stream.
pub(crate) async fn stream_response(
    client: &Client,
    cfg: &OpenAiResponsesConfig,
    request: ResponsesRequest,
    ctx: ExecCtx,
) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
    let url = format!("{}/responses", cfg.base_url.trim_end_matches('/'));

    if tracing::enabled!(tracing::Level::DEBUG) {
        match serde_json::to_string(&request) {
            Ok(json) => {
                tracing::debug!(target: "cogito::prompt", url = %url, "request: {json}");
            }
            Err(e) => {
                tracing::debug!(target: "cogito::prompt", "request body serialization failed: {e}");
            }
        }
    }

    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", cfg.api_key))
        .json(&request)
        .send()
        .await
        .map_err(|e| from_reqwest(&e))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let message = response.text().await.unwrap_or_default();
        return Err(match status {
            401 | 403 => ModelError::Auth,
            429 => ModelError::RateLimited {
                retry_after_secs: None,
            },
            _ => ModelError::Provider { status, message },
        });
    }

    let mut sse = Box::pin(response.bytes_stream().eventsource());
    let mut decoder = Decoder::new();
    let cancel = ctx.cancel.clone();

    let s = try_stream! {
        loop {
            // The select! result is communicated via Step so the inner
            // `?` works inside try_stream!.
            enum Step {
                Cancelled,
                Event(Option<Result<eventsource_stream::Event, eventsource_stream::EventStreamError<reqwest::Error>>>),
            }
            let step = tokio::select! {
                () = cancel.cancelled() => Step::Cancelled,
                e = sse.next() => Step::Event(e),
            };
            match step {
                Step::Cancelled => {
                    Err(ModelError::Cancelled)?;
                }
                Step::Event(None) => {
                    break;
                }
                Step::Event(Some(res)) => {
                    let evt = res.map_err(|e| ModelError::Decode(format!("sse parse: {e}")))?;
                    if evt.data.is_empty() {
                        continue;
                    }
                    let parsed: StreamEvent = match serde_json::from_str(&evt.data) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::debug!(
                                error = %e,
                                raw = %evt.data,
                                "unparseable Responses SSE event; skipping",
                            );
                            continue;
                        }
                    };
                    let (events, terminal) = decoder.translate(parsed)?;
                    for m in events {
                        yield m;
                    }
                    if terminal {
                        break;
                    }
                }
            }
        }
    };
    Ok(s.boxed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    //! Unit tests for the parser's degenerate paths.
    //!
    //! Fixture-driven integration coverage lives in
    //! `crates/cogito-model/tests/openai_responses_decode.rs` and drives
    //! recorded SSE bytes through
    //! `cogito_model::sse::replay_openai_responses_into_model_events`.
    //! The full async stream integration is exercised by a manual
    //! live-API smoke test (not in CI).

    use super::*;

    #[test]
    fn unknown_event_type_is_skipped() {
        let mut d = Decoder::new();
        let json = r#"{"type":"response.future_thing.delta","delta":"hi"}"#;
        let parsed: StreamEvent = serde_json::from_str(json).unwrap();
        let (events, terminal) = d.translate(parsed).unwrap();
        assert!(events.is_empty());
        assert!(!terminal);
    }

    #[test]
    fn output_text_delta_emits_text_delta_event() {
        let mut d = Decoder::new();
        let parsed: StreamEvent =
            serde_json::from_str(r#"{"type":"response.output_text.delta","delta":"hello"}"#)
                .unwrap();
        let (events, _) = d.translate(parsed).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelEvent::TextDelta { block_index, chunk } => {
                assert_eq!(*block_index, 0);
                assert_eq!(chunk, "hello");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }
}

//! Shared SSE helper — produces a stream of `(event_name: Option<String>,
//! data: String)` tuples from a reqwest response body.
//!
//! Both `anthropic` and `openai_compat` decoders consume this; provider-
//! specific JSON parsing happens in their own `decode.rs` modules.

use eventsource_stream::Eventsource;
use futures::stream::{Stream, StreamExt};
use reqwest::Response;

use crate::error::wire;
use cogito_protocol::gateway::ModelError;

/// One SSE event line, normalized.
#[derive(Debug, Clone)]
pub struct SseLine {
    /// Anthropic uses this (`event: content_block_delta`); `OpenAI` doesn't.
    pub event: Option<String>,
    /// JSON-encoded payload from `data: ...`.
    pub data: String,
}

/// Wrap a `reqwest::Response` body into an `SseLine` stream.
///
/// Errors map any `reqwest` decode failure into `ModelError::Decode`.
pub fn lines(response: Response) -> impl Stream<Item = Result<SseLine, ModelError>> + Send + 'static {
    response.bytes_stream().eventsource().map(|res| match res {
        Ok(evt) => {
            let event_name = if evt.event.is_empty() { None } else { Some(evt.event) };
            Ok(SseLine { event: event_name, data: evt.data })
        }
        Err(e) => Err(wire::decode(format!("sse parse: {e}"))),
    })
}

/// Test helper: feed raw SSE bytes through the Anthropic decoder
/// synchronously and collect the resulting `ModelEvent`s. Used by
/// integration replay tests; not part of the public API surface.
#[doc(hidden)]
pub fn replay_anthropic_into_model_events(
    bytes: &[u8],
) -> Result<Vec<cogito_protocol::gateway::ModelEvent>, ModelError> {
    use eventsource_stream::EventStream;
    use futures::StreamExt;

    let body = futures::stream::iter(vec![Ok::<_, std::io::Error>(
        ::bytes::Bytes::copy_from_slice(bytes),
    )]);
    let mut parsed = EventStream::new(body);
    let mut decoder = crate::anthropic::decode::Decoder::new();
    let mut out = Vec::new();
    futures::executor::block_on(async {
        while let Some(res) = parsed.next().await {
            let evt = res.map_err(|e| ModelError::Decode(format!("sse parse: {e}")))?;
            if evt.data.is_empty() {
                continue;
            }
            let sse: crate::anthropic::wire::SseEvent =
                serde_json::from_str(&evt.data).map_err(|e| ModelError::Decode(e.to_string()))?;
            for m in decoder.translate(sse)? {
                out.push(m);
            }
        }
        Ok::<_, ModelError>(())
    })?;
    Ok(out)
}

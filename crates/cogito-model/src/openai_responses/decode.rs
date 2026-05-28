//! Responses SSE → `ModelEvent`. Implemented in Task 13.

use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelError, ModelEvent};
use futures::stream::BoxStream;
use reqwest::Client;

use super::OpenAiResponsesConfig;
use super::wire::ResponsesRequest;

/// Open the Responses streaming call and return a `ModelEvent` stream.
///
/// Task 13 lands the real decoder; this stub returns a Decode error.
#[allow(clippy::unused_async)]
pub(crate) async fn stream_response(
    _client: &Client,
    _cfg: &OpenAiResponsesConfig,
    _request: ResponsesRequest,
    _ctx: ExecCtx,
) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
    Err(ModelError::Decode("Task 13 lands the decoder".into()))
}

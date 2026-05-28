//! `ContentBlock` → Responses input items. Implemented in Task 12.

use cogito_protocol::gateway::ModelInput;

use super::OpenAiResponsesConfig;
use super::wire::ResponsesRequest;

/// Encode a `ModelInput` into a Responses API request body.
///
/// Task 12 lands the real encoder; this stub panics if invoked.
#[allow(clippy::unimplemented)]
pub(crate) fn encode_request(
    _input: &ModelInput,
    _cfg: &OpenAiResponsesConfig,
) -> ResponsesRequest {
    unimplemented!("Task 12 lands the encoder")
}

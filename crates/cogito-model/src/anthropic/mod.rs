//! `AnthropicGateway` — implements `ModelGateway` against the Anthropic
//! Messages API (`POST /v1/messages` with `stream: true`).

pub mod decode;
pub mod encode;
pub mod wire;

use std::time::Duration;

use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{
    ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits, parse_context_window_suffix,
};
use futures::stream::BoxStream;
use reqwest::Client;

/// Static configuration for `AnthropicGateway`.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// `x-api-key` header value.
    pub api_key: String,
    /// Base URL. Default: `https://api.anthropic.com`.
    pub base_url: String,
    /// `anthropic-version` header. Default: `2023-06-01`.
    pub anthropic_version: String,
    /// Per-request timeout. Default: 5 minutes.
    pub timeout: Duration,
    /// Model identifier, e.g. `"claude-opus-4-7"` or `"claude-opus-4-7[1m]"`.
    /// Used by `model_limits()` to return context window size.
    pub model_id: String,
}

impl AnthropicConfig {
    /// Sensible defaults; caller provides only `api_key`.
    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout: Duration::from_secs(5 * 60),
            model_id: String::new(),
        }
    }
}

/// `ModelGateway` impl for Anthropic.
pub struct AnthropicGateway {
    cfg: AnthropicConfig,
    client: Client,
}

impl AnthropicGateway {
    /// Build a gateway.
    ///
    /// # Errors
    ///
    /// Returns `ModelError::Network` if the underlying reqwest client
    /// cannot be constructed (rare — typically TLS config failures).
    pub fn new(cfg: AnthropicConfig) -> Result<Self, ModelError> {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| crate::error::from_reqwest(&e))?;
        Ok(Self { cfg, client })
    }

    /// Construct a minimal gateway for unit/integration tests, bypassing
    /// normal HTTP client configuration. The returned gateway is not
    /// suitable for real API calls.
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_test(api_key: &str, model_id: &str) -> Self {
        let cfg = AnthropicConfig {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".into(),
            anthropic_version: "2023-06-01".into(),
            timeout: Duration::from_secs(5 * 60),
            model_id: model_id.into(),
        };
        // unwrap is safe: default Client::new() cannot fail on a modern system.
        #[allow(clippy::unwrap_used)]
        let client = Client::new();
        Self { cfg, client }
    }
}

/// Return the default context window for a known Anthropic base model id
/// (with any `[<size>]` suffix stripped). Returns `None` for unknown models.
fn anthropic_default_window(model_id: &str) -> Option<u64> {
    let base = model_id.split_once('[').map_or(model_id, |(b, _)| b);
    match base {
        "claude-opus-4-7"
        | "claude-opus-4-7-20260301"
        | "claude-sonnet-4-6"
        | "claude-sonnet-4-6-20260301"
        | "claude-haiku-4-5"
        | "claude-haiku-4-5-20251001" => Some(200_000),
        _ => None,
    }
}

use async_stream::try_stream;
use futures::stream::StreamExt;

use crate::sse::lines;

#[async_trait::async_trait]
impl ModelGateway for AnthropicGateway {
    async fn stream(
        &self,
        input: ModelInput,
        ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        let body = encode::encode(input);
        let url = format!("{}/v1/messages", self.cfg.base_url.trim_end_matches('/'));

        if tracing::enabled!(tracing::Level::DEBUG) {
            match serde_json::to_string(&body) {
                Ok(json) => {
                    tracing::debug!(target: "cogito::prompt", url = %url, "request: {json}");
                }
                Err(e) => {
                    tracing::debug!(target: "cogito::prompt", "request body serialization failed: {e}");
                }
            }
        }

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", &self.cfg.anthropic_version)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| crate::error::from_reqwest(&e))?;

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

        let mut sse = Box::pin(lines(response));
        let mut decoder = decode::Decoder::new();
        let cancel = ctx.cancel.clone();

        let s = try_stream! {
            loop {
                // Use an enum to communicate the select result back into
                // try_stream! where `?` works correctly.
                enum Step {
                    Cancelled,
                    Line(Option<Result<crate::sse::SseLine, ModelError>>),
                }
                let step = tokio::select! {
                    () = cancel.cancelled() => Step::Cancelled,
                    line = sse.next() => Step::Line(line),
                };
                match step {
                    Step::Cancelled => {
                        Err(ModelError::Cancelled)?;
                    }
                    Step::Line(None) => break,
                    Step::Line(Some(res)) => {
                        let line = res?;
                        if line.data.is_empty() {
                            continue;
                        }
                        let sse_event: wire::SseEvent = serde_json::from_str(&line.data)
                            .map_err(|e| ModelError::Decode(format!("anthropic event: {e}")))?;
                        for m in decoder.translate(sse_event)? {
                            yield m;
                        }
                    }
                }
            }
        };
        Ok(s.boxed())
    }

    fn provider_id(&self) -> &'static str {
        "anthropic"
    }

    fn model_limits(&self) -> ModelLimits {
        let window = parse_context_window_suffix(&self.cfg.model_id)
            .or_else(|| anthropic_default_window(&self.cfg.model_id))
            .unwrap_or_else(|| {
                tracing::warn!(
                    model_id = %self.cfg.model_id,
                    "no context window declared for model; falling back to 200_000",
                );
                200_000
            });
        ModelLimits::new(self.cfg.model_id.clone(), window)
    }
}

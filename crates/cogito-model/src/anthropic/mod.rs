//! `AnthropicGateway` — implements `ModelGateway` against the Anthropic
//! Messages API (`POST /v1/messages` with `stream: true`).

pub mod decode;
pub mod encode;
pub mod wire;

use std::time::Duration;

use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelGateway, ModelInput};
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
}

//! `OpenAiCompatGateway` — implements `ModelGateway` against any
//! OpenAI-compatible Chat Completions endpoint (vLLM / `SGLang` / Azure
//! `OpenAI` / internal LLM gateway).
//!
//! NOT the `OpenAI` Responses API — that arrives in Sprint 5.

pub mod decode;
pub mod encode;
pub mod wire;

use std::time::Duration;

use reqwest::Client;

/// Configuration for an OpenAI-Compatible endpoint.
///
/// `base_url` is required; auth header naming and scheme are configurable for
/// private deployments that diverge from the `OpenAI` defaults.
#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    /// Bearer token (or equivalent). `None` means no auth header is sent —
    /// for unauthenticated private gateways.
    pub api_key: Option<String>,
    /// Required base URL, e.g. `http://vllm:8000/v1`.
    pub base_url: String,
    /// HTTP header carrying the credential. Default: `Authorization`.
    pub auth_header: String,
    /// Scheme prefix prepended to `api_key`. Default: `Bearer`.
    pub auth_scheme: String,
    /// Per-request timeout. Default: 5 minutes.
    pub timeout: Duration,
}

impl OpenAiCompatConfig {
    /// Build with sensible defaults for OpenAI-style auth.
    ///
    /// The returned config sends no API key; set `cfg.api_key = Some(...)` for
    /// authenticated deployments.
    #[must_use]
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            api_key: None,
            base_url: base_url.into(),
            auth_header: "Authorization".into(),
            auth_scheme: "Bearer".into(),
            timeout: Duration::from_secs(5 * 60),
        }
    }
}

/// `ModelGateway` implementation for OpenAI-compatible Chat Completions.
///
/// Construct via [`OpenAiCompatGateway::new`]; inject into the runtime as
/// `Arc<dyn ModelGateway>`.
pub struct OpenAiCompatGateway {
    cfg: OpenAiCompatConfig,
    client: Client,
}

impl OpenAiCompatGateway {
    /// Build a gateway from `cfg`.
    ///
    /// # Errors
    ///
    /// Returns `ModelError::Network` if the underlying reqwest client cannot be
    /// constructed (rare — typically TLS configuration failures).
    pub fn new(cfg: OpenAiCompatConfig) -> Result<Self, ModelError> {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| crate::error::from_reqwest(&e))?;
        Ok(Self { cfg, client })
    }
}

use async_stream::try_stream;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelGateway, ModelInput};
use cogito_protocol::ExecCtx;
use futures::stream::{BoxStream, StreamExt};

use crate::error::from_reqwest;
use crate::sse::lines;

#[async_trait::async_trait]
impl ModelGateway for OpenAiCompatGateway {
    /// Stream a model call against the configured Chat Completions endpoint.
    ///
    /// The stream emits `TextDelta` / `ToolUseStarted` / `ToolUseCompleted` /
    /// `TextBlockCompleted` events, terminated by exactly one
    /// `MessageCompleted`.  Cancellation via `ctx.cancel` aborts mid-stream.
    ///
    /// # Errors
    ///
    /// Returns `ModelError` on network, auth, or rate-limit failures.
    async fn stream(
        &self,
        input: ModelInput,
        ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        let body = encode::encode(input);
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );

        let mut builder = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body);

        if let Some(key) = &self.cfg.api_key {
            let value = if self.cfg.auth_scheme.is_empty() {
                key.clone()
            } else {
                format!("{} {}", self.cfg.auth_scheme, key)
            };
            builder = builder.header(self.cfg.auth_header.as_str(), value);
        }

        let response = builder.send().await.map_err(|e| from_reqwest(&e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(match status {
                401 | 403 => ModelError::Auth,
                429 => ModelError::RateLimited { retry_after_secs: None },
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
                        // Skip empty lines and the final [DONE] sentinel.
                        if line.data.is_empty() || line.data == "[DONE]" {
                            continue;
                        }
                        let chunk: wire::StreamChunk =
                            serde_json::from_str(&line.data).map_err(|e| {
                                ModelError::Decode(format!("openai-compat chunk: {e}"))
                            })?;
                        for m in decoder.translate(chunk)? {
                            yield m;
                        }
                    }
                }
            }
        };
        Ok(s.boxed())
    }

    fn provider_id(&self) -> &'static str {
        "openai-compat"
    }
}

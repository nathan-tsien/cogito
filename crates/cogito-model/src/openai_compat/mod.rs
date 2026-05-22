//! `OpenAiCompatGateway` — implements `ModelGateway` against any
//! OpenAI-compatible Chat Completions endpoint (vLLM / `SGLang` / Azure
//! `OpenAI` / internal LLM gateway).
//!
//! NOT the `OpenAI` Responses API — that arrives in Sprint 5.

pub mod decode;
pub mod encode;
pub mod wire;

use std::time::Duration;

use async_stream::try_stream;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{
    ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits, parse_context_window_suffix,
};
use futures::stream::{BoxStream, StreamExt};
use reqwest::Client;

use crate::error::from_reqwest;
use crate::sse::lines;

/// Strip a `[<size>]` suffix from a model identifier.
///
/// The suffix is a local-only annotation used by `model_limits()` to derive the
/// context window size (e.g. `"Llama-3.3-70B[32k]"`). vLLM / `SGLang` servers
/// treat the full string as an unknown model id and reject the request.
///
/// Returns the base string slice before `[`, or the whole input if no `[` is
/// present.
fn strip_size_suffix(model: &str) -> &str {
    model.split_once('[').map_or(model, |(base, _)| base)
}

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
    /// Whether to re-feed prior-turn `ContentBlock::Thinking` blocks
    /// when building outgoing assistant messages. See ADR-0019 §5.3.
    /// Default `false` — matches DeepSeek-R1 / `QwQ` convention.
    pub include_prior_thinking: bool,
    /// Optional fallback context-window size in tokens. Used by
    /// `model_limits()` when the model id carries no `[<size>]` suffix.
    /// `None` causes the gateway to fall back to `32_768` with a warn log.
    pub context_window_tokens: Option<u64>,
    /// Model identifier used for `model_limits()` and to derive the wire-level
    /// `api_model_id()`. May carry a `[<size>]` suffix (e.g. `"Llama-3.3-70B[32k]"`)
    /// that is stripped before sending to the server. Empty string means no
    /// gateway-level model override; callers rely on `ModelInput.params.model`.
    pub model: String,
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
            include_prior_thinking: false,
            context_window_tokens: None,
            model: String::new(),
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

    /// Construct a minimal gateway for unit/integration tests, bypassing
    /// normal HTTP client configuration. The returned gateway is not
    /// suitable for real API calls.
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_test(cfg: OpenAiCompatConfig) -> Self {
        // unwrap is safe: default Client::new() cannot fail on a modern system.
        #[allow(clippy::unwrap_used)]
        let client = Client::new();
        Self { cfg, client }
    }

    /// Return the model identifier to send on the wire, with any `[<size>]`
    /// suffix stripped. The suffix is a local-only annotation that vLLM /
    /// `SGLang` servers would reject.
    ///
    /// Falls back to `cfg.model` unchanged if no suffix is present.
    #[must_use]
    pub fn api_model_id(&self) -> String {
        strip_size_suffix(&self.cfg.model).to_owned()
    }
}

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
        mut input: ModelInput,
        ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        // Strip the [<size>] suffix before sending to the server. The suffix is
        // a local convention for adaptive ModelLimits (see ADR-0008) and is not
        // a real model name recognised by vLLM / SGLang.
        let stripped = strip_size_suffix(&input.params.model).to_owned();
        input.params.model = stripped;
        let body = encode::encode(input, self.cfg.include_prior_thinking);
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );

        // In DEBUG builds / when RUST_LOG includes debug, emit the full wire-level
        // request JSON so developers can verify the tool schema and conversation
        // history reach the model exactly as intended.
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
                    Step::Line(None) => {
                        // Stream closed: emit terminal events
                        // (TextBlockCompleted / ToolUseCompleted /
                        // MessageCompleted) from the decoder before exit.
                        for m in decoder.finalize() {
                            yield m;
                        }
                        break;
                    }
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

    fn model_limits(&self) -> ModelLimits {
        let model_id = &self.cfg.model;
        let window = parse_context_window_suffix(model_id)
            .or(self.cfg.context_window_tokens)
            .unwrap_or_else(|| {
                tracing::warn!(
                    model = %model_id,
                    "no context window declared (suffix nor provider config); falling back to 32_768",
                );
                32_768
            });
        ModelLimits::new(model_id.clone(), window)
    }
}

#[cfg(test)]
mod strip_suffix_tests {
    use super::*;
    use cogito_protocol::gateway::{Message, ModelParams};

    #[test]
    fn strip_with_suffix_returns_base() {
        assert_eq!(strip_size_suffix("Llama-3.3-70B[32k]"), "Llama-3.3-70B");
    }

    #[test]
    fn strip_without_suffix_returns_unchanged() {
        assert_eq!(strip_size_suffix("Llama-3.3-70B"), "Llama-3.3-70B");
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(strip_size_suffix(""), "");
    }

    #[test]
    fn api_model_id_strips_suffix() {
        let cfg = OpenAiCompatConfig {
            model: "Llama-3.3-70B[32k]".into(),
            ..OpenAiCompatConfig::with_base_url("http://localhost:8000/v1")
        };
        let gw = OpenAiCompatGateway::new_for_test(cfg);
        assert_eq!(gw.api_model_id(), "Llama-3.3-70B");
    }

    #[test]
    fn api_model_id_no_suffix_passthrough() {
        let cfg = OpenAiCompatConfig {
            model: "Llama-3.3-70B".into(),
            ..OpenAiCompatConfig::with_base_url("http://localhost:8000/v1")
        };
        let gw = OpenAiCompatGateway::new_for_test(cfg);
        assert_eq!(gw.api_model_id(), "Llama-3.3-70B");
    }

    /// Verify the suffix is stripped in the wire request body produced by
    /// `encode::encode`. This confirms the fix is applied on the encode path,
    /// not just in `api_model_id()`.
    #[test]
    fn encode_strips_suffix_in_wire_request() {
        let input = ModelInput {
            system: String::new(),
            messages: vec![Message::User {
                content: vec![cogito_protocol::content::ContentBlock::Text {
                    text: "hello".into(),
                }],
            }],
            tools: Vec::new(),
            params: ModelParams {
                model: "Llama-3.3-70B[32k]".into(),
                max_tokens: 256,
                temperature: None,
                top_p: None,
                stop_sequences: Vec::new(),
            },
        };
        // Simulate what stream() does: strip the suffix, then encode.
        let mut stripped_input = input;
        stripped_input.params.model = strip_size_suffix(&stripped_input.params.model).to_owned();
        let req = encode::encode(stripped_input, false);
        assert_eq!(req.model, "Llama-3.3-70B");
    }
}

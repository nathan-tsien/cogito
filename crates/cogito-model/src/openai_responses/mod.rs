//! `OpenAiResponsesGateway` — implements `ModelGateway` against the
//! `OpenAI` Responses API (`POST /v1/responses`).
//!
//! Distinct from `openai_compat` (Chat Completions): Responses uses a
//! flat top-level input array, native `reasoning` items for thinking
//! re-feed, and `response.*` SSE event names. Decodes reasoning summary
//! parts into `ContentBlock::Thinking` per ADR-0019 §5.4.
//!
//! No built-in tools (`file_search` / `web_search` / `code_interpreter`) —
//! those are a Hands concern, not a Boundary concern.

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

pub use wire::ReasoningEffort;

/// Configuration for an `OpenAI` Responses endpoint.
///
/// `api_key` is required; `base_url` defaults to the public `OpenAI`
/// endpoint (override for compatible third-party services).
#[derive(Debug, Clone)]
pub struct OpenAiResponsesConfig {
    /// Bearer token for `Authorization: Bearer ...`.
    pub api_key: String,
    /// Base URL. Defaults to [`OpenAiResponsesConfig::DEFAULT_BASE_URL`].
    pub base_url: String,
    /// Per-request timeout. Default: 5 minutes.
    pub timeout: Duration,
    /// Optional `reasoning.effort` toggle.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Model identifier used for `model_limits()`. May carry a
    /// `[<size>]` suffix (e.g. `"o1-preview[128k]"`) to declare the
    /// context window. Empty string means rely on `ModelInput.params.model`
    /// at call time.
    pub model: String,
    /// Optional fallback context-window size in tokens. Used by
    /// `model_limits()` when the model id carries no `[<size>]` suffix.
    pub context_window_tokens: Option<u64>,
}

impl OpenAiResponsesConfig {
    /// Default base URL for the public `OpenAI` Responses API.
    pub const DEFAULT_BASE_URL: &'static str = "https://api.openai.com/v1";

    /// Build with sensible defaults and the supplied API key.
    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: Self::DEFAULT_BASE_URL.into(),
            timeout: Duration::from_secs(5 * 60),
            reasoning_effort: None,
            model: String::new(),
            context_window_tokens: None,
        }
    }
}

/// `ModelGateway` implementation for the `OpenAI` Responses API.
///
/// Construct via [`OpenAiResponsesGateway::new`]; inject into the
/// runtime as `Arc<dyn ModelGateway>`.
pub struct OpenAiResponsesGateway {
    cfg: OpenAiResponsesConfig,
    client: Client,
}

impl OpenAiResponsesGateway {
    /// Build a gateway from `cfg`.
    ///
    /// # Errors
    ///
    /// Returns `ModelError::Network` if the underlying reqwest client
    /// cannot be constructed (rare — typically TLS configuration
    /// failures).
    pub fn new(cfg: OpenAiResponsesConfig) -> Result<Self, ModelError> {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| crate::error::from_reqwest(&e))?;
        Ok(Self { cfg, client })
    }

    /// Construct a minimal gateway for unit/integration tests, bypassing
    /// the normal HTTP client configuration. The returned gateway is
    /// not suitable for real API calls.
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_test(cfg: OpenAiResponsesConfig) -> Self {
        Self {
            cfg,
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ModelGateway for OpenAiResponsesGateway {
    /// Stream a model call against the configured Responses endpoint.
    ///
    /// The stream emits `TextDelta` / `ThinkingDelta` / `ToolUseStarted`
    /// / `TextBlockCompleted` / `ThinkingBlockCompleted` /
    /// `ToolUseCompleted` events, terminated by exactly one
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
        let body = encode::encode_request(&input, &self.cfg);
        decode::stream_response(&self.client, &self.cfg, body, ctx).await
    }

    fn provider_id(&self) -> &'static str {
        "openai-responses"
    }

    fn model_limits(&self) -> ModelLimits {
        let model_id = &self.cfg.model;
        let window = parse_context_window_suffix(model_id)
            .or(self.cfg.context_window_tokens)
            // Responses-eligible models (o1, o3, gpt-4o) generally
            // support 128k. Conservative default until provider config
            // pins the value explicitly.
            .unwrap_or(128_000);
        ModelLimits::new(model_id.clone(), window)
    }
}

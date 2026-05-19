//! `OpenAiCompatGateway` — stub placeholder for Sprint 3+.
//!
//! Only `OpenAiCompatConfig` and `OpenAiCompatGateway` types are re-exported
//! from the crate root; no `ModelGateway` impl is wired here yet.

use std::time::Duration;

use cogito_protocol::gateway::{ModelError, ModelEvent, ModelGateway, ModelInput};
use cogito_protocol::ExecCtx;
use futures::stream::BoxStream;
use reqwest::Client;

/// Static configuration for `OpenAiCompatGateway`.
#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    /// Bearer token for the `Authorization` header.
    pub api_key: String,
    /// Base URL, e.g. `https://api.openai.com` or a custom endpoint.
    pub base_url: String,
    /// Per-request timeout. Default: 5 minutes.
    pub timeout: Duration,
}

impl OpenAiCompatConfig {
    /// Sensible defaults; caller provides only `api_key`.
    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.openai.com".into(),
            timeout: Duration::from_secs(5 * 60),
        }
    }
}

/// `ModelGateway` impl for OpenAI-compatible endpoints (stub; Sprint 3+).
pub struct OpenAiCompatGateway {
    _cfg: OpenAiCompatConfig,
    _client: Client,
}

impl OpenAiCompatGateway {
    /// Build a gateway.
    ///
    /// # Errors
    ///
    /// Returns `ModelError::Network` if the underlying reqwest client
    /// cannot be constructed.
    pub fn new(cfg: OpenAiCompatConfig) -> Result<Self, ModelError> {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| crate::error::from_reqwest(&e))?;
        Ok(Self { _cfg: cfg, _client: client })
    }
}

#[async_trait::async_trait]
impl ModelGateway for OpenAiCompatGateway {
    async fn stream(
        &self,
        _input: ModelInput,
        _ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        Err(ModelError::Network("OpenAI-compat gateway not yet implemented".into()))
    }

    fn provider_id(&self) -> &'static str {
        "openai_compat"
    }
}

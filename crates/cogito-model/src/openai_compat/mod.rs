//! `OpenAiCompatGateway` — implements `ModelGateway` against any
//! OpenAI-compatible Chat Completions endpoint (vLLM / SGLang / Azure
//! OpenAI / internal LLM gateway).
//!
//! NOT the OpenAI Responses API — that arrives in Sprint 5.

pub mod decode;
pub mod encode;
pub mod wire;

use std::time::Duration;

use cogito_protocol::gateway::ModelError;
use reqwest::Client;

/// Configuration for an OpenAI-Compatible endpoint.
///
/// `base_url` is required; auth header naming and scheme are configurable for
/// private deployments that diverge from the OpenAI defaults.
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

// `impl ModelGateway` added in Task 4.3 (see below).

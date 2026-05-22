//! `ProviderConfig` — declarative description of a `ModelGateway`
//! instance. The single source of truth for the
//! `(connection-endpoint, auth, model-family)` triple that surfaces
//! (`cogito-cli`, future `cogito-tui`, consumer Server) read from
//! configuration files / environment / databases.
//!
//! See ADR-0017 §4 for the schema decision and CLAUDE.md
//! §"Coding standards" for the "tagged-config factories belong in the
//! crate that owns the implementations" rule.

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::gateway::{ModelError, ModelGateway};
use serde::{Deserialize, Serialize};

use crate::{AnthropicConfig, AnthropicGateway, OpenAiCompatConfig, OpenAiCompatGateway};

/// Provider configuration: a tagged-union over the gateway kinds
/// `cogito-model` knows how to construct. `kind` is the serde tag.
///
/// Serializes as flat TOML/JSON with `kind` as a discriminator field;
/// kebab-case to match `cogito.toml` conventions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderConfig {
    /// Anthropic Messages API endpoint. `base_url` defaults to the
    /// public endpoint; override for Anthropic-compatible third-party
    /// services.
    Anthropic {
        /// Provider entry name (used by surfaces for `--provider <name>` lookup).
        name: String,
        /// `x-api-key` header value.
        api_key: String,
        /// Base URL. Defaults to `https://api.anthropic.com`.
        #[serde(default = "defaults::anthropic_base_url")]
        base_url: String,
        /// `anthropic-version` header. Defaults to `2023-06-01`.
        #[serde(default = "defaults::anthropic_version")]
        anthropic_version: String,
        /// Per-request timeout in seconds. `None` keeps the gateway default
        /// (5 minutes).
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    /// OpenAI-compatible Chat Completions endpoint (vLLM, `SGLang`, Azure,
    /// internal LLM gateways). Required `base_url`; optional `api_key`
    /// (`None` skips the auth header for unauthenticated deployments).
    #[serde(rename = "openai-compat")]
    OpenAiCompat {
        /// Provider entry name (used by surfaces for `--provider <name>` lookup).
        name: String,
        /// Bearer credential (or equivalent). `None` omits the auth header.
        #[serde(default)]
        api_key: Option<String>,
        /// Required base URL, e.g. `http://vllm:8000/v1`.
        base_url: String,
        /// HTTP header carrying the credential. Defaults to `Authorization`.
        #[serde(default = "defaults::auth_header")]
        auth_header: String,
        /// Scheme prefix prepended to `api_key`. Defaults to `Bearer`.
        #[serde(default = "defaults::auth_scheme")]
        auth_scheme: String,
        /// Per-request timeout in seconds. `None` keeps the gateway default
        /// (5 minutes).
        #[serde(default)]
        timeout_secs: Option<u64>,
        /// Whether to re-feed prior-turn `ContentBlock::Thinking` blocks
        /// back into outgoing messages. Most open-source reasoning models
        /// (DeepSeek-R1, QwQ) explicitly drop prior thinking on follow-up
        /// turns; default `false` matches that convention. Set `true`
        /// only if the backend model is documented to handle prior
        /// `<think>` context. See ADR-0019 §5.3.
        #[serde(default)]
        include_prior_thinking: bool,
    },
    // OpenAiResponses { ... } lands in Sprint 5 — single-arm addition.
}

impl ProviderConfig {
    /// The configured `name` for this provider entry (used by surfaces
    /// for `--provider <name>` lookup).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic { name, .. } | Self::OpenAiCompat { name, .. } => name,
        }
    }
}

/// Build a concrete `ModelGateway` from a `ProviderConfig`. This is the
/// only place in the workspace that pattern-matches on `kind`; surfaces
/// must call this function rather than reproducing the dispatch table.
///
/// # Errors
///
/// Forwards `ModelError` from the underlying gateway constructors
/// (TLS / client builder failures; rare).
pub fn build_gateway(cfg: ProviderConfig) -> Result<Arc<dyn ModelGateway>, ModelError> {
    match cfg {
        ProviderConfig::Anthropic {
            api_key,
            base_url,
            anthropic_version,
            timeout_secs,
            ..
        } => {
            let mut c = AnthropicConfig::with_api_key(api_key);
            c.base_url = base_url;
            c.anthropic_version = anthropic_version;
            if let Some(s) = timeout_secs {
                c.timeout = Duration::from_secs(s);
            }
            Ok(Arc::new(AnthropicGateway::new(c)?))
        }
        ProviderConfig::OpenAiCompat {
            api_key,
            base_url,
            auth_header,
            auth_scheme,
            timeout_secs,
            include_prior_thinking,
            ..
        } => {
            let mut c = OpenAiCompatConfig::with_base_url(base_url);
            c.api_key = api_key;
            c.auth_header = auth_header;
            c.auth_scheme = auth_scheme;
            c.include_prior_thinking = include_prior_thinking;
            if let Some(s) = timeout_secs {
                c.timeout = Duration::from_secs(s);
            }
            Ok(Arc::new(OpenAiCompatGateway::new(c)?))
        }
    }
}

mod defaults {
    pub(super) fn anthropic_base_url() -> String {
        "https://api.anthropic.com".into()
    }
    pub(super) fn anthropic_version() -> String {
        "2023-06-01".into()
    }
    pub(super) fn auth_header() -> String {
        "Authorization".into()
    }
    pub(super) fn auth_scheme() -> String {
        "Bearer".into()
    }
}

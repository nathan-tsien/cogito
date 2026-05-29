//! `web_fetch` — fetch an http(s) URL and return its content as markdown
//! (HTML) or text. Synchronous `BuiltinTool`. Does NOT call any model
//! (stays provider-free); URL/SSRF gating is an H09 hook concern.

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Tunables for `web_fetch`. Lives here (owning crate); aggregated into
/// `cogito-config`'s `[tools]` section.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WebFetchConfig {
    /// Per-request timeout, seconds.
    pub timeout_secs: u64,
    /// Maximum response body bytes read before truncation.
    pub max_bytes: usize,
    /// `User-Agent` header.
    pub user_agent: String,
    /// Maximum redirects to follow.
    pub max_redirects: usize,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_bytes: 1 << 20,
            user_agent: "cogito/0.1".into(),
            max_redirects: 5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    url: String,
}

/// HTML-to-markdown fetcher.
#[derive(Debug, Clone)]
pub struct WebFetch {
    // read by fetch() in Task 9
    #[allow(dead_code)]
    cfg: WebFetchConfig,
}

impl WebFetch {
    /// Construct from config.
    #[must_use]
    pub fn new(cfg: WebFetchConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl BuiltinTool for WebFetch {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "web_fetch".into(),
            description:
                "Fetch an http(s) URL. HTML is converted to Markdown; other text is returned as-is."
                    .into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "http(s) URL to fetch." }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, _ctx: ExecCtx) -> ToolResult {
        let Args { url } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("web_fetch args: {e}"),
                    retryable: false,
                };
            }
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("web_fetch: only http(s) URLs are supported, got: {url}"),
                retryable: false,
            };
        }
        self.fetch(&url).await
    }
}

impl WebFetch {
    // stub has no await; real fetch lands in Task 9
    #[allow(clippy::unused_async)]
    async fn fetch(&self, _url: &str) -> ToolResult {
        ToolResult::text("todo")
    }
}

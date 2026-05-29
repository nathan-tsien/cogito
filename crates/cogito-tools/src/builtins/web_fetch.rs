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
    async fn fetch(&self, url: &str) -> ToolResult {
        use futures::StreamExt as _;

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_secs))
            .user_agent(self.cfg.user_agent.clone())
            .redirect(reqwest::redirect::Policy::limited(self.cfg.max_redirects))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("web_fetch: client build failed: {e}"),
                    retryable: false,
                };
            }
        };

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("web_fetch: request failed: {e}"),
                    retryable: true,
                };
            }
        };

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_html = content_type.contains("text/html");
        let is_text = content_type.starts_with("text/")
            || content_type.contains("json")
            || content_type.contains("xml");
        if !is_html && !is_text {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("web_fetch: unsupported content-type: {content_type}"),
                retryable: false,
            };
        }

        // Read the body with a hard byte cap.
        let mut body: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    body.extend_from_slice(&bytes);
                    if body.len() >= self.cfg.max_bytes {
                        body.truncate(self.cfg.max_bytes);
                        truncated = true;
                        break;
                    }
                }
                Err(e) => {
                    return ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: format!("web_fetch: body read failed: {e}"),
                        retryable: true,
                    };
                }
            }
        }
        let text = String::from_utf8_lossy(&body).to_string();

        // Produce the final string for the matching content kind, then tell
        // the model when the body was cut at the byte cap so it does not treat
        // the output as complete.
        let mut out = if is_html {
            match htmd::convert(&text) {
                Ok(md) => md,
                Err(e) => {
                    return ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: format!("web_fetch: html->markdown failed: {e}"),
                        retryable: false,
                    };
                }
            }
        } else {
            text
        };
        if truncated {
            use std::fmt::Write as _;
            // Ignore the formatting error: writing into a `String` never fails.
            let _ = write!(
                out,
                "\n\n[web_fetch: output truncated at {} bytes]",
                self.cfg.max_bytes
            );
        }
        ToolResult::text(out)
    }
}

//! Build per-server transports (stdio child process or streamable-HTTP
//! client). Each builder returns either a ready transport or an
//! `McpStartupFailure` recording why the build failed.
//!
//! For streamable-HTTP we delegate reqwest client construction to
//! `rmcp::transport::StreamableHttpClientTransport::from_config`. rmcp
//! 1.7 vendors its own reqwest major (0.13) and the trait
//! `StreamableHttpClient` is implemented only for *its* `reqwest::Client`,
//! so we cannot construct one ourselves from the workspace's reqwest 0.12
//! and pass it in. Custom HTTP headers are typed through the `http` crate
//! (`HeaderName` / `HeaderValue`), which is the type rmcp's config exposes.
//!
//! Items are reached only from `client::handshake_and_list`, which is
//! consumed by the factory (Task 10). Until then, dead-code lints are
//! suppressed at the module level.
#![allow(dead_code)]

use std::collections::HashMap;
use std::process::Stdio;

use http::{HeaderName, HeaderValue};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::McpTransportConfig;
use crate::error::McpStartupFailure;

/// Discriminated transport ready to be handed to `rmcp::service::serve_client`.
///
/// `StreamableHttp` is parameterised by rmcp's internal `reqwest::Client`
/// (0.13.x), not the workspace `reqwest` (0.12.x). The `from_config`
/// constructor handles that for us.
pub(crate) enum BuiltTransport {
    ChildProcess(TokioChildProcess),
    StreamableHttp(StreamableHttpClientTransport<reqwest::Client>),
}

/// Build a transport from a [`McpTransportConfig`]. The `server_name`
/// is used only to annotate failures.
pub(crate) fn build_transport(
    server_name: &str,
    cfg: &McpTransportConfig,
) -> Result<BuiltTransport, McpStartupFailure> {
    match cfg {
        McpTransportConfig::Stdio { command, args, env } => {
            build_stdio(server_name, command, args, env.as_ref())
        }
        McpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
        } => build_streamable_http(
            server_name,
            url,
            bearer_token_env_var.as_deref(),
            http_headers.as_ref(),
        ),
    }
}

fn build_stdio(
    server_name: &str,
    command: &str,
    args: &[String],
    env: Option<&HashMap<String, String>>,
) -> Result<BuiltTransport, McpStartupFailure> {
    let mut cmd = Command::new(command);
    cmd.kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(args);
    if let Some(env_map) = env {
        for (k, v) in env_map {
            cmd.env(k, v);
        }
    }

    let (transport, stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| McpStartupFailure::TransportError {
            name: server_name.to_string(),
            error: format!("spawn `{command}`: {e}"),
        })?;

    if let Some(stderr) = stderr {
        let name = server_name.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        info!(mcp.server = %name, "stderr: {line}");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        warn!(mcp.server = %name, "stderr read failed: {err}");
                        break;
                    }
                }
            }
        });
    }

    Ok(BuiltTransport::ChildProcess(transport))
}

fn build_streamable_http(
    server_name: &str,
    url: &str,
    bearer_token_env_var: Option<&str>,
    http_headers: Option<&HashMap<String, String>>,
) -> Result<BuiltTransport, McpStartupFailure> {
    // Resolve bearer token from env. Missing or empty env → soft fail
    // (McpStartupFailure::BearerEnvMissing) so the Runtime can keep
    // booting without this server (ADR-0018 §3).
    let bearer = if let Some(env_var) = bearer_token_env_var {
        match std::env::var(env_var) {
            Ok(v) if !v.trim().is_empty() => Some(v),
            _ => {
                return Err(McpStartupFailure::BearerEnvMissing {
                    name: server_name.to_string(),
                    env_var: env_var.to_string(),
                });
            }
        }
    } else {
        None
    };

    // Parse static headers up front so misconfiguration surfaces before
    // we hand the config to rmcp.
    let mut typed_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
    if let Some(headers) = http_headers {
        for (k, v) in headers {
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                McpStartupFailure::TransportError {
                    name: server_name.to_string(),
                    error: format!("invalid header name `{k}`: {e}"),
                }
            })?;
            let value =
                HeaderValue::from_str(v).map_err(|e| McpStartupFailure::TransportError {
                    name: server_name.to_string(),
                    // Note: do NOT echo `v` — it might contain a secret if
                    // the user wired a sensitive value through static
                    // headers. Echo only the key.
                    error: format!("invalid header value for `{k}`: {e}"),
                })?;
            typed_headers.insert(name, value);
        }
    }

    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(token) = bearer {
        config = config.auth_header(token);
    }
    if !typed_headers.is_empty() {
        config = config.custom_headers(typed_headers);
    }
    // `from_config` constructs rmcp's internal reqwest client (matching
    // its expected `StreamableHttpClient` impl) — we don't need to wire
    // one up ourselves.
    let transport = StreamableHttpClientTransport::from_config(config);
    Ok(BuiltTransport::StreamableHttp(transport))
}

//! Integration tests for `web_fetch` against a minimal local HTTP server
//! (raw `TcpListener`, no extra deps).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::builtins::web_fetch::{WebFetch, WebFetchConfig};
use cogito_tools::provider::BuiltinTool;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// Spawn a one-shot server that replies with a fixed `content_type` + body.
/// Returns the bound `http://127.0.0.1:<port>/` URL.
async fn serve_once(content_type: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // drain the request line/headers
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        }
    });
    format!("http://{addr}/")
}

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

/// Extract the text payload from `ToolResult`. The text constructor builds
/// `ToolResult::Output(vec![Value::String(..)])`, so concatenate any string
/// blocks back into a single `String`.
fn text_of(r: &ToolResult) -> String {
    match r {
        ToolResult::Output(blocks) => blocks
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(""),
        other => panic!("expected Output, got {other:?}"),
    }
}

#[tokio::test]
async fn html_is_converted_to_markdown() {
    let url = serve_once("text/html; charset=utf-8", "<h1>Title</h1><p>Body text</p>").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    let md = text_of(&out);
    assert!(
        md.contains("Title"),
        "markdown should keep the heading text: {md:?}"
    );
    assert!(
        md.contains("Body text"),
        "markdown should keep body: {md:?}"
    );
    assert!(!md.contains("<h1>"), "raw HTML tags must be gone: {md:?}");
}

#[tokio::test]
async fn plain_text_passes_through() {
    let url = serve_once("text/plain", "hello world").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    assert!(text_of(&out).contains("hello world"));
}

#[tokio::test]
async fn binary_content_type_is_rejected() {
    let url = serve_once("image/png", "PNG-binary-bytes").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    assert!(matches!(
        out,
        ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            ..
        }
    ));
}

#[tokio::test]
async fn non_http_scheme_is_rejected() {
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool
        .invoke(serde_json::json!({ "url": "file:///etc/passwd" }), ctx())
        .await;
    assert!(matches!(
        out,
        ToolResult::Error {
            kind: ToolErrorKind::InvalidArgs,
            ..
        }
    ));
}

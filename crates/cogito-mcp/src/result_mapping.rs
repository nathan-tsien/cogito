//! Map an `rmcp::model::CallToolResult` to cogito's `ToolResult`.
//!
//! See ADR-0018 §5 for the mapping table. v0.1 collapses image /
//! resource content blocks into JSON objects; the multimodal upgrade
//! (ADR-0009 in v0.2) will swap `Output(Vec<serde_json::Value>)`
//! for `Output(Vec<ContentBlock>)` and unblock visual model awareness.

use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use rmcp::model::{CallToolResult, Content, RawContent};
use serde_json::{Value, json};

/// Convert an rmcp `CallToolResult` into a cogito `ToolResult`.
///
/// - `is_error: true` → [`ToolResult::Error`] with
///   [`ToolErrorKind::InvocationFailed`] (conservative
///   `retryable: false`; we don't know the server's state).
/// - `is_error: false` → [`ToolResult::Output`] with one JSON value
///   per content block. Text blocks become JSON strings; image /
///   resource blocks become tagged JSON objects (`{"kind": "image",
///   ...}` etc.) for v0.1; v0.2 multimodal upgrade will preserve
///   them as native `ContentBlock`s.
/// - When `structured_content` is present (non-error case), append a
///   `{"kind": "structured", "data": ...}` element.
#[must_use]
pub fn to_cogito_result(call: CallToolResult) -> ToolResult {
    let is_error = call.is_error.unwrap_or(false);

    if is_error {
        let message = join_text_blocks(&call.content);
        return ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            message: if message.is_empty() {
                "MCP server returned is_error=true with no message".into()
            } else {
                message
            },
            retryable: false,
        };
    }

    let mut output: Vec<Value> = call
        .content
        .into_iter()
        .map(content_block_to_json)
        .collect();

    if let Some(structured) = call.structured_content {
        output.push(json!({
            "kind": "structured",
            "data": structured,
        }));
    }

    ToolResult::Output(output)
}

fn join_text_blocks(blocks: &[Content]) -> String {
    let mut out = String::new();
    for block in blocks {
        if let RawContent::Text(t) = &block.raw {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t.text);
        }
    }
    out
}

fn content_block_to_json(block: Content) -> Value {
    // The `other` arm currently only matches `ResourceLink`, but rmcp's
    // `RawContent` enum has grown a new variant in every recent minor
    // release. Keep the catchall so future variants degrade gracefully
    // instead of becoming a compile error on the next `cargo update`.
    #[allow(clippy::match_wildcard_for_single_variants)]
    match block.raw {
        RawContent::Text(t) => Value::String(t.text),
        RawContent::Image(i) => json!({
            "kind": "image",
            "mime_type": i.mime_type,
            "data": i.data,
        }),
        RawContent::Resource(r) => json!({
            "kind": "resource",
            "resource": serde_json::to_value(r).unwrap_or(Value::Null),
        }),
        RawContent::Audio(a) => json!({
            "kind": "audio",
            "mime_type": a.mime_type,
            "data": a.data,
        }),
        // Future rmcp variants land here; emit a tagged unknown so
        // future-tool authors can spot the variant.
        other => json!({
            "kind": "unknown",
            "debug": format!("{other:?}"),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn text_block(s: &str) -> Content {
        Content::text(s)
    }

    #[test]
    fn is_error_true_maps_to_invocation_failed() {
        let call = CallToolResult::error(vec![text_block("boom")]);
        match to_cogito_result(call) {
            ToolResult::Error {
                kind,
                message,
                retryable,
            } => {
                assert!(matches!(kind, ToolErrorKind::InvocationFailed));
                assert_eq!(message, "boom");
                assert!(!retryable);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn is_error_true_with_empty_content_yields_default_message() {
        let call = CallToolResult::error(vec![]);
        let result = to_cogito_result(call);
        let ToolResult::Error { message, .. } = result else {
            panic!("expected Error");
        };
        assert!(message.contains("no message"));
    }

    #[test]
    fn multi_text_blocks_concatenate_with_newline_for_error() {
        let call = CallToolResult::error(vec![text_block("line one"), text_block("line two")]);
        let ToolResult::Error { message, .. } = to_cogito_result(call) else {
            panic!("expected Error");
        };
        assert_eq!(message, "line one\nline two");
    }

    #[test]
    fn single_text_output_maps_to_output_with_one_string() {
        let call = CallToolResult::success(vec![text_block("hello world")]);
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], Value::String("hello world".into()));
    }

    #[test]
    fn missing_is_error_defaults_to_success() {
        // CallToolResult is #[non_exhaustive]; construct via the
        // public `success` constructor then null out `is_error` to
        // simulate a server response that omits the field.
        let mut call = CallToolResult::success(vec![text_block("ok")]);
        call.is_error = None;
        let ToolResult::Output(_) = to_cogito_result(call) else {
            panic!("expected Output, got Error");
        };
    }

    #[test]
    fn image_block_serializes_to_tagged_json_object() {
        // Use Content::image constructor; the data/mime_type pair
        // mirrors what a server would actually emit.
        let img = Content::image("BASE64DATA", "image/png");
        let call = CallToolResult::success(vec![img]);
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 1);
        let obj = &v[0];
        assert_eq!(obj["kind"], "image");
        assert_eq!(obj["mime_type"], "image/png");
        assert_eq!(obj["data"], "BASE64DATA");
    }

    #[test]
    fn structured_content_appends_extra_element() {
        let mut call = CallToolResult::success(vec![text_block("hi")]);
        call.structured_content = Some(json!({"count": 3}));
        let ToolResult::Output(v) = to_cogito_result(call) else {
            panic!("expected Output");
        };
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Value::String("hi".into()));
        assert_eq!(v[1]["kind"], "structured");
        assert_eq!(v[1]["data"]["count"], 3);
    }
}

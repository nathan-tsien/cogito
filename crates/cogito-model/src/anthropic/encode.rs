//! `ModelInput` → Anthropic request body.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::tool::ToolResult;

use super::wire::{Request, RequestContentBlock, RequestMessage, RequestTool};

/// Encode a `ModelInput` into the Anthropic request body.
pub(crate) fn encode(input: ModelInput) -> Request {
    let mut messages = Vec::with_capacity(input.messages.len());
    for m in input.messages {
        messages.push(match m {
            Message::User { content } => RequestMessage {
                role: "user".into(),
                content: content.into_iter().map(encode_block).collect(),
            },
            Message::Assistant { content } => RequestMessage {
                role: "assistant".into(),
                content: content.into_iter().map(encode_block).collect(),
            },
        });
    }
    Request {
        model: input.params.model,
        max_tokens: input.params.max_tokens,
        temperature: input.params.temperature,
        top_p: input.params.top_p,
        stop_sequences: input.params.stop_sequences,
        stream: true,
        system: input.system,
        messages,
        tools: input.tools.into_iter().map(encode_tool).collect(),
    }
}

fn encode_block(b: ContentBlock) -> RequestContentBlock {
    match b {
        ContentBlock::Text { text } => RequestContentBlock::Text { text },
        ContentBlock::ToolUse {
            call_id,
            tool_name,
            args,
        } => RequestContentBlock::ToolUse {
            id: call_id,
            name: tool_name,
            input: args,
        },
        ContentBlock::ToolResult { call_id, result } => match result {
            ToolResult::Output(values) => {
                // Flatten JSON values to text: string scalars pass through,
                // other values are serialized to their JSON representation.
                let text = values
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                RequestContentBlock::ToolResult {
                    tool_use_id: call_id,
                    content: text,
                    is_error: Some(false),
                }
            }
            ToolResult::Error { message, .. } => RequestContentBlock::ToolResult {
                tool_use_id: call_id,
                content: message,
                is_error: Some(true),
            },
            // `ToolResult` is `#[non_exhaustive]`; handle any future variants
            // gracefully by treating them as empty successful output.
            _ => RequestContentBlock::ToolResult {
                tool_use_id: call_id,
                content: String::new(),
                is_error: Some(false),
            },
        },
        ContentBlock::Thinking {
            text,
            provider_opaque,
        } => {
            // Disambiguate plain vs redacted by which key is present
            // in provider_opaque. Per ADR-0019 §5.1: decode.rs packs
            // {signature} for plain, {data} for redacted_thinking.
            let opaque = provider_opaque.as_ref();
            let redacted_data = opaque
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_str())
                .map(String::from);
            if let Some(data) = redacted_data {
                RequestContentBlock::RedactedThinking { data }
            } else {
                let signature = opaque
                    .and_then(|v| v.get("signature"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                RequestContentBlock::Thinking {
                    thinking: text,
                    signature,
                }
            }
        }
        // `ContentBlock` is `#[non_exhaustive]`; future variants (e.g. Image)
        // are silently dropped until the encoder is extended in v0.2+.
        _ => RequestContentBlock::Text {
            text: String::new(),
        },
    }
}

fn encode_tool(d: cogito_protocol::tool::ToolDescriptor) -> RequestTool {
    RequestTool {
        name: d.name,
        description: d.description,
        input_schema: d.schema,
    }
}

#[cfg(test)]
mod thinking_encode_tests {
    use super::*;
    use cogito_protocol::content::ContentBlock;

    #[test]
    fn encodes_thinking_block_with_signature() {
        let block = encode_block(ContentBlock::Thinking {
            text: "I should grep.".into(),
            provider_opaque: Some(serde_json::json!({"signature": "sig_xyz"})),
        });
        let json = serde_json::to_value(&block).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("serialize failed: {e}")
            }
        });
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("thinking"));
        assert_eq!(
            json.get("thinking").and_then(|v| v.as_str()),
            Some("I should grep.")
        );
        assert_eq!(
            json.get("signature").and_then(|v| v.as_str()),
            Some("sig_xyz")
        );
    }

    #[test]
    fn encodes_redacted_thinking_block() {
        let block = encode_block(ContentBlock::Thinking {
            text: String::new(),
            provider_opaque: Some(serde_json::json!({"data": "enc_blob"})),
        });
        let json = serde_json::to_value(&block).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("serialize failed: {e}")
            }
        });
        assert_eq!(
            json.get("type").and_then(|v| v.as_str()),
            Some("redacted_thinking")
        );
        assert_eq!(json.get("data").and_then(|v| v.as_str()), Some("enc_blob"));
        assert!(
            json.get("thinking").is_none(),
            "redacted must not emit a `thinking` key"
        );
        assert!(
            json.get("signature").is_none(),
            "redacted must not emit a `signature` key"
        );
    }

    #[test]
    fn encodes_thinking_block_missing_opaque_falls_back_to_plain_thinking_without_signature() {
        // Defensive: provider_opaque: None or missing both keys → emit
        // plain thinking with just the text (no signature field). Better
        // than silently converting to Text — even malformed thinking
        // should stay structurally a thinking block.
        let block = encode_block(ContentBlock::Thinking {
            text: "implicit".into(),
            provider_opaque: None,
        });
        let json = serde_json::to_value(&block).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("serialize failed: {e}")
            }
        });
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("thinking"));
        assert_eq!(
            json.get("thinking").and_then(|v| v.as_str()),
            Some("implicit")
        );
        assert!(json.get("signature").is_none());
    }
}

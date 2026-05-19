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

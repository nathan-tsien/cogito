//! `ModelInput` → `OpenAI` Chat Completions request body.
//!
//! Key transform: cogito's `Message::User { content: [ToolResult …] }` blocks
//! must be split into independent `{role: "tool", tool_call_id, content}` wire
//! messages placed immediately after the assistant message that requested them.
//! Plain user text becomes a single `{role: "user", content}` message.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::tool::ToolResult;

use super::wire::{Request, RequestMessage, RequestTool, ToolCall, ToolCallFunction, ToolDef};

/// Encode a `ModelInput` into an `OpenAI` Chat Completions request body.
pub(crate) fn encode(input: ModelInput, include_prior_thinking: bool) -> Request {
    let mut messages = Vec::new();
    for m in input.messages {
        match m {
            Message::User { content } => encode_user(content, &mut messages),
            Message::Assistant { content } => {
                encode_assistant(content, include_prior_thinking, &mut messages);
            }
        }
    }
    Request {
        model: input.params.model,
        messages,
        max_tokens: input.params.max_tokens,
        temperature: input.params.temperature,
        top_p: input.params.top_p,
        stop: input.params.stop_sequences,
        stream: true,
        tools: input.tools.into_iter().map(encode_tool).collect(),
    }
}

/// Encode a user-role message.
///
/// Text blocks become one `{role: "user"}` wire message; `ToolResult` blocks
/// become individual `{role: "tool", tool_call_id}` messages.  The order
/// preserves the original cogito ordering: text first, then tool results.
fn encode_user(content: Vec<ContentBlock>, out: &mut Vec<RequestMessage>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_results: Vec<(String, String)> = Vec::new();

    for b in content {
        match b {
            ContentBlock::Text { text } => text_parts.push(text),
            ContentBlock::ToolResult { call_id, result } => {
                // Flatten result to a plain string for the wire format.
                let body = match result {
                    ToolResult::Output(values) => values
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    ToolResult::Error { message, .. } => message,
                    // `ToolResult` is `#[non_exhaustive]`; future variants
                    // treated as empty successful output until encoder extends.
                    _ => String::new(),
                };
                tool_results.push((call_id, body));
            }
            // `ToolUse` inside a User message should not occur in v0.1; other
            // `ContentBlock` variants (Image etc.) arrive in v0.2+.  Silently
            // drop unknown variants to stay forward-compatible.
            _ => {}
        }
    }

    if !text_parts.is_empty() {
        out.push(RequestMessage {
            role: "user".into(),
            content: Some(text_parts.join("\n")),
            tool_call_id: None,
            tool_calls: vec![],
        });
    }

    for (id, body) in tool_results {
        out.push(RequestMessage {
            role: "tool".into(),
            content: Some(body),
            tool_call_id: Some(id),
            tool_calls: vec![],
        });
    }
}

/// Encode an assistant-role message.
///
/// Text blocks are joined into the `content` field; `ToolUse` blocks become
/// `tool_calls` entries.  An empty `content` is omitted (per the spec).
fn encode_assistant(
    content: Vec<ContentBlock>,
    include_prior_thinking: bool,
    out: &mut Vec<RequestMessage>,
) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for b in content {
        match b {
            ContentBlock::Text { text } => text_parts.push(text),
            ContentBlock::ToolUse {
                call_id,
                tool_name,
                args,
            } => {
                // `serde_json::to_string` on a `Value` is infallible in
                // practice; the closure default guards the rare edge case.
                let arguments = serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                tool_calls.push(ToolCall {
                    id: call_id,
                    kind: "function".into(),
                    function: ToolCallFunction {
                        name: tool_name,
                        arguments,
                    },
                });
            }
            ContentBlock::Thinking { text, .. } => {
                // Default behavior (ADR-0019 §5.3): drop prior thinking
                // blocks — DeepSeek-R1 / QwQ are trained to think fresh
                // each turn. Operators opt in via
                // `provider.<name>.include_prior_thinking = true` for
                // backends that benefit from carrying prior reasoning.
                if include_prior_thinking {
                    text_parts.push(format!("<think>{text}</think>"));
                }
                // else: drop silently.
            }
            // `ToolResult` inside an Assistant message is not valid; other
            // future variants silently dropped for forward compatibility.
            _ => {}
        }
    }

    out.push(RequestMessage {
        role: "assistant".into(),
        content: if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n"))
        },
        tool_call_id: None,
        tool_calls,
    });
}

/// Encode a `ToolDescriptor` as an OpenAI-style function tool definition.
fn encode_tool(d: cogito_protocol::tool::ToolDescriptor) -> RequestTool {
    RequestTool {
        kind: "function".into(),
        function: ToolDef {
            name: d.name,
            description: d.description,
            parameters: d.schema,
        },
    }
}

#[cfg(test)]
mod thinking_encode_tests {
    use super::*;
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::gateway::{Message, ModelInput, ModelParams};

    fn input_with_thinking() -> ModelInput {
        ModelInput {
            system: String::new(),
            messages: vec![Message::Assistant {
                content: vec![
                    ContentBlock::Thinking {
                        text: "I should grep.".into(),
                        provider_opaque: None,
                    },
                    ContentBlock::Text { text: "OK.".into() },
                ],
            }],
            tools: Vec::new(),
            params: ModelParams {
                model: "test-model".into(),
                max_tokens: 100,
                temperature: None,
                top_p: None,
                stop_sequences: Vec::new(),
            },
        }
    }

    #[test]
    fn encode_default_drops_prior_thinking() {
        let req = encode(
            input_with_thinking(),
            /*include_prior_thinking=*/ false,
        );
        let json = serde_json::to_string(&req).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("serialize failed: {e}")
            }
        });
        assert!(!json.contains("<think>"), "no <think> tags: {json}");
        assert!(
            !json.contains("I should grep."),
            "thinking text must be dropped: {json}"
        );
        assert!(
            json.contains("OK."),
            "non-thinking text must remain: {json}"
        );
    }

    #[test]
    fn encode_include_prior_thinking_wraps_in_tags() {
        let req = encode(input_with_thinking(), /*include_prior_thinking=*/ true);
        let json = serde_json::to_string(&req).unwrap_or_else(|e| {
            #[allow(clippy::panic)]
            {
                panic!("serialize failed: {e}")
            }
        });
        assert!(
            json.contains(r"<think>I should grep.</think>"),
            "tags must wrap thinking: {json}"
        );
        assert!(
            json.contains("OK."),
            "non-thinking text must remain: {json}"
        );
    }
}

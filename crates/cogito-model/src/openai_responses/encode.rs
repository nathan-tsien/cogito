//! `ContentBlock` → Responses input items.
//!
//! Mapping rules:
//! - `ContentBlock::Text` in a user turn → `InputItem::Message { role: "user", content: [InputText] }`
//! - `ContentBlock::Text` in an assistant turn → `Message { role: "assistant", content: [OutputText] }`
//! - `ContentBlock::ToolUse` → `FunctionCall { call_id, name, arguments }`
//! - `ContentBlock::ToolResult` → `FunctionCallOutput { call_id, output }`
//! - `ContentBlock::Thinking` → `Reasoning { summary: [...] }` (re-feeds prior reasoning per ADR-0019 §5.4)
//! - System prompt → `instructions` field on the request

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{Message, ModelInput};
use cogito_protocol::tool::{ToolDescriptor, ToolResult};

use super::OpenAiResponsesConfig;
use super::wire::{
    InputItem, MessageContent, ReasoningParams, ReasoningSummary, ResponsesRequest, ToolDef,
};

/// Encode a `ModelInput` into a Responses API request body.
pub(crate) fn encode_request(input: &ModelInput, cfg: &OpenAiResponsesConfig) -> ResponsesRequest {
    let mut items: Vec<InputItem> = Vec::new();
    for msg in &input.messages {
        encode_message(msg, &mut items);
    }

    let tools = input.tools.iter().cloned().map(encode_tool).collect();

    let reasoning = cfg
        .reasoning_effort
        .map(|effort| ReasoningParams { effort });

    let instructions = if input.system.is_empty() {
        None
    } else {
        Some(input.system.clone())
    };

    ResponsesRequest {
        model: input.params.model.clone(),
        input: items,
        stream: true,
        max_output_tokens: Some(input.params.max_tokens),
        temperature: input.params.temperature,
        top_p: input.params.top_p,
        tools,
        reasoning,
        instructions,
    }
}

/// Encode one cogito `Message` into zero or more `InputItem`s.
///
/// Text and Thinking blocks within an assistant message are flushed as
/// they appear so the resulting flat array preserves the original
/// `[Thinking, Text, ToolUse]` ordering (ADR-0019 §4). User messages
/// always emit text and tool results in distinct items.
fn encode_message(msg: &Message, out: &mut Vec<InputItem>) {
    match msg {
        Message::User { content } => encode_user(content, out),
        Message::Assistant { content } => encode_assistant(content, out),
    }
}

/// Encode a user-role message.
///
/// Text blocks become one `Message { role: "user" }` wire item per run;
/// `ToolResult` blocks become individual `FunctionCallOutput` items.
fn encode_user(content: &[ContentBlock], out: &mut Vec<InputItem>) {
    let mut text_runs: Vec<MessageContent> = Vec::new();

    let flush = |runs: &mut Vec<MessageContent>, sink: &mut Vec<InputItem>| {
        if !runs.is_empty() {
            sink.push(InputItem::Message {
                role: "user".into(),
                content: std::mem::take(runs),
            });
        }
    };

    for b in content {
        match b {
            ContentBlock::Text { text } => {
                text_runs.push(MessageContent::InputText { text: text.clone() });
            }
            ContentBlock::ToolResult { call_id, result } => {
                flush(&mut text_runs, out);
                let output = flatten_tool_result(result);
                out.push(InputItem::FunctionCallOutput {
                    call_id: call_id.clone(),
                    output,
                });
            }
            // `ToolUse` inside a User message is invalid in v0.1; other
            // future variants (Image, etc.) silently dropped for forward
            // compatibility.
            _ => {}
        }
    }
    flush(&mut text_runs, out);
}

/// Encode an assistant-role message.
///
/// Text blocks become `Message { role: "assistant" }` items with
/// `OutputText` content; `Thinking` blocks become `Reasoning` items
/// (re-feeding prior reasoning per ADR-0019 §5.4); `ToolUse` blocks
/// become `FunctionCall` items.
fn encode_assistant(content: &[ContentBlock], out: &mut Vec<InputItem>) {
    let mut text_runs: Vec<MessageContent> = Vec::new();

    let flush = |runs: &mut Vec<MessageContent>, sink: &mut Vec<InputItem>| {
        if !runs.is_empty() {
            sink.push(InputItem::Message {
                role: "assistant".into(),
                content: std::mem::take(runs),
            });
        }
    };

    for b in content {
        match b {
            ContentBlock::Text { text } => {
                text_runs.push(MessageContent::OutputText { text: text.clone() });
            }
            ContentBlock::ToolUse {
                call_id,
                tool_name,
                args,
            } => {
                flush(&mut text_runs, out);
                // `serde_json::to_string` on a `Value` is infallible in
                // practice; the closure default guards the rare edge case.
                let arguments = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
                out.push(InputItem::FunctionCall {
                    call_id: call_id.clone(),
                    name: tool_name.clone(),
                    arguments,
                });
            }
            ContentBlock::Thinking { text, .. } => {
                flush(&mut text_runs, out);
                out.push(InputItem::Reasoning {
                    summary: vec![ReasoningSummary {
                        kind: "summary_text".into(),
                        text: text.clone(),
                    }],
                });
            }
            // `ToolResult` inside an Assistant message is invalid; other
            // future variants silently dropped for forward compatibility.
            _ => {}
        }
    }
    flush(&mut text_runs, out);
}

/// Flatten a cogito `ToolResult` into the flat string Responses expects
/// for `function_call_output.output`.
///
/// `Output` values that are JSON strings pass through; non-string JSON
/// values fall back to their string-serialized form. Error variants
/// surface their `message` field.
fn flatten_tool_result(result: &ToolResult) -> String {
    match result {
        ToolResult::Output(values) => values
            .iter()
            .map(|v| {
                v.as_str().map_or_else(
                    || serde_json::to_string(v).unwrap_or_default(),
                    String::from,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ToolResult::Error { message, .. } => message.clone(),
        // `ToolResult` is `#[non_exhaustive]`; future variants treated as
        // empty successful output until the encoder is extended.
        _ => String::new(),
    }
}

/// Encode a cogito `ToolDescriptor` as a Responses function-tool definition.
fn encode_tool(d: ToolDescriptor) -> ToolDef {
    ToolDef {
        kind: "function".into(),
        name: d.name,
        description: d.description,
        parameters: d.schema,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cogito_protocol::content::ContentBlock;
    use cogito_protocol::gateway::{Message, ModelInput, ModelParams};

    fn empty_params() -> ModelParams {
        ModelParams {
            model: "test".into(),
            max_tokens: 100,
            temperature: None,
            top_p: None,
            stop_sequences: vec![],
        }
    }

    #[test]
    fn encodes_a_simple_user_message() {
        let input = ModelInput {
            system: String::new(),
            messages: vec![Message::User {
                content: vec![ContentBlock::Text { text: "hi".into() }],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        assert!(req.stream);
        assert!(req.instructions.is_none());
        assert!(req.reasoning.is_none());
        assert_eq!(req.input.len(), 1);
        match &req.input[0] {
            InputItem::Message { role, content } => {
                assert_eq!(role, "user");
                assert!(matches!(
                    content[0],
                    MessageContent::InputText { ref text } if text == "hi"
                ));
            }
            other => panic!("unexpected first item: {other:?}"),
        }
    }

    #[test]
    fn encodes_reasoning_effort_when_set() {
        let input = ModelInput {
            system: "sys".into(),
            messages: vec![],
            tools: vec![],
            params: empty_params(),
        };
        let mut cfg = OpenAiResponsesConfig::with_api_key("k");
        cfg.reasoning_effort = Some(super::super::ReasoningEffort::Medium);
        let req = encode_request(&input, &cfg);
        assert!(req.reasoning.is_some());
        assert_eq!(
            req.reasoning.as_ref().map(|r| r.effort),
            Some(super::super::ReasoningEffort::Medium)
        );
        assert_eq!(req.instructions.as_deref(), Some("sys"));
    }

    #[test]
    fn encodes_tool_use_and_result_as_function_items() {
        let input = ModelInput {
            system: String::new(),
            messages: vec![
                Message::Assistant {
                    content: vec![ContentBlock::ToolUse {
                        call_id: "call_1".into(),
                        tool_name: "read_file".into(),
                        args: serde_json::json!({"path": "/etc/hosts"}),
                    }],
                },
                Message::User {
                    content: vec![ContentBlock::ToolResult {
                        call_id: "call_1".into(),
                        result: ToolResult::text("file contents"),
                    }],
                },
            ],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        assert_eq!(req.input.len(), 2);
        match &req.input[0] {
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_1");
                assert_eq!(name, "read_file");
                assert!(arguments.contains("/etc/hosts"));
            }
            other => panic!("expected FunctionCall, got {other:?}"),
        }
        match &req.input[1] {
            InputItem::FunctionCallOutput { call_id, output } => {
                assert_eq!(call_id, "call_1");
                assert_eq!(output, "file contents");
            }
            other => panic!("expected FunctionCallOutput, got {other:?}"),
        }
    }

    #[test]
    fn encodes_mixed_assistant_message_in_correct_order() {
        // A single Assistant message carrying [Thinking, Text, ToolUse]
        // must flush as three flat items in the same order: Reasoning,
        // Message{assistant, OutputText}, FunctionCall. This guards the
        // multi-block flush logic in encode_assistant (text-run flush on
        // both Thinking and ToolUse boundaries) so adding new block
        // variants cannot silently reorder a re-feed.
        let input = ModelInput {
            system: String::new(),
            messages: vec![Message::Assistant {
                content: vec![
                    ContentBlock::Thinking {
                        text: "reasoning prelude".into(),
                        provider_opaque: None,
                    },
                    ContentBlock::Text {
                        text: "answer body".into(),
                    },
                    ContentBlock::ToolUse {
                        call_id: "call_42".into(),
                        tool_name: "read_file".into(),
                        args: serde_json::json!({"path": "/etc/hosts"}),
                    },
                ],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);

        assert_eq!(req.input.len(), 3, "expected exactly 3 items, got {req:?}");

        assert!(
            matches!(&req.input[0], InputItem::Reasoning { summary } if summary.len() == 1
                && summary[0].text == "reasoning prelude"),
            "first item must be Reasoning with the original text, got {:?}",
            req.input[0]
        );

        match &req.input[1] {
            InputItem::Message { role, content } => {
                assert_eq!(role, "assistant");
                assert_eq!(content.len(), 1);
                assert!(
                    matches!(
                        &content[0],
                        MessageContent::OutputText { text } if text == "answer body"
                    ),
                    "second item must carry OutputText 'answer body', got {:?}",
                    content[0]
                );
            }
            other => panic!("expected Message{{assistant}}, got {other:?}"),
        }

        match &req.input[2] {
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_42");
                assert_eq!(name, "read_file");
                assert!(arguments.contains("/etc/hosts"));
            }
            other => panic!("expected FunctionCall, got {other:?}"),
        }
    }

    #[test]
    fn encodes_thinking_block_as_reasoning_item() {
        let input = ModelInput {
            system: String::new(),
            messages: vec![Message::Assistant {
                content: vec![ContentBlock::Thinking {
                    text: "let me think...".into(),
                    provider_opaque: None,
                }],
            }],
            tools: vec![],
            params: empty_params(),
        };
        let cfg = OpenAiResponsesConfig::with_api_key("k");
        let req = encode_request(&input, &cfg);
        assert_eq!(req.input.len(), 1);
        match &req.input[0] {
            InputItem::Reasoning { summary } => {
                assert_eq!(summary.len(), 1);
                assert_eq!(summary[0].kind, "summary_text");
                assert_eq!(summary[0].text, "let me think...");
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }
}

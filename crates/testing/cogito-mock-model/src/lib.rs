//! `MockModelGateway` — a scripted `ModelGateway` for testing.
//!
//! Tests pre-load one or more `MockScript`s; each `stream()` call pops the
//! next script and emits its events (or returns its error).
//!
//! [`ScriptedMockModel`] is a parallel, deterministic variant intended for
//! chaos tests: it maps `ModelInput` characteristics to pre-recorded
//! `OutputScript`s without consuming them, so repeated calls with the same
//! input within a process yield byte-identical event streams.

#![warn(clippy::pedantic)]

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{
    Message, ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits,
};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use parking_lot::Mutex;

/// One scripted response.
#[derive(Debug, Clone)]
pub enum MockScript {
    /// Stream these events in order, then end the stream cleanly.
    Reply(Vec<ModelEvent>),
    /// Fail the `stream()` call up front with this error.
    Error(String),
}

/// Test gateway. Cheap to clone (`Arc` inside).
#[derive(Debug, Default, Clone)]
pub struct MockModelGateway {
    scripts: Arc<Mutex<VecDeque<MockScript>>>,
    /// Optional override for `model_limits().context_window_tokens`.
    ///
    /// When `None`, the default implementation returns `32_768`. Set via
    /// `with_context_window` to test adaptive-threshold logic.
    context_window_tokens: Option<u64>,
}

impl MockModelGateway {
    /// Construct empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the context-window size returned by `model_limits()`.
    ///
    /// Used to verify that `TruncateCompactor` `Ratio` thresholds scale
    /// correctly for different provider windows (e.g. 1 M, 200 k, 32 k).
    #[must_use]
    pub fn with_context_window(mut self, tokens: u64) -> Self {
        self.context_window_tokens = Some(tokens);
        self
    }

    /// Queue a successful reply.
    pub fn push_reply(&self, events: Vec<ModelEvent>) {
        self.scripts.lock().push_back(MockScript::Reply(events));
    }

    /// Queue an error at `stream()` time.
    pub fn push_error(&self, message: impl Into<String>) {
        self.scripts
            .lock()
            .push_back(MockScript::Error(message.into()));
    }

    /// Inspect how many scripts remain (for test assertions).
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.scripts.lock().len()
    }
}

#[async_trait]
impl ModelGateway for MockModelGateway {
    async fn stream(
        &self,
        _input: ModelInput,
        _ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        let script = self.scripts.lock().pop_front();
        match script {
            Some(MockScript::Reply(events)) => {
                let s = stream::iter(events.into_iter().map(Ok));
                Ok(s.boxed())
            }
            Some(MockScript::Error(msg)) => Err(ModelError::Provider {
                status: 500,
                message: msg,
            }),
            None => Err(ModelError::Provider {
                status: 0,
                message: "mock gateway: no scripts queued".into(),
            }),
        }
    }

    fn provider_id(&self) -> &'static str {
        "mock"
    }

    fn model_limits(&self) -> ModelLimits {
        let window = self.context_window_tokens.unwrap_or(32_768);
        ModelLimits::new("mock", window)
    }
}

/// Scripted deterministic mock for chaos tests. Given an [`InputMatcher`],
/// returns a pre-recorded [`OutputScript`] byte-for-byte identical across
/// repeated calls within a process — unlike [`MockModelGateway`] which
/// pops a FIFO queue. Used by chaos tests where the same logical model
/// call may fire twice (pre-crash + post-resume) and must yield the same
/// stream.
///
/// Matchers are evaluated in declaration order; the first match wins. If
/// none match, a defensive `MessageCompleted { stop_reason: EndTurn }` is
/// emitted so the FSM still reaches a terminal state.
#[derive(Default, Clone)]
pub struct ScriptedMockModel {
    matchers: Arc<Vec<(InputMatcher, OutputScript)>>,
}

/// Input-matching predicate for [`ScriptedMockModel`].
#[derive(Debug, Clone)]
pub enum InputMatcher {
    /// Match if any text in the last user message contains this substring.
    LastUserTextContains(String),
    /// Match if any text in the last `tool_result` block (searched across
    /// all messages, most recent first) contains this substring.
    LastToolResultContains(String),
    /// Always match (fallback).
    Any,
}

/// A scripted reply stream. Cloned into each `stream()` call so the
/// underlying events are never consumed.
#[derive(Debug, Clone)]
pub struct OutputScript {
    /// Events to emit, in order.
    pub events: Vec<ModelEvent>,
}

impl ScriptedMockModel {
    /// Construct from an ordered list of (matcher, script) pairs.
    #[must_use]
    pub fn new(matchers: Vec<(InputMatcher, OutputScript)>) -> Self {
        Self {
            matchers: Arc::new(matchers),
        }
    }
}

impl InputMatcher {
    /// Returns true iff this matcher applies to the given input.
    fn matches(&self, input: &ModelInput) -> bool {
        match self {
            InputMatcher::Any => true,
            InputMatcher::LastUserTextContains(needle) => {
                // Walk messages newest-first; the first User message we hit
                // is the "last" user message. Check its text blocks for the
                // substring.
                for msg in input.messages.iter().rev() {
                    if let Message::User { content } = msg {
                        for block in content {
                            if let ContentBlock::Text { text } = block {
                                if text.contains(needle.as_str()) {
                                    return true;
                                }
                            }
                        }
                        return false;
                    }
                }
                false
            }
            InputMatcher::LastToolResultContains(needle) => {
                // Walk messages newest-first; for each, walk content blocks
                // (also newest-first) and look at the first ToolResult we
                // encounter. ToolResult is a structured enum (Output / Error)
                // whose payload carries either JSON values or a message
                // string; the simplest tolerant predicate is to search the
                // block's Debug representation for the needle. This keeps
                // the matcher robust against ToolResult shape changes (e.g.
                // v0.2 multimodal upgrade) without taking a serde_json dep.
                for msg in input.messages.iter().rev() {
                    let content = match msg {
                        Message::User { content } | Message::Assistant { content } => content,
                    };
                    for block in content.iter().rev() {
                        if let ContentBlock::ToolResult { .. } = block {
                            let rendered = format!("{block:?}");
                            if rendered.contains(needle.as_str()) {
                                return true;
                            }
                            // Only the most-recent ToolResult is considered;
                            // stop after the first one we see.
                            return false;
                        }
                    }
                }
                false
            }
        }
    }
}

#[async_trait]
impl ModelGateway for ScriptedMockModel {
    async fn stream(
        &self,
        input: ModelInput,
        _ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        for (matcher, script) in self.matchers.iter() {
            if matcher.matches(&input) {
                let events: Vec<_> = script.events.iter().cloned().map(Ok).collect();
                return Ok(stream::iter(events).boxed());
            }
        }
        // Defensive fallback: emit a clean EndTurn so the FSM terminates.
        Ok(stream::iter(vec![Ok(ModelEvent::MessageCompleted {
            stop_reason: cogito_protocol::gateway::StopReason::EndTurn,
            usage: cogito_protocol::gateway::Usage::default(),
        })])
        .boxed())
    }

    fn provider_id(&self) -> &'static str {
        "scripted-mock"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use cogito_protocol::gateway::{Message, ModelEvent, ModelParams, StopReason, Usage};
    use cogito_protocol::ids::{SessionId, TurnId};

    fn dummy_exec_ctx() -> ExecCtx {
        ExecCtx::open_ended(SessionId::new(), TurnId::new())
    }

    fn make_input(text: &str) -> ModelInput {
        ModelInput {
            system: String::new(),
            messages: vec![Message::User {
                content: vec![ContentBlock::Text { text: text.into() }],
            }],
            tools: vec![],
            params: ModelParams {
                model: "scripted-mock".into(),
                max_tokens: 1,
                temperature: None,
                top_p: None,
                stop_sequences: vec![],
            },
        }
    }

    #[tokio::test]
    async fn scripted_mock_is_deterministic_across_calls() -> Result<(), Box<dyn std::error::Error>>
    {
        let script = OutputScript {
            events: vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "hi".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "hi".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                },
            ],
        };
        let mock = ScriptedMockModel::new(vec![(InputMatcher::Any, script)]);

        let input = make_input("hello");
        let ctx = dummy_exec_ctx();

        // Collect twice; assert byte-equality.
        let s1 = mock.stream(input.clone(), ctx.clone()).await?;
        let r1: Vec<_> = s1.collect().await;
        let s2 = mock.stream(input, ctx).await?;
        let r2: Vec<_> = s2.collect().await;

        // `Result<ModelEvent, ModelError>` doesn't derive PartialEq because
        // `ModelError` is non-PartialEq; compare via Debug format instead.
        assert_eq!(format!("{r1:?}"), format!("{r2:?}"));
        Ok(())
    }

    #[tokio::test]
    async fn scripted_mock_matches_by_last_user_text() -> Result<(), Box<dyn std::error::Error>> {
        // Two scripts; first matches "hello", second matches "world".
        let hello_script = OutputScript {
            events: vec![ModelEvent::TextBlockCompleted {
                block_index: 0,
                text: "matched-hello".into(),
            }],
        };
        let world_script = OutputScript {
            events: vec![ModelEvent::TextBlockCompleted {
                block_index: 0,
                text: "matched-world".into(),
            }],
        };
        let mock = ScriptedMockModel::new(vec![
            (
                InputMatcher::LastUserTextContains("hello".into()),
                hello_script,
            ),
            (
                InputMatcher::LastUserTextContains("world".into()),
                world_script,
            ),
        ]);

        let ctx = dummy_exec_ctx();

        let s1 = mock
            .stream(make_input("say hello please"), ctx.clone())
            .await?;
        let r1: Vec<_> = s1.collect().await;
        let s2 = mock.stream(make_input("hello world"), ctx.clone()).await?;
        let r2: Vec<_> = s2.collect().await;

        // The first call must hit matcher 0 ("hello"); the second call also
        // contains "hello" so it must hit matcher 0 (first-match-wins,
        // declaration order). Verify the second call's text is "matched-hello".
        assert!(format!("{r1:?}").contains("matched-hello"));
        assert!(format!("{r2:?}").contains("matched-hello"));

        // Now flip the order: a call that contains ONLY "world" should hit
        // matcher 1.
        let s3 = mock.stream(make_input("greetings world"), ctx).await?;
        let r3: Vec<_> = s3.collect().await;
        assert!(format!("{r3:?}").contains("matched-world"));
        Ok(())
    }

    #[tokio::test]
    async fn scripted_mock_fallback_terminates_when_no_match()
    -> Result<(), Box<dyn std::error::Error>> {
        // No matchers configured at all — fallback must emit MessageCompleted.
        let mock = ScriptedMockModel::new(vec![]);
        let s = mock
            .stream(make_input("anything"), dummy_exec_ctx())
            .await?;
        let r: Vec<_> = s.collect().await;
        let dbg = format!("{r:?}");
        assert!(dbg.contains("MessageCompleted"), "fallback debug: {dbg}");
        assert!(dbg.contains("EndTurn"), "fallback debug: {dbg}");
        Ok(())
    }

    #[tokio::test]
    async fn scripted_mock_matches_last_tool_result_substring()
    -> Result<(), Box<dyn std::error::Error>> {
        use cogito_protocol::tool::ToolResult;

        let script = OutputScript {
            events: vec![ModelEvent::TextBlockCompleted {
                block_index: 0,
                text: "matched-tool-result".into(),
            }],
        };
        let mock = ScriptedMockModel::new(vec![(
            InputMatcher::LastToolResultContains("needle-token".into()),
            script,
        )]);

        // Build a ModelInput whose last user message carries a ToolResult
        // containing the needle string somewhere in its serialized JSON.
        let input = ModelInput {
            system: String::new(),
            messages: vec![Message::User {
                content: vec![ContentBlock::ToolResult {
                    call_id: "toolu_01".into(),
                    result: ToolResult::text("payload contains needle-token here"),
                }],
            }],
            tools: vec![],
            params: ModelParams {
                model: "scripted-mock".into(),
                max_tokens: 1,
                temperature: None,
                top_p: None,
                stop_sequences: vec![],
            },
        };

        let s = mock.stream(input, dummy_exec_ctx()).await?;
        let r: Vec<_> = s.collect().await;
        assert!(format!("{r:?}").contains("matched-tool-result"));
        Ok(())
    }
}

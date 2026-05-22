//! H06 Stream Demultiplexer — consume a `ModelEvent` stream, drive the
//! `StepRecorder` text-block lifecycle, and accumulate a sealed
//! `ModelOutput` for H07.
//!
//! See `docs/components/H06-stream-demux.md`.

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelOutput, StopReason, Usage};
use cogito_protocol::ids::TurnId;
use futures::stream::{Stream, StreamExt};

use crate::harness::step_recorder::StepRecorder;

/// Consume the gateway stream to completion.
///
/// Side effects (via `recorder`):
/// - `TextDelta`: buffer chunk and broadcast as `StreamEvent::TextDelta`.
/// - `TextBlockCompleted`: flush buffer → one `AssistantMessageAppended` event.
/// - `ToolUseCompleted`: persist `ToolUseRecorded` + broadcast `ToolDispatchStarted`.
/// - `ToolUseStarted`: no action (H08 emits `ToolDispatchStarted/Ended`).
///
/// Returns the sealed `ModelOutput` with blocks in `block_index` order, or
/// the first `ModelError` encountered (caller transitions to `Failed`).
pub async fn demux<S>(
    mut stream: S,
    recorder: &mut StepRecorder,
    turn_id: TurnId,
) -> Result<ModelOutput, ModelError>
where
    S: Stream<Item = Result<ModelEvent, ModelError>> + Unpin,
{
    let mut content: Vec<(u32, ContentBlock)> = Vec::new();
    let mut stop_reason = StopReason::EndTurn;
    let mut usage = Usage::default();

    while let Some(evt) = stream.next().await {
        match evt? {
            ModelEvent::TextDelta {
                block_index: _,
                chunk,
            } => {
                // StepRecorder buffers + broadcasts TextDelta internally.
                recorder.on_text_delta(turn_id, chunk);
            }
            ModelEvent::TextBlockCompleted { block_index, text } => {
                recorder
                    .on_text_block_complete()
                    .await
                    .map_err(|e| ModelError::Provider {
                        status: 0,
                        message: format!("recorder flush: {e}"),
                    })?;
                content.push((block_index, ContentBlock::Text { text }));
            }
            ModelEvent::ThinkingDelta {
                block_index: _,
                chunk,
            } => {
                recorder.on_thinking_delta(turn_id, chunk);
            }
            ModelEvent::ThinkingBlockCompleted {
                block_index,
                text,
                provider_opaque,
            } => {
                recorder
                    .on_thinking_block_complete(provider_opaque.clone())
                    .await
                    .map_err(|e| ModelError::Provider {
                        status: 0,
                        message: format!("recorder thinking flush: {e}"),
                    })?;
                content.push((
                    block_index,
                    ContentBlock::Thinking {
                        text,
                        provider_opaque,
                    },
                ));
            }
            ModelEvent::ToolUseCompleted {
                block_index,
                call_id,
                tool_name,
                args,
            } => {
                recorder
                    .record_tool_use(turn_id, call_id.clone(), tool_name.clone(), args.clone())
                    .await
                    .map_err(|e| ModelError::Provider {
                        status: 0,
                        message: format!("recorder tool_use: {e}"),
                    })?;
                content.push((
                    block_index,
                    ContentBlock::ToolUse {
                        call_id,
                        tool_name,
                        args,
                    },
                ));
            }
            ModelEvent::MessageCompleted {
                stop_reason: sr,
                usage: u,
            } => {
                // Sprint 3: persist the sealing event before returning ModelOutput.
                // H03 relies on this to distinguish "model call done" from "in flight".
                recorder
                    .record_model_call_completed(turn_id, sr, u.clone())
                    .await
                    .map_err(|e| ModelError::Provider {
                        status: 0,
                        message: format!("recorder model_call_completed: {e}"),
                    })?;
                stop_reason = sr;
                usage = u;
            }
            // `ToolUseStarted` and any future variants (#[non_exhaustive]):
            // H06 takes no action — H08 emits ToolDispatchStarted/Ended
            // at actual dispatch time after the block is fully sealed.
            _ => {}
        }
    }

    content.sort_by_key(|(idx, _)| *idx);
    let ordered = content.into_iter().map(|(_, b)| b).collect();
    Ok(ModelOutput {
        content: ordered,
        stop_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
    use cogito_protocol::ids::{SessionId, TurnId};
    use cogito_protocol::store::ConversationStore;
    use cogito_store_jsonl::JsonlStore;
    use futures::stream;
    use tokio::sync::broadcast;

    use super::*;
    use crate::harness::step_recorder::StepRecorder;

    #[tokio::test]
    async fn demux_writes_model_call_completed_at_message_completed()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let (tx, _rx) = broadcast::channel(64);
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let mut recorder = StepRecorder::new(Arc::clone(&store), tx, session_id, 0);

        let events = stream::iter(vec![Ok(ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
            },
        })]);

        let output = demux(events, &mut recorder, turn_id).await?;
        assert_eq!(output.stop_reason, StopReason::EndTurn);

        // Verify ModelCallCompleted was persisted as the single event (seq 0).
        let last_seq = store.latest_seq(session_id).await?;
        assert!(
            last_seq.is_some(),
            "expected at least one event persisted after MessageCompleted"
        );

        // The store trait exposes replay(from_seq) returning seq > from_seq.
        // Since the only event is at seq 0, replay from 0 skips it.
        // Read the JSONL file directly to verify the payload type.
        let session_file = std::fs::read_dir(tmp.path())?
            .next()
            .ok_or("no session file found")?
            .map_err(|e| format!("dir entry error: {e}"))?
            .path();
        let text = tokio::fs::read_to_string(session_file).await?;
        assert!(
            text.contains("model_call_completed"),
            "expected model_call_completed payload in persisted log, got: {text}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn demux_routes_thinking_delta_and_completed() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
        let (tx, _rx) = broadcast::channel(64);
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let mut recorder = StepRecorder::new(Arc::clone(&store), tx, session_id, 0);
        recorder
            .record_session_started(cogito_protocol::session::SessionMeta {
                cogito_version: "0.1.0".into(),
                ..Default::default()
            })
            .await?;

        let events = stream::iter(vec![
            Ok(ModelEvent::ThinkingDelta {
                block_index: 0,
                chunk: "I should ".into(),
            }),
            Ok(ModelEvent::ThinkingDelta {
                block_index: 0,
                chunk: "grep.".into(),
            }),
            Ok(ModelEvent::ThinkingBlockCompleted {
                block_index: 0,
                text: "I should grep.".into(),
                provider_opaque: Some(serde_json::json!({"signature":"sig"})),
            }),
            Ok(ModelEvent::TextDelta {
                block_index: 1,
                chunk: "ok".into(),
            }),
            Ok(ModelEvent::TextBlockCompleted {
                block_index: 1,
                text: "ok".into(),
            }),
            Ok(ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            }),
        ]);

        let output = demux(events, &mut recorder, turn_id).await?;

        // ModelOutput.content carries the Thinking block at index 0, Text at 1.
        assert_eq!(output.content.len(), 2);
        #[allow(clippy::panic)]
        match &output.content[0] {
            cogito_protocol::content::ContentBlock::Thinking {
                text,
                provider_opaque,
            } => {
                assert_eq!(text, "I should grep.");
                assert_eq!(
                    provider_opaque.as_ref().and_then(|v| v.get("signature")),
                    Some(&serde_json::json!("sig"))
                );
            }
            other => panic!("expected Thinking at idx 0, got {other:?}"),
        }
        #[allow(clippy::panic)]
        match &output.content[1] {
            cogito_protocol::content::ContentBlock::Text { text } => assert_eq!(text, "ok"),
            other => panic!("expected Text at idx 1, got {other:?}"),
        }

        // Persisted: a thinking_block_recorded event preceded the assistant_message_appended.
        let session_file = std::fs::read_dir(tmp.path())?
            .next()
            .ok_or("no session file")?
            .map_err(|e| format!("{e}"))?
            .path();
        let log = tokio::fs::read_to_string(session_file).await?;
        let think_pos = log
            .find("thinking_block_recorded")
            .ok_or("thinking event missing")?;
        let text_pos = log
            .find("assistant_message_appended")
            .ok_or("text event missing")?;
        assert!(
            think_pos < text_pos,
            "thinking event must precede text event by seq"
        );
        Ok(())
    }
}

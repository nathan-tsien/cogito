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

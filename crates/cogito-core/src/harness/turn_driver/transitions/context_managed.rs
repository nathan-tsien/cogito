//! `ContextManaged → PromptBuilt` transition.
//!
//! Reads event history from the store, calls H04 Prompt Composer and H05
//! Tool Surface Builder, runs the `pre_prompt` hook, and records
//! `ContextManageCompleted` + `PromptComposed` before advancing.

use futures::StreamExt;

use cogito_protocol::ids::EventId;
use cogito_protocol::turn::TurnFailureReason;

use crate::harness::hooks::HookDecision;
use crate::harness::prompt::compose;
use crate::harness::tool_surface::surface;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `ContextManaged` to `PromptBuilt` (or `Failed`).
///
/// Event-write order (ADR-0003):
/// 1. `ContextManageCompleted` — marks the context-manage pass-through done.
/// 2. `PromptComposed`          — records the prompt metadata.
///
/// History is read via `ConversationStore::replay(session_id, 0)` which
/// streams all events. An empty history is valid (fresh session).
pub async fn transit(ctx: TurnCtx, deps: &TurnDeps) -> TurnState {
    // --- Record ContextManageCompleted (write before transition, ADR-0003) ---
    let _ = deps
        .step
        .lock()
        .await
        .record_context_manage_completed(ctx.turn_id)
        .await;

    // --- Read history for H04 ---
    // `replay(session_id, 0)` streams events with seq > 0, so the first event
    // (seq 0, typically SessionStarted / TurnStarted) is skipped. For Sprint 2
    // this is acceptable; Sprint 3 will pass `from_seq = u64::MAX` on fresh
    // sessions and use the full log on resumes.
    //
    // If the store fails to replay we fall back to an empty history so the
    // FSM can still make progress with an empty conversation context.
    let mut history_stream = deps.store.replay(ctx.session_id, 0);
    let mut history = Vec::new();
    while let Some(result) = history_stream.next().await {
        match result {
            Ok(event) => history.push(event),
            Err(_) => break,
        }
    }

    // --- H05 Tool Surface Builder ---
    let tool_surface = surface(&ctx.strategy, deps.tools.as_ref());

    // --- H04 Prompt Composer ---
    let model_input = compose(&history, &ctx.strategy, &tool_surface);

    // --- Record PromptComposed ---
    let surface_size = u32::try_from(tool_surface.len()).unwrap_or(u32::MAX);
    let _ = deps
        .step
        .lock()
        .await
        .record_prompt_composed(
            ctx.turn_id,
            ctx.strategy.model_params.model.clone(),
            surface_size,
        )
        .await;

    // --- H09 pre_prompt hook ---
    match deps.hooks.pre_prompt(&model_input) {
        HookDecision::Reject { hook_name, reason } => {
            // Persist the HookRejected event (additive log entry, ADR-0007)
            // before the TurnFailed event so the log ordering reflects causality.
            let _ = deps
                .step
                .lock()
                .await
                .record_hook_rejected(
                    ctx.turn_id,
                    hook_name.clone(),
                    cogito_protocol::hook::HookLifecyclePoint::PrePrompt,
                    reason.clone(),
                )
                .await;
            let failure_reason = TurnFailureReason::HookRejected {
                hook_name,
                message: reason,
            };
            // Capture reason string before moving `failure_reason` into `record_turn_failed`.
            let reason_str = format!("{failure_reason:?}");
            let recorded_event_id = match deps
                .step
                .lock()
                .await
                .record_turn_failed(ctx.turn_id, failure_reason.clone())
                .await
            {
                Ok(id) => id,
                // Recorder failed while recording the failure itself.
                Err(_) => EventId::recorder_failure_placeholder(),
            };
            deps.hooks.on_error(&reason_str);
            TurnState::Failed {
                reason: failure_reason,
                recorded_event_id,
            }
        }
        // `HookDecision` is `#[non_exhaustive]`; Allow and unknown variants continue.
        _ => TurnState::PromptBuilt {
            ctx,
            input: model_input,
            surface: tool_surface,
        },
    }
}

//! `ContextManaged -> PromptBuilt` transition.
//!
//! Sprint 6 implements the real H11 orchestration per ADR-0008:
//! 1. Compactor  — may compact history; failures degrade (recorded in errors).
//! 2. `SystemPromptInjector` — always writes a `SystemPromptInjected` event.
//! 3. `ToolFilterOverrider`  — always writes a `ToolFilterOverridden` event.
//! 4. `ContextDecisionRecorded` summary event.
//! 5. `ContextManageCompleted` — marks the H11 pass done.
//!
//! Failure semantics (ADR-0008): any trait failure is captured in
//! `ContextDecisionErrors` and the turn continues. H11 never propagates a
//! `ContextError` to H01 as a fatal `TurnFailureReason`.

use futures::StreamExt;
use tracing::warn;

use cogito_protocol::context::{
    CompactionInput, ContextDecisionErrors, InjectionInput, ToolFilterInput, ToolFilterOverrideMode,
};
use cogito_protocol::ids::EventId;
use cogito_protocol::turn::TurnFailureReason;

use crate::harness::hooks::HookDecision;
use crate::harness::prompt::compose;
use crate::harness::tool_surface::surface;
use crate::harness::turn_driver::deps::TurnDeps;
use crate::harness::turn_driver::state::{TurnCtx, TurnState};

/// Transition from `ContextManaged` to `PromptBuilt` (or `Failed`).
///
/// Event-write order (ADR-0003 / ADR-0008):
/// 1. `SystemPromptInjected`    — written by injector (or fallback empty).
/// 2. `ToolFilterOverridden`    — written by overrider (or fallback Inherit).
/// 3. `ContextDecisionRecorded` — summary cross-referencing 1 and 2.
/// 4. `ContextManageCompleted`  — marks H11 done, H01 advances to `PromptBuilt`.
/// 5. `PromptComposed`          — H04/H05 output.
///
/// Compaction events (`ContextCompacted`) are written by the `Compactor`
/// itself, before the injector step, and their event ids are collected
/// into `ContextDecisionRecorded.compactions`.
#[allow(clippy::too_many_lines)]
pub async fn transit(ctx: TurnCtx, deps: &TurnDeps) -> TurnState {
    let pipeline = &deps.context_pipeline;
    let mut errors = ContextDecisionErrors::default();

    // Load history once. On store failure, fall back to empty history so H11
    // can still make progress (the same pattern used by the old pass-through).
    let history = {
        let mut stream = deps.store.replay(ctx.session_id, 0);
        let mut events = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(ev) => events.push(ev),
                Err(_) => break,
            }
        }
        events
    };

    // --- Step 1: Compactor ---
    let compactions: Vec<EventId> = {
        let mut recorder = deps.step.lock().await;
        let input = CompactionInput {
            session_id: ctx.session_id,
            turn_id: ctx.turn_id,
            history: &history,
            strategy: &ctx.strategy,
            // `last_usage` is not yet threaded through TurnCtx (Task 31 will
            // add it from the prior ModelCallCompleted event in history).
            last_usage: None,
            model_gateway: deps.model.as_ref(),
            recorder: &mut *recorder,
        };
        match pipeline.compactor.maybe_compact(input).await {
            Ok(applied) => applied.into_iter().map(|c| c.event_id).collect(),
            Err(e) => {
                warn!(error = %e, "H11 compactor degraded; recording error and continuing");
                errors.compactor = Some(e.to_string());
                vec![]
            }
        }
    };

    // --- Step 2: SystemPromptInjector ---
    let system_prompt_event: EventId = {
        let mut recorder = deps.step.lock().await;
        let input = InjectionInput {
            session_id: ctx.session_id,
            turn_id: ctx.turn_id,
            strategy: &ctx.strategy,
            history: &history,
            exec_ctx: &ctx.exec_ctx,
            recorder: &mut *recorder,
        };
        match pipeline.injector.inject(input).await {
            Ok(eid) => eid,
            Err(e) => {
                warn!(error = %e, "H11 injector degraded; writing fallback empty SystemPromptInjected");
                errors.injector = Some(e.to_string());
                // Fallback: write an empty suffix so the event sequence is always complete.
                match recorder
                    .record_system_prompt_injected(
                        ctx.turn_id,
                        String::new(),
                        vec![],
                        "fallback-empty",
                    )
                    .await
                {
                    Ok(eid) => eid,
                    Err(store_err) => {
                        // Recorder itself failed — use a sentinel and log loudly.
                        warn!(
                            error = %store_err,
                            "H11 fallback SystemPromptInjected write failed; using placeholder EventId"
                        );
                        EventId::recorder_failure_placeholder()
                    }
                }
            }
        }
    };

    // --- Step 3: ToolFilterOverrider ---
    let tool_filter_event: EventId = {
        let mut recorder = deps.step.lock().await;
        let input = ToolFilterInput {
            session_id: ctx.session_id,
            turn_id: ctx.turn_id,
            strategy: &ctx.strategy,
            history: &history,
            exec_ctx: &ctx.exec_ctx,
            recorder: &mut *recorder,
        };
        match pipeline.overrider.override_filter(input).await {
            Ok(eid) => eid,
            Err(e) => {
                warn!(error = %e, "H11 overrider degraded; writing fallback Inherit ToolFilterOverridden");
                errors.overrider = Some(e.to_string());
                // Fallback: write an Inherit override so the event sequence is always complete.
                match recorder
                    .record_tool_filter_overridden(
                        ctx.turn_id,
                        ToolFilterOverrideMode::Inherit,
                        vec![],
                        "fallback-inherit",
                    )
                    .await
                {
                    Ok(eid) => eid,
                    Err(store_err) => {
                        warn!(
                            error = %store_err,
                            "H11 fallback ToolFilterOverridden write failed; using placeholder EventId"
                        );
                        EventId::recorder_failure_placeholder()
                    }
                }
            }
        }
    };

    // --- Step 4: ContextDecisionRecorded summary ---
    {
        let mut recorder = deps.step.lock().await;
        let _ = recorder
            .record_context_decision(
                ctx.turn_id,
                compactions,
                system_prompt_event,
                tool_filter_event,
                errors,
            )
            .await;
        // Recorder failure here is non-fatal — the summary is observability-only;
        // its absence does not break the FSM. The missing event will be visible
        // in the log gap.
    }

    // --- Step 5: ContextManageCompleted (marks H11 done) ---
    let _ = deps
        .step
        .lock()
        .await
        .record_context_manage_completed(ctx.turn_id)
        .await;

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
            let reason_str = format!("{failure_reason:?}");
            let recorded_event_id = match deps
                .step
                .lock()
                .await
                .record_turn_failed(ctx.turn_id, failure_reason.clone())
                .await
            {
                Ok(id) => id,
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

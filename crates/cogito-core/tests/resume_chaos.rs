//! Sprint 3 P5.6 — chaos test main.
//!
//! Verifies cogito's resume guarantees across crash boundaries. For each
//! scenario, runs a golden (uncrashed) baseline, then crashes at every
//! resumable event boundary, resumes in a fresh Runtime sharing the same
//! backing store, and asserts 4 oracles:
//!
//! 1. `prefix_immutable` — events `[0..n]` identical across golden and resumed
//! 2. `terminal_equivalent` — same terminal kind (Completed/Failed/Paused)
//! 3. `tool_mapping_equivalent` — `(call_id -> (tool_name, args, result))` map
//!    is identical
//! 4. `final_text_equivalent` — concatenated `AssistantMessageAppended` text
//!    is identical
//!
//! v0.1 narrowing:
//!
//! - X path (poisoned-actor detection from outside the actor) is deferred —
//!   the test stays on the spec's "Y path" lane: crash, observe via the
//!   fault wrapper, fresh Runtime, Resume.
//! - Of the 4 catalog scenarios, only `single_tool_happy_path` and
//!   `no_tool_short_turn` are exercised. `paused_async_job` is unrunnable
//!   (Runtime has no `JobManager` injection in v0.1) and
//!   `tool_returns_error` is deferred.
//! - Crash boundaries are filtered to those that resolve to
//!   `ResumeFromModelCompleted` in `harness/resume.rs`. The
//!   `ResumeFromToolDispatching` path has a pre-existing ordering bug
//!   (looks for `ToolUseRecorded` after `ModelCallCompleted`, but H06
//!   writes them before), and `RestartCurrentTurn` is downgraded to
//!   `FreshTurn` in `apply_resume_point`. Both are explicit `TODO(post-Sprint-3)`
//!   in the codebase; they are out of scope for P5.6.
//!
//! Comparison note: `ConversationStore::replay(id, 0)` yields events with
//! `seq > 0` strictly, so the seq=0 `SessionStarted` event is NOT included in
//! the oracles' inputs. That is acceptable: `SessionStarted` is identical
//! across runs by construction and the chaos invariant is about turn-level
//! events, not session lifecycle.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::panic,
    clippy::print_stderr
)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cogito_core::runtime::{OpenMode, Runtime, SessionHandle, SessionSpec, ShutdownOutcome};
use cogito_mock_model::{InputMatcher, OutputScript, ScriptedMockModel};
use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobId, JobManager, JobOutcome};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolProvider, ToolResult,
};
use cogito_protocol::{ConversationEvent, EventPayload};
use cogito_store::JsonlStore;
use cogito_test_fixtures::chaos_scenarios::{self, ChaosScenario};
use cogito_test_fixtures::fault_store::{FaultInjectingStore, FaultTrigger};
use cogito_test_fixtures::mock_job_manager::MockJobManager;
use futures::TryStreamExt as _;

/// In-test `ToolProvider` that returns `MOCK_TOOL_RESULT` for any invocation.
///
/// Used by the chaos test so the second model call in
/// `single_tool_happy_path` can be dispatched via
/// `InputMatcher::LastToolResultContains("MOCK_TOOL_RESULT")`. The descriptor
/// list is intentionally minimal — the model scripts hard-code the tool name
/// (`read_file`), so H07 only needs that one descriptor in the surface.
struct MockToolProvider;

#[async_trait]
impl ToolProvider for MockToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: "read_file".into(),
            description: "test stub for chaos test".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, _name: &str, _args: serde_json::Value, _ctx: ExecCtx) -> InvokeOutcome {
        InvokeOutcome::Sync(ToolResult::text("MOCK_TOOL_RESULT"))
    }
}

/// Build a deterministic `ScriptedMockModel` from a scenario's scripts.
///
/// - 1 script: a single `InputMatcher::Any` matcher.
/// - 2 scripts: first matcher dispatches on
///   `LastToolResultContains("MOCK_TOOL_RESULT")` (the post-tool call), and
///   `InputMatcher::Any` falls back to the initial call.
///
/// First-match-wins ordering matters: the tool-result matcher must come
/// before the `Any` fallback.
fn build_scripted_mock(scenario: &ChaosScenario) -> ScriptedMockModel {
    let mut matchers = Vec::new();
    if scenario.model_scripts.len() == 1 {
        matchers.push((
            InputMatcher::Any,
            OutputScript {
                events: scenario.model_scripts[0].clone(),
            },
        ));
    } else {
        matchers.push((
            InputMatcher::LastToolResultContains("MOCK_TOOL_RESULT".into()),
            OutputScript {
                events: scenario.model_scripts[1].clone(),
            },
        ));
        matchers.push((
            InputMatcher::Any,
            OutputScript {
                events: scenario.model_scripts[0].clone(),
            },
        ));
    }
    ScriptedMockModel::new(matchers)
}

/// Wire a `Runtime` with the chaos test's deterministic mock model + tools.
fn build_runtime(store: Arc<dyn ConversationStore>, scenario: &ChaosScenario) -> Arc<Runtime> {
    let mock = Arc::new(build_scripted_mock(scenario));
    let tools = Arc::new(MockToolProvider);
    Runtime::builder()
        .store(store)
        .model(mock as Arc<dyn ModelGateway>)
        .tools(tools as Arc<dyn ToolProvider>)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()
        .expect("runtime builds")
}

/// Extract the first text block from a scenario's user input.
fn extract_user_text(scenario: &ChaosScenario) -> String {
    match &scenario.user_input[0] {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("scenario user_input[0] is not Text: {other:?}"),
    }
}

/// Wait (with a 5s ceiling) for any terminal `StreamEvent` to appear on the
/// session's broadcast. If the store already contains a terminal event the
/// turn has nothing left to drive and the broadcast will never fire, so we
/// short-circuit by polling `latest_seq + replay` first. Receiver errors
/// (`Lagged`/`Closed`) are also treated as terminal so the test does not
/// hang.
async fn wait_for_terminal_with_store(
    handle: &SessionHandle,
    store: &dyn ConversationStore,
    session_id: SessionId,
) {
    // If the log already has a terminal, the resumed actor idles silently
    // (FreshTurn path on a completed session). No broadcast will arrive.
    if log_has_terminal(store, session_id).await {
        return;
    }
    let mut events_rx = handle.subscribe();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Ok(
                    StreamEvent::TurnCompleted { .. }
                    | StreamEvent::TurnFailed { .. }
                    | StreamEvent::TurnCancelled { .. }
                    | StreamEvent::TurnPaused { .. },
                )
                | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
}

/// Wait variant for the golden run (no resume) — always subscribes.
async fn wait_for_terminal_broadcast(handle: &SessionHandle) {
    let mut events_rx = handle.subscribe();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Ok(
                    StreamEvent::TurnCompleted { .. }
                    | StreamEvent::TurnFailed { .. }
                    | StreamEvent::TurnCancelled { .. }
                    | StreamEvent::TurnPaused { .. },
                )
                | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
}

/// Whether the store already contains a terminal event for the session.
async fn log_has_terminal(store: &dyn ConversationStore, session_id: SessionId) -> bool {
    let events = read_log(store, session_id).await;
    events.iter().any(|e| {
        matches!(
            e.payload,
            EventPayload::TurnCompleted { .. }
                | EventPayload::TurnFailed { .. }
                | EventPayload::TurnPaused { .. }
        )
    })
}

/// Read the full event log (seq > 0) from the store.
async fn read_log(store: &dyn ConversationStore, session_id: SessionId) -> Vec<ConversationEvent> {
    store
        .replay(session_id, 0)
        .try_collect()
        .await
        .expect("replay")
}

/// Captured result of a golden (no-fault) run.
#[derive(Debug)]
struct GoldenRun {
    /// All events with `seq > 0` for the session, in order.
    events: Vec<ConversationEvent>,
    /// The final terminal payload (`TurnCompleted` / `TurnFailed` / `TurnPaused`).
    terminal: EventPayload,
}

/// Run the scenario to natural completion without injecting any faults.
async fn run_to_completion_without_faults(scenario: &ChaosScenario) -> GoldenRun {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_runtime(Arc::clone(&store), scenario);

    let session_id = SessionId::new();
    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");

    // Subscribe before send to avoid missing the terminal broadcast.
    let _events_rx = handle.subscribe();

    handle
        .submit_user_text(extract_user_text(scenario))
        .await
        .expect("submit_user_text");
    wait_for_terminal_broadcast(&handle).await;

    let events = read_log(store.as_ref(), session_id).await;
    let terminal = terminal_payload(&events).clone();

    let out = handle
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got {out:?}"
    );

    drop(tmp);
    GoldenRun { events, terminal }
}

/// Run the scenario with a `PanicAt(crash_after_n)` Y-path crash, then
/// resume in a fresh `Runtime` and let the turn drive to terminal.
///
/// Mechanism: `FaultInjectingStore` panics inside `append` immediately after
/// the N-th successful write. The panic propagates up to the spawned actor
/// task, which dies — tokio cleanly aborts the task and does not unwind the
/// rest of the runtime. No terminal event is written, leaving an in-flight
/// turn on disk for phase 2 to resume.
///
/// Why not `NotifyAt`? The "clean shutdown" path would issue
/// `SessionHandle::shutdown`, but shutdown cancels the in-flight turn which
/// writes `TurnFailed { TurnTimedOut }` — a terminal event that defeats the
/// chaos invariant. `PanicAt` is the only `FaultTrigger` variant that stops
/// the actor without writing a terminal. (The spec's "Y path = `NotifyAt`"
/// label predates this observation; we adopt `PanicAt` while keeping the
/// "Y path only — X path deferred" narrowing since we still don't probe
/// poisoned-actor state from outside.)
async fn run_with_y_fault(scenario: &ChaosScenario, crash_after_n: u64) -> Vec<ConversationEvent> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::new();

    // ----- Phase 1: run until PanicAt fires inside the actor task. -----
    //
    // `open_session` writes seq=0 SessionStarted on the *caller* thread (not
    // the actor task), so setting the trigger before open_session would
    // panic the test process for low crash_after_n values. Arm the trigger
    // AFTER open_session so the panic only ever lands inside the actor
    // task once `submit_user_text` kicks off turn-event appends.
    let inner1: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let wrapper1: Arc<FaultInjectingStore<JsonlStore>> =
        Arc::new(FaultInjectingStore::new(Arc::clone(&inner1)));

    let store1: Arc<dyn ConversationStore> = Arc::clone(&wrapper1) as Arc<dyn ConversationStore>;
    let runtime1 = build_runtime(store1, scenario);
    let handle1 = runtime1
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");

    // `wrapper1.written_count()` is now 1 (SessionStarted). `crash_after_n`
    // is expressed in turn-event-index (1 = first turn event = seq 1), so
    // the corresponding append count is `crash_after_n + 1`.
    wrapper1
        .set_trigger(FaultTrigger::PanicAt {
            event_no: crash_after_n + 1,
            message: "chaos test fault",
        })
        .await;

    handle1
        .submit_user_text(extract_user_text(scenario))
        .await
        .expect("submit_user_text");

    // Spin until either the on-disk log has `crash_after_n` events (panic
    // landed) or the turn reaches a terminal (turn finished before the
    // boundary — degenerate case, resumed log == golden). Bounded by a
    // generous deadline so a stuck actor cannot hang the test.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = read_log(inner1.as_ref(), session_id).await;
        let has_terminal = events.iter().any(|e| {
            matches!(
                e.payload,
                EventPayload::TurnCompleted { .. }
                    | EventPayload::TurnFailed { .. }
                    | EventPayload::TurnPaused { .. }
            )
        });
        // Panic hits after `crash_after_n + 1` appends (the +1 accounts for
        // SessionStarted having already been written).
        if has_terminal || wrapper1.written_count() > crash_after_n {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Drop the runtime side. The panicked actor task is already dead; this
    // releases the channels so no late writes can interleave with phase 2.
    drop(handle1);
    drop(runtime1);

    // Give tokio a moment to fully tear down the actor task and flush any
    // pending writes before phase 2 reads the file.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ----- Phase 2: fresh Runtime, same on-disk JSONL, Resume mode. -----
    let inner2: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let store2: Arc<dyn ConversationStore> = inner2.clone();
    let runtime2 = build_runtime(Arc::clone(&store2), scenario);

    let handle2 = runtime2
        .open_session(session_id, OpenMode::Resume)
        .await
        .expect("open Resume");

    wait_for_terminal_with_store(&handle2, store2.as_ref(), session_id).await;

    let events = read_log(store2.as_ref(), session_id).await;
    let _ = handle2.shutdown(Duration::from_secs(5)).await;

    drop(tmp);
    events
}

// === Oracles =================================================================

/// Per-event canonical view used by `assert_prefix_immutable`. Skips fields
/// that legitimately vary across runs:
///
/// - `event_id` and `ts` are generated per-write.
/// - `turn_id` on the envelope is a fresh ULID minted by each `TurnDriver`
///   instance; the golden run and the resumed run drive different instances
///   even though they describe the same logical turn.
/// - Sprint 6 H11 events (`SystemPromptInjected`, `ToolFilterOverridden`,
///   `ContextDecisionRecorded`) embed `turn_id` and `EventId` values inside
///   the payload. These are also run-specific, so they are normalized to
///   sentinel/nil values before comparison.
#[derive(Debug, PartialEq)]
struct Canonical {
    seq: u64,
    payload: EventPayload,
}

/// Nil sentinel values used to normalize run-specific IDs in H11 payloads.
fn nil_turn_id() -> cogito_protocol::ids::TurnId {
    cogito_protocol::ids::TurnId::from(ulid::Ulid::nil())
}

fn nil_event_id() -> cogito_protocol::ids::EventId {
    cogito_protocol::ids::EventId::recorder_failure_placeholder()
}

/// Nil sentinel `JobId` used to normalize run-specific job identifiers in
/// async-job payloads. Each phase of the chaos test instantiates its own
/// `JobManager` (and the async-tool fixture mints a fresh `JobId` on
/// every invoke), so the persisted `JobSubmitted` / `TurnPaused` /
/// `JobCompletedRecorded` events from golden vs resumed runs carry
/// non-equal ULIDs even though they describe the same logical job in the
/// same logical position. Normalizing to a fixed sentinel makes
/// prefix-immutability comparable; the per-call_id pairing remains
/// asserted via `assert_tool_mapping_equivalent`.
fn nil_job_id() -> cogito_protocol::job::JobId {
    serde_json::from_value::<cogito_protocol::job::JobId>(serde_json::Value::String(
        ulid::Ulid::nil().to_string(),
    ))
    .expect("nil ULID parses as JobId")
}

/// Nil sentinel `MessageId` used to normalize the per-assistant-message id
/// (ADR-0041). It is minted fresh per model call (`MessageId::new()`), so
/// golden and resumed runs carry non-equal ULIDs at the same logical
/// position — exactly like `EventId` / `JobId`. Normalizing to a fixed
/// sentinel keeps prefix-immutability comparable; the per-message grouping
/// is still structurally asserted (same id shared by a message's events).
fn nil_message_id() -> cogito_protocol::ids::MessageId {
    cogito_protocol::ids::MessageId::from(ulid::Ulid::nil())
}

/// Return a canonicalized payload that replaces run-specific ID fields with
/// stable sentinel values so golden and resumed logs compare equal.
fn normalize_payload(payload: EventPayload) -> EventPayload {
    match payload {
        EventPayload::SystemPromptInjected {
            suffix,
            contributors,
            produced_by,
            ..
        } => EventPayload::SystemPromptInjected {
            turn_id: nil_turn_id(),
            suffix,
            contributors,
            produced_by,
        },
        EventPayload::ToolFilterOverridden {
            mode,
            contributors,
            produced_by,
            ..
        } => EventPayload::ToolFilterOverridden {
            turn_id: nil_turn_id(),
            mode,
            contributors,
            produced_by,
        },
        EventPayload::ContextDecisionRecorded {
            compactions,
            errors,
            ..
        } => EventPayload::ContextDecisionRecorded {
            turn_id: nil_turn_id(),
            // EventId cross-references differ between runs — normalize to nil.
            compactions: compactions.into_iter().map(|_| nil_event_id()).collect(),
            system_prompt_event: nil_event_id(),
            tool_filter_event: nil_event_id(),
            errors,
        },
        // Sprint 8: async-job payloads embed a `JobId` minted per phase
        // (each `JobManager` instance mints its own ULIDs). Normalize so
        // golden and resumed logs compare equal up to logical structure.
        EventPayload::JobSubmitted {
            call_id, tool_name, ..
        } => EventPayload::JobSubmitted {
            call_id,
            job_id: nil_job_id(),
            tool_name,
        },
        EventPayload::TurnPaused { .. } => EventPayload::TurnPaused {
            job_id: nil_job_id(),
        },
        EventPayload::JobCompletedRecorded { outcome, .. } => EventPayload::JobCompletedRecorded {
            job_id: nil_job_id(),
            outcome,
        },
        // ADR-0041: the per-message `MessageId` is minted fresh per model
        // call, so normalize it to a sentinel (like `EventId` / `JobId`)
        // for prefix-immutability comparison.
        EventPayload::AssistantMessageAppended { text, .. } => {
            EventPayload::AssistantMessageAppended {
                text,
                message_id: Some(nil_message_id()),
            }
        }
        EventPayload::ToolUseRecorded {
            call_id,
            tool_name,
            args,
            ..
        } => EventPayload::ToolUseRecorded {
            call_id,
            tool_name,
            args,
            message_id: Some(nil_message_id()),
        },
        EventPayload::ThinkingBlockRecorded {
            text,
            provider_opaque,
            ..
        } => EventPayload::ThinkingBlockRecorded {
            text,
            provider_opaque,
            message_id: Some(nil_message_id()),
        },
        // All other variants carry no run-specific IDs inside the payload.
        other => other,
    }
}

fn canonical(e: &ConversationEvent) -> Canonical {
    Canonical {
        seq: e.seq,
        payload: normalize_payload(e.payload.clone()),
    }
}

/// Oracle 1 — the first `n` events of the resumed log must exactly equal
/// the first `n` events of the golden log.
fn assert_prefix_immutable(golden: &[ConversationEvent], resumed: &[ConversationEvent], n: u64) {
    let prefix_len = usize::try_from(n).expect("crash boundary fits in usize");
    assert!(
        resumed.len() >= prefix_len,
        "resumed log shorter than crash point: resumed_len={} crash_n={n}",
        resumed.len()
    );
    assert!(
        golden.len() >= prefix_len,
        "golden log shorter than crash point: golden_len={} crash_n={n}",
        golden.len()
    );
    let golden_prefix: Vec<_> = golden[..prefix_len].iter().map(canonical).collect();
    let resumed_prefix: Vec<_> = resumed[..prefix_len].iter().map(canonical).collect();
    assert_eq!(
        golden_prefix, resumed_prefix,
        "prefix immutability violated at n={n}"
    );
}

/// Locate the terminal event in a log. Panics if none is present — every
/// well-formed turn must end in `TurnCompleted`/`TurnFailed`/`TurnPaused`.
fn terminal_payload(events: &[ConversationEvent]) -> &EventPayload {
    events
        .iter()
        .rev()
        .find_map(|e| match &e.payload {
            EventPayload::TurnCompleted { .. }
            | EventPayload::TurnFailed { .. }
            | EventPayload::TurnPaused { .. } => Some(&e.payload),
            _ => None,
        })
        .expect("no terminal event in log")
}

/// Oracle 2 — golden and resumed must reach the same terminal kind.
fn assert_terminal_equivalent(g: &EventPayload, r: &EventPayload) {
    use EventPayload::{TurnCompleted, TurnFailed, TurnPaused};
    match (g, r) {
        (TurnCompleted { .. }, TurnCompleted { .. }) => {}
        (TurnFailed { reason: r1 }, TurnFailed { reason: r2 }) => {
            assert_eq!(
                std::mem::discriminant(r1),
                std::mem::discriminant(r2),
                "TurnFailed reasons differ: golden={r1:?} resumed={r2:?}"
            );
        }
        (TurnPaused { job_id: j1 }, TurnPaused { job_id: j2 }) => {
            assert_eq!(j1, j2, "TurnPaused job_id differs");
        }
        _ => panic!("terminal kind differs: golden={g:?} resumed={r:?}"),
    }
}

/// Build a `call_id -> (tool_name, args, result)` mapping from an event log.
fn collect_tool_mapping(
    events: &[ConversationEvent],
) -> std::collections::BTreeMap<String, (String, serde_json::Value, ToolResult)> {
    use std::collections::{BTreeMap, HashMap};
    let mut uses: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut results: HashMap<String, ToolResult> = HashMap::new();
    for e in events {
        match &e.payload {
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
                ..
            } => {
                uses.insert(call_id.clone(), (tool_name.clone(), args.clone()));
            }
            EventPayload::ToolResultRecorded { call_id, result } => {
                results.insert(call_id.clone(), result.clone());
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for (id, (name, args)) in uses {
        if let Some(r) = results.get(&id) {
            out.insert(id, (name, args, r.clone()));
        }
    }
    out
}

/// Oracle 3 — `(call_id, tool_name, args, result)` tuples must be identical.
fn assert_tool_mapping_equivalent(g: &[ConversationEvent], r: &[ConversationEvent]) {
    let gm = collect_tool_mapping(g);
    let rm = collect_tool_mapping(r);
    // `ToolResult` is `PartialEq` but the tuple's `serde_json::Value` is too,
    // so `BTreeMap` should compare cleanly. Use Debug-fmt as a defensive
    // representation so failure messages show the full mismatched mapping.
    assert_eq!(format!("{gm:?}"), format!("{rm:?}"), "tool mappings differ");
}

/// Concatenate all `AssistantMessageAppended` text in order.
fn collect_assistant_text(events: &[ConversationEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::AssistantMessageAppended { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Oracle 4 — concatenated assistant text must match across runs.
fn assert_final_text_equivalent(g: &[ConversationEvent], r: &[ConversationEvent]) {
    assert_eq!(
        collect_assistant_text(g),
        collect_assistant_text(r),
        "final assistant text differs"
    );
}

/// v0.1 narrowing: only resume points wired in `apply_resume_point` produce
/// a testable resume. `RestartCurrentTurn` is downgraded to `FreshTurn`
/// (TODO in actor.rs) so any crash boundary that lands on `TurnStarted` or
/// inside a model call (before `ModelCallCompleted`) cannot be exercised.
///
/// Additionally, `ResumeFromToolDispatching` in `harness/resume.rs` looks for
/// `ToolUseRecorded` events AFTER `ModelCallCompleted`, but H06 actually
/// writes them BEFORE — the resume path then sees an empty pending list
/// against a `stop_reason=ToolUse` `ModelCallCompleted` and reports
/// `ResumeError::Malformed`. That is a pre-existing resume-logic bug
/// (out of scope for P5.6) — we work around it here by selecting only
/// boundaries whose latest `ModelCallCompleted` has a non-`ToolUse`
/// stop reason, i.e. boundaries that resolve to `ResumeFromModelCompleted`.
///
/// Returns 1-indexed turn-event-indices (matching `crash_after_n`).
fn resumable_boundaries(events: &[ConversationEvent]) -> Vec<u64> {
    use cogito_protocol::gateway::StopReason;
    events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if let EventPayload::ModelCallCompleted { stop_reason, .. } = &e.payload {
                if !matches!(stop_reason, StopReason::ToolUse) {
                    return Some((i + 1) as u64);
                }
            }
            None
        })
        .collect()
}

// === Context-manage pairing oracle ==========================================

/// Assert that every `ContextManageEntered` in the log has a matching
/// `ContextManageCompleted` (or that the turn ended with `TurnFailed`, which
/// resets any pending enter). Both variants are empty structs — they carry no
/// `turn_id` — so pairing is tracked by sequential count rather than by ID.
///
/// A well-formed log satisfies:
/// - `count(ContextManageCompleted) == count(ContextManageEntered)`
/// - No `ContextManageCompleted` appears before its corresponding `Entered`
/// - `TurnFailed` may close an open `Entered` (pessimistic reset)
fn assert_context_managed_pairing(events: &[ConversationEvent]) {
    let mut entered_without_completed: i64 = 0;
    for ev in events {
        match ev.payload {
            EventPayload::ContextManageEntered { .. } => entered_without_completed += 1,
            EventPayload::ContextManageCompleted { .. } => {
                assert!(
                    entered_without_completed > 0,
                    "ContextManageCompleted with no preceding Entered"
                );
                entered_without_completed -= 1;
            }
            EventPayload::TurnFailed { .. } => {
                // Turn failure closes any pending entered — pessimistic reset.
                entered_without_completed = 0;
            }
            _ => {}
        }
    }
    assert_eq!(
        entered_without_completed, 0,
        "unclosed ContextManageEntered count: {entered_without_completed}"
    );
}

// === Tests ===================================================================

#[tokio::test]
async fn chaos_y_path_every_event_boundary() {
    let scenarios = [
        chaos_scenarios::no_tool_short_turn(),
        chaos_scenarios::single_tool_happy_path(),
        chaos_scenarios::thinking_then_text_then_tool(),
    ];

    for scenario in &scenarios {
        let golden = run_to_completion_without_faults(scenario).await;
        let total = golden.events.len() as u64;
        let boundaries = resumable_boundaries(&golden.events);
        eprintln!(
            "scenario={} golden_events={total} resumable_boundaries={boundaries:?}",
            scenario.name
        );
        assert!(
            !boundaries.is_empty(),
            "scenario {} produced no resumable boundaries — \
             chaos test would be a no-op",
            scenario.name
        );

        for &crash_after_n in &boundaries {
            // Skip the very last resumable boundary if it lands at or past
            // the second-to-last event (phase 1 would already have written
            // TurnCompleted — no resume work).
            if crash_after_n >= total.saturating_sub(1) {
                continue;
            }
            let resumed = run_with_y_fault(scenario, crash_after_n).await;
            eprintln!(
                "  chaos: scenario={} crash_after_n={crash_after_n} resumed_len={} \
                 (golden_len={total})",
                scenario.name,
                resumed.len()
            );

            // Pairing check runs first so a missing Completed surfaces before
            // the prefix-immutability diff (which would be harder to diagnose).
            assert_context_managed_pairing(&resumed);
            assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
            assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
            assert_tool_mapping_equivalent(&golden.events, &resumed);
            assert_final_text_equivalent(&golden.events, &resumed);
        }
    }
}

// === Sprint 9a: strategy_with_tool_filter chaos scenario ====================
//
// Spec: docs/superpowers/specs/2026-05-27-sprint-9a-multi-model-strategy-design.md
//
// Goal: verify that a `HarnessStrategy` carrying `ToolFilter::Allow(...)` + a
// non-default `model_params.temperature` + a non-default `system_prompt`
// produces the same observable strategy effects on the post-resume side
// as on the golden side. Concretely:
//
//   - `PromptComposed.surface_size` equals the filtered count on BOTH the
//     golden and resumed runs (proves H05 honored the strategy after resume,
//     even though the strategy is re-derived from the registry, not from
//     the event log).
//   - The single `ToolUseRecorded` carries `tool_name = "read_file"`
//     (the only Allow-listed name) — proves the wider catalog never
//     reached the model.
//   - `ModelCallStarted.model` equals the strategy's `model_params.model`
//     across both runs — proves the model identifier survived the
//     re-derivation.
//
// Strategy choice rationale: `EventPayload::TurnStarted` does NOT carry
// strategy fields (it never has — strategies are re-derived from the
// registry deterministically, not persisted with the turn). The above
// three observable invariants are the chaos-visible footprint of the
// strategy. If `HarnessStrategy` is re-derived correctly on resume, they
// all match the golden run; if a resume code path accidentally lost or
// mutated the filter, the surface_size or tool_name diverges.
//
// The scenario shares `single_tool_happy_path`'s two-call model script,
// but uses a fresh `TwoToolMockProvider` (advertises both `read_file` and
// `write_file`) and a filtered strategy with `Allow(["read_file"])`. With
// `ToolFilter::All`, surface_size would be 2; the filter narrows it to 1.

/// In-test `ToolProvider` that exposes BOTH `read_file` and `write_file`.
/// Used by the `strategy_with_tool_filter` scenario so the strategy's
/// `Allow(["read_file"])` actually shrinks the surface (2 -> 1). The
/// model script only ever calls `read_file`; the second tool exists
/// solely to make the filter observable in `PromptComposed.surface_size`.
struct TwoToolMockProvider;

#[async_trait]
impl ToolProvider for TwoToolMockProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor {
                name: "read_file".into(),
                description: "test stub for chaos test (read).".into(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
                execution_class: ExecutionClass::AlwaysSync,
                outputs_model_visible_multimodal: false,
            },
            ToolDescriptor {
                name: "write_file".into(),
                description: "test stub for chaos test (write).".into(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
                execution_class: ExecutionClass::AlwaysSync,
                outputs_model_visible_multimodal: false,
            },
        ]
    }

    async fn invoke(&self, _name: &str, _args: serde_json::Value, _ctx: ExecCtx) -> InvokeOutcome {
        InvokeOutcome::Sync(ToolResult::text("MOCK_TOOL_RESULT"))
    }
}

/// Build the filtered-strategy chaos `HarnessStrategy`. Same shape as the
/// generic `default_with_model("mock-filtered")` factory output, but with
/// the four fields the chaos oracle asserts on flipped to non-defaults:
/// `name`, `system_prompt`, `allowed_tools` (Allow whitelist), and
/// `model_params.temperature`.
fn filtered_strategy() -> HarnessStrategy {
    let mut strategy = HarnessStrategy::default_with_model("mock-filtered");
    strategy.name = "filtered_coder".into();
    strategy.system_prompt = "Strict read-only coder.".into();
    strategy.allowed_tools = cogito_protocol::strategy::ToolFilter::Allow(vec!["read_file".into()]);
    strategy.model_params.temperature = Some(0.3);
    strategy
}

/// Wire a `Runtime` for the filtered-strategy chaos scenario. Same shape as
/// `build_runtime`, but uses `TwoToolMockProvider` so the filter is
/// observable in the recorded `PromptComposed.surface_size`, and the
/// filtered strategy.
fn build_filtered_runtime(
    store: Arc<dyn ConversationStore>,
    scenario: &ChaosScenario,
) -> Arc<Runtime> {
    let mock = Arc::new(build_scripted_mock(scenario));
    let tools = Arc::new(TwoToolMockProvider);
    Runtime::builder()
        .store(store)
        .model(mock as Arc<dyn ModelGateway>)
        .tools(tools as Arc<dyn ToolProvider>)
        .strategy(filtered_strategy())
        .build()
        .expect("runtime builds")
}

/// Drive the scenario to natural completion with the filtered strategy +
/// two-tool provider, no faults.
async fn filtered_run_to_completion(scenario: &ChaosScenario) -> GoldenRun {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_filtered_runtime(Arc::clone(&store), scenario);

    let session_id = SessionId::new();
    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");
    let _events_rx = handle.subscribe();

    handle
        .submit_user_text(extract_user_text(scenario))
        .await
        .expect("submit_user_text");
    wait_for_terminal_broadcast(&handle).await;

    let events = read_log(store.as_ref(), session_id).await;
    let terminal = terminal_payload(&events).clone();

    let out = handle
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got {out:?}"
    );
    drop(tmp);
    GoldenRun { events, terminal }
}

/// Y-path crash + resume runner for the filtered-strategy scenario. Mirrors
/// the generic `run_with_y_fault` but swaps in `build_filtered_runtime`
/// for both phases so phase 2 also gets the filtered strategy.
async fn filtered_run_with_y_fault(
    scenario: &ChaosScenario,
    crash_after_n: u64,
) -> Vec<ConversationEvent> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::new();

    // ----- Phase 1: drive until PanicAt fires. -----
    let inner1: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let wrapper1: Arc<FaultInjectingStore<JsonlStore>> =
        Arc::new(FaultInjectingStore::new(Arc::clone(&inner1)));
    let store1: Arc<dyn ConversationStore> = Arc::clone(&wrapper1) as Arc<dyn ConversationStore>;
    let runtime1 = build_filtered_runtime(store1, scenario);
    let handle1 = runtime1
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");
    wrapper1
        .set_trigger(FaultTrigger::PanicAt {
            event_no: crash_after_n + 1,
            message: "filtered-strategy chaos fault",
        })
        .await;

    handle1
        .submit_user_text(extract_user_text(scenario))
        .await
        .expect("submit_user_text");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = read_log(inner1.as_ref(), session_id).await;
        let has_terminal = events.iter().any(|e| {
            matches!(
                e.payload,
                EventPayload::TurnCompleted { .. }
                    | EventPayload::TurnFailed { .. }
                    | EventPayload::TurnPaused { .. }
            )
        });
        if has_terminal || wrapper1.written_count() > crash_after_n {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    drop(handle1);
    drop(runtime1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ----- Phase 2: fresh Runtime sharing the same on-disk JSONL. -----
    let inner2: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let store2: Arc<dyn ConversationStore> = inner2.clone();
    let runtime2 = build_filtered_runtime(Arc::clone(&store2), scenario);
    let handle2 = runtime2
        .open_session(session_id, OpenMode::Resume)
        .await
        .expect("open Resume");

    wait_for_terminal_with_store(&handle2, store2.as_ref(), session_id).await;

    let events = read_log(store2.as_ref(), session_id).await;
    let _ = handle2.shutdown(Duration::from_secs(5)).await;
    drop(tmp);
    events
}

/// Strategy-effect oracle for the `strategy_with_tool_filter` scenario.
///
/// `TurnStarted` does NOT carry strategy fields (the strategy is re-derived
/// from the registry, not persisted with the turn). The strategy's effects
/// instead surface in three downstream events:
///
/// - `PromptComposed.surface_size` reflects the post-filter tool surface.
///   With `TwoToolMockProvider` (2 tools) + `Allow(["read_file"])` the
///   filter narrows it to 1; with `ToolFilter::All` it would be 2. So a
///   `surface_size` of 1 proves the Allow filter was honored.
/// - `ToolUseRecorded.tool_name` MUST be `read_file` (the only allowed
///   tool). The model script never tries to call anything else, so this is
///   a passive guard: if a resume bug re-broadened the filter and the model
///   subsequently named a tool outside the allow-list, dispatching would
///   surface a failure, not a different `ToolUseRecorded` — but we still
///   pin the name for fast diagnosis.
/// - `ModelCallStarted.model` MUST equal the strategy's
///   `model_params.model` (`"mock-filtered"`).
fn assert_filtered_strategy_effects(events: &[ConversationEvent]) {
    let surface_sizes: Vec<u32> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::PromptComposed { surface_size, .. } => Some(*surface_size),
            _ => None,
        })
        .collect();
    assert!(
        !surface_sizes.is_empty(),
        "expected at least one PromptComposed event"
    );
    for sz in &surface_sizes {
        assert_eq!(
            *sz, 1,
            "expected PromptComposed.surface_size == 1 (Allow(read_file) filters 2 -> 1); \
             saw {sz} (full sequence: {surface_sizes:?})"
        );
    }

    let tool_names: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ToolUseRecorded { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !tool_names.is_empty(),
        "expected at least one ToolUseRecorded event"
    );
    for name in &tool_names {
        assert_eq!(
            *name, "read_file",
            "expected ToolUseRecorded.tool_name == read_file; saw {name} \
             (full sequence: {tool_names:?})"
        );
    }

    let models: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ModelCallStarted { model } => Some(model.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !models.is_empty(),
        "expected at least one ModelCallStarted event"
    );
    for m in &models {
        assert_eq!(
            *m, "mock-filtered",
            "expected ModelCallStarted.model == mock-filtered (strategy's model_params.model); \
             saw {m} (full sequence: {models:?})"
        );
    }
}

#[tokio::test]
async fn strategy_with_tool_filter() {
    let scenario = chaos_scenarios::single_tool_happy_path();
    let golden = filtered_run_to_completion(&scenario).await;
    let total = golden.events.len() as u64;
    let boundaries = resumable_boundaries(&golden.events);
    eprintln!(
        "scenario=strategy_with_tool_filter golden_events={total} \
         resumable_boundaries={boundaries:?}"
    );
    assert!(
        !boundaries.is_empty(),
        "filtered-strategy scenario produced no resumable boundaries"
    );

    // Golden run already exercises the filter; assert before we ever crash.
    assert_filtered_strategy_effects(&golden.events);

    for &crash_after_n in &boundaries {
        if crash_after_n >= total.saturating_sub(1) {
            continue;
        }
        let resumed = filtered_run_with_y_fault(&scenario, crash_after_n).await;
        eprintln!(
            "  filtered chaos: crash_after_n={crash_after_n} resumed_len={} (golden_len={total})",
            resumed.len()
        );

        assert_context_managed_pairing(&resumed);
        assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
        assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
        assert_tool_mapping_equivalent(&golden.events, &resumed);
        assert_final_text_equivalent(&golden.events, &resumed);

        // Strategy-effect oracle: the resumed log carries the SAME
        // filter / tool / model identifiers as the golden log, i.e. the
        // strategy was re-derived from the registry intact on resume.
        assert_filtered_strategy_effects(&resumed);
    }
}

// === Sprint 7: text_then_skill_then_tool chaos scenario =====================
//
// Spec: docs/superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md §11.
//
// Goal: verify that the H11 SkillInjector's idempotency and the H06 sigil
// recording survive a mid-flight crash. Spec §11 names two crash boundaries:
//   (a) after the turn-1 `AssistantMessageAppended` carrying `$foo`
//       (sigil text persisted, no SkillActivated written yet)
//   (b) after `SkillActivated` of turn 2, before its `SystemPromptInjected`
//       (partial-write; SkillInjector must skip re-emitting on resume)
//
// v0.1 narrowing (matching the rest of this test file): both spec boundaries
// land inside an in-flight turn where the harness has written `TurnStarted`
// but no `ModelCallCompleted` yet. H03 `replay()` classifies that into
// `RestartCurrentTurn`, which `apply_resume_point` downgrades to `FreshTurn`
// (the user_input recovery path is a documented TODO). So resuming from
// either spec boundary today would silently drop the in-flight turn and
// destroy the prefix-immutability oracle. Both boundaries are therefore
// shifted forward to the nearest `ResumeFromModelCompleted`-compatible
// boundary on the same conceptual "side" of the partial write:
//
//   - Spec (a) "after sigil text + before SkillActivated" → moved to
//     "after turn-1 final `ModelCallCompleted`" (one event before
//     `TurnCompleted`). At this point the sigil text is durably on disk;
//     resume fast-paths turn 1 to `TurnCompleted`. Turn 2's SkillInjector
//     still gets to scan the recovered sigil text.
//   - Spec (b) "after SkillActivated + before SystemPromptInjected" →
//     moved to "after turn-2's `ModelCallCompleted`" (one event before
//     turn 2's `TurnCompleted`). At this point BOTH SkillActivated and
//     SystemPromptInjected are durably on disk. Resume fast-paths turn 2
//     and verifies the SkillInjector idempotency check (`find_existing_
//     injection`) does not re-emit `SystemPromptInjected`. This is a
//     strictly weaker test than the spec's idealized boundary, but it's
//     the strongest one this v0.1 chaos infra can mechanically exercise.
//
// When `RestartCurrentTurn` is wired (post-Sprint-3 TODO in `session_loop.
// rs::apply_resume_point`), the boundaries should be tightened back to
// match the spec exactly.

/// Single-skill `SkillProvider`: registers "foo" with a recognizable body
/// so we can assert against the `<skill name="foo"` envelope in the
/// injected system-prompt suffix.
struct StaticFooSkillProvider;

const FOO_BODY: &str = "## foo\n\nA test skill body used by the chaos scenario.";
const FOO_DESCRIPTION: &str = "Test skill 'foo' for chaos coverage.";

impl cogito_protocol::skill::SkillProvider for StaticFooSkillProvider {
    fn list(&self) -> Vec<cogito_protocol::skill::SkillMetadata> {
        vec![cogito_protocol::skill::SkillMetadata {
            name: "foo".into(),
            description: FOO_DESCRIPTION.into(),
            source: cogito_protocol::skill::SkillSource::User,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }]
    }

    fn get(&self, name: &str) -> Option<cogito_protocol::skill::SkillContent> {
        if name == "foo" {
            Some(cogito_protocol::skill::SkillContent {
                name: "foo".into(),
                source: cogito_protocol::skill::SkillSource::User,
                body: FOO_BODY.into(),
                root: None,
            })
        } else {
            None
        }
    }

    fn is_registered(&self, name: &str) -> bool {
        name == "foo"
    }
}

/// Single-skill `SkillProvider` whose one skill is **plugin-loaded**: it
/// registers under the namespaced name `acme:review` with
/// `SkillSource::Plugin { plugin_id: "acme" }` (ADR-0021 §3). Drives the
/// `plugin_skill_then_tool` chaos scenario.
struct StaticPluginSkillProvider;

const PLUGIN_SKILL_NAME: &str = "acme:review";
const PLUGIN_SKILL_BODY: &str =
    "## review\n\nA plugin-loaded test skill body used by the chaos scenario.";
const PLUGIN_SKILL_DESCRIPTION: &str = "Plugin test skill 'acme:review' for chaos coverage.";

impl cogito_protocol::skill::SkillProvider for StaticPluginSkillProvider {
    fn list(&self) -> Vec<cogito_protocol::skill::SkillMetadata> {
        vec![cogito_protocol::skill::SkillMetadata {
            name: PLUGIN_SKILL_NAME.into(),
            description: PLUGIN_SKILL_DESCRIPTION.into(),
            source: cogito_protocol::skill::SkillSource::Plugin {
                plugin_id: "acme".into(),
            },
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }]
    }

    fn get(&self, name: &str) -> Option<cogito_protocol::skill::SkillContent> {
        if name == PLUGIN_SKILL_NAME {
            Some(cogito_protocol::skill::SkillContent {
                name: PLUGIN_SKILL_NAME.into(),
                source: cogito_protocol::skill::SkillSource::Plugin {
                    plugin_id: "acme".into(),
                },
                body: PLUGIN_SKILL_BODY.into(),
                root: None,
            })
        } else {
            None
        }
    }

    fn is_registered(&self, name: &str) -> bool {
        name == PLUGIN_SKILL_NAME
    }
}

/// Strategy with H11 `SystemPromptInjectorConfig::Skill`. Other slots are the
/// defaults from `default_with_model("mock")`.
fn skill_strategy() -> HarnessStrategy {
    let mut strategy = HarnessStrategy::default_with_model("mock");
    strategy.context.system_prompt_injector =
        cogito_protocol::context::SystemPromptInjectorConfig::Skill;
    strategy
}

/// Build a `ScriptedMockModel` for the two-turn `text_then_skill_then_tool`
/// flow. Three model calls total. Matchers are checked in declaration
/// order, first-match-wins:
///
/// 1. `LastUserTextContains("follow up")` → turn 2 call 1 (ack). Placed
///    first so turn 2's User message wins over the still-present turn-1
///    tool result.
/// 2. `LastToolResultContains("MOCK_TOOL_RESULT")` → turn 1 call 2 (after
///    `read_file` returns).
/// 3. `LastUserTextContains("please use $foo")` → turn 1 call 1 (sigil text
///    + tool use).
fn build_skill_chaos_model(scenario: &ChaosScenario) -> ScriptedMockModel {
    let matchers = vec![
        (
            InputMatcher::LastUserTextContains("follow up".into()),
            OutputScript {
                events: turn2_reply_script(),
            },
        ),
        (
            InputMatcher::LastToolResultContains("MOCK_TOOL_RESULT".into()),
            OutputScript {
                events: scenario.model_scripts[1].clone(),
            },
        ),
        (
            InputMatcher::LastUserTextContains(extract_user_text(scenario)),
            OutputScript {
                events: scenario.model_scripts[0].clone(),
            },
        ),
    ];
    ScriptedMockModel::new(matchers)
}

/// Turn 2 model script: a short ack with no tool use. Pure data lives here
/// (not in `chaos_scenarios.rs`) because turn 2 is conceptually a separate
/// model call wired by the runner, not part of the scenario's
/// `model_scripts` (which represents turn 1).
fn turn2_reply_script() -> Vec<cogito_protocol::gateway::ModelEvent> {
    use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
    vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "ack".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "ack".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 1,
            },
        },
    ]
}

/// Wire a `Runtime` for the skill chaos scenario. Same shape as the generic
/// `build_runtime`, but overrides the model, attaches a `SkillProvider`, and
/// uses the `SystemPromptInjectorConfig::Skill` strategy.
fn build_skill_runtime(
    store: Arc<dyn ConversationStore>,
    scenario: &ChaosScenario,
    skills: Arc<dyn cogito_protocol::skill::SkillProvider>,
) -> Arc<Runtime> {
    let mock = Arc::new(build_skill_chaos_model(scenario));
    let tools = Arc::new(MockToolProvider);
    Runtime::builder()
        .store(store)
        .model(mock as Arc<dyn ModelGateway>)
        .tools(tools as Arc<dyn ToolProvider>)
        .skills(skills)
        .strategy(skill_strategy())
        .build()
        .expect("runtime builds")
}

/// Drive both turns of the skill scenario to completion (no faults) and
/// return the full event log.
async fn skill_run_to_completion(
    scenario: &ChaosScenario,
    skills: Arc<dyn cogito_protocol::skill::SkillProvider>,
) -> GoldenRun {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_skill_runtime(Arc::clone(&store), scenario, skills);

    let session_id = SessionId::new();
    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");
    let _events_rx = handle.subscribe();

    // Turn 1: sigil text + tool.
    handle
        .submit_user_text(&extract_user_text(scenario))
        .await
        .expect("submit_user_text turn 1");
    wait_for_turn_completed(&handle).await;

    // Turn 2: follow-up. The SkillInjector re-derives "foo" from turn 1's
    // recorded assistant text and writes SkillActivated + SystemPromptInjected
    // with the body.
    handle
        .submit_user_text("follow up")
        .await
        .expect("submit_user_text turn 2");
    wait_for_turn_completed(&handle).await;

    let events = read_log(store.as_ref(), session_id).await;
    let terminal = terminal_payload(&events).clone();

    let out = handle
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got {out:?}"
    );
    drop(tmp);
    GoldenRun { events, terminal }
}

/// Wait for a turn's single `TurnCompleted` broadcast. Since ISSUE#69 part 2
/// was fixed, exactly one is emitted per turn (the FSM transition is the sole
/// emitter; the actor's terminal hook no longer re-records it). A subsequent
/// `submit_user_text` is still safe: the single-threaded session actor parks a
/// trigger arriving mid-retirement in the single-slot `pending_user_input`
/// queue and drains it on retirement, so it is never dropped. Mirrors
/// `h11_skill_injection::wait_for_turn_completed`.
async fn wait_for_turn_completed(handle: &SessionHandle) {
    let mut events_rx = handle.subscribe();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                // Stop on the turn's terminal broadcast; a stream error (lagged
                // or closed) likewise ends the wait — the timeout wrapper caps it.
                Ok(StreamEvent::TurnCompleted { .. }) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
}

/// Run the skill scenario with a `PanicAt(crash_after_n)` Y-path crash
/// somewhere across the two-turn flow, then resume in a fresh `Runtime`
/// sharing the same JSONL store. Returns the resumed log.
///
/// `turn1_total` counts how many appends to expect for turn 1 (everything
/// up through the second `TurnCompleted` event). Crash boundaries with
/// `crash_after_n <= turn1_total` panic mid-turn-1; boundaries past that
/// crash mid-turn-2.
/// The phase-1 actor is replayed forward (turn 1 submitted, then turn 2
/// submitted as soon as the on-disk log shows turn 1 has terminated) so
/// post-turn-1 boundaries are reachable.
async fn skill_run_with_y_fault(
    scenario: &ChaosScenario,
    crash_after_n: u64,
    turn1_total: u64,
    skills: Arc<dyn cogito_protocol::skill::SkillProvider>,
) -> Vec<ConversationEvent> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::new();

    // ----- Phase 1: drive both turns; fault may fire in either. -----
    let inner1: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let wrapper1: Arc<FaultInjectingStore<JsonlStore>> =
        Arc::new(FaultInjectingStore::new(Arc::clone(&inner1)));
    let store1: Arc<dyn ConversationStore> = Arc::clone(&wrapper1) as Arc<dyn ConversationStore>;
    let runtime1 = build_skill_runtime(store1, scenario, Arc::clone(&skills));
    let handle1 = runtime1
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");

    // Arm the trigger AFTER open_session so seq=0 SessionStarted is not the
    // panic target. `crash_after_n` is 1-indexed against turn-event seq, so
    // the trigger fires after `crash_after_n + 1` total appends (the +1
    // accounts for SessionStarted).
    wrapper1
        .set_trigger(FaultTrigger::PanicAt {
            event_no: crash_after_n + 1,
            message: "skill chaos fault",
        })
        .await;

    handle1
        .submit_user_text(&extract_user_text(scenario))
        .await
        .expect("submit_user_text turn 1");

    // Wait until either the panic fires or turn 1 reaches a terminal. We
    // do not subscribe to TurnCompleted because the actor may already be
    // dead by the time we'd start listening; polling the on-disk log is
    // panic-tolerant.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut turn1_seen_terminal = false;
    loop {
        if log_has_terminal(inner1.as_ref(), session_id).await {
            turn1_seen_terminal = true;
            break;
        }
        if wrapper1.written_count() > crash_after_n {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // If turn 1 terminated cleanly and the crash boundary is past turn 1,
    // submit turn 2 from the SAME handle so the second-turn appends land in
    // the still-live actor; this is the only way to reach turn-2 boundaries.
    if turn1_seen_terminal && crash_after_n > turn1_total {
        // Allow the actor's on_turn_complete hook to run so the
        // `in_flight = None` guard releases.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = handle1.submit_user_text("follow up").await;
        let deadline2 = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let events = read_log(inner1.as_ref(), session_id).await;
            // Count distinct turns by TurnStarted events; require >= 2 to
            // confirm turn 2 has at least begun.
            let turns_started = events
                .iter()
                .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
                .count();
            // Done condition: both turns terminated (i.e. >= 2 TurnStarted
            // AND >= 2 terminals, one per turn since ISSUE#69 part 2 made
            // TurnCompleted single-emit).
            let terminal_count = events
                .iter()
                .filter(|e| {
                    matches!(
                        e.payload,
                        EventPayload::TurnCompleted { .. }
                            | EventPayload::TurnFailed { .. }
                            | EventPayload::TurnPaused { .. }
                    )
                })
                .count();
            let both_turns_done = turns_started >= 2 && terminal_count >= 2;
            if both_turns_done || wrapper1.written_count() > crash_after_n {
                break;
            }
            if tokio::time::Instant::now() >= deadline2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    drop(handle1);
    drop(runtime1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ----- Phase 2: fresh Runtime, same on-disk JSONL, Resume mode. -----
    let inner2: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let store2: Arc<dyn ConversationStore> = inner2.clone();
    let runtime2 = build_skill_runtime(Arc::clone(&store2), scenario, Arc::clone(&skills));

    let handle2 = runtime2
        .open_session(session_id, OpenMode::Resume)
        .await
        .expect("open Resume");

    // Drive turn 1 to terminal (resume-from-crashed-turn-1) OR turn 2 to
    // terminal (turn 1 already done, turn 2 crashed). The phase-2 handle
    // only knows what's on disk; if turn 2 was never submitted (i.e. phase 1
    // crashed mid-turn-1), we need to submit "follow up" again so turn 2 runs.
    wait_for_terminal_with_store(&handle2, store2.as_ref(), session_id).await;

    let mid_events = read_log(store2.as_ref(), session_id).await;
    let turns_started = mid_events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .count();

    // If the resumed log only has turn 1 begun (no turn 2 TurnStarted),
    // submit turn 2 ourselves so the chaos scenario observes the same
    // two-turn flow as the golden.
    if turns_started < 2 {
        // Brief settling pause so the resume-induced TurnCompleted for
        // turn 1 has flushed before we kick off turn 2.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = handle2.submit_user_text("follow up").await;
        // Wait until both turns have terminated (turn 2's TurnCompleted is the
        // second terminal). Since wait_for_terminal_with_store short-circuits
        // on any prior terminal, we poll the log instead.
        let deadline3 = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let events = read_log(store2.as_ref(), session_id).await;
            let ts = events
                .iter()
                .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
                .count();
            let tc = events
                .iter()
                .filter(|e| {
                    matches!(
                        e.payload,
                        EventPayload::TurnCompleted { .. }
                            | EventPayload::TurnFailed { .. }
                            | EventPayload::TurnPaused { .. }
                    )
                })
                .count();
            // One terminal per turn since ISSUE#69 part 2 (single-emit).
            if ts >= 2 && tc >= 2 {
                break;
            }
            if tokio::time::Instant::now() >= deadline3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    let events = read_log(store2.as_ref(), session_id).await;
    let _ = handle2.shutdown(Duration::from_secs(5)).await;
    drop(tmp);
    events
}

/// Locate the two `ResumeFromModelCompleted`-compatible crash boundaries
/// chosen for `text_then_skill_then_tool`. Returns 1-indexed turn-event-
/// indices into the golden log. See the file-level comment for why these
/// differ from the spec's idealized boundaries.
///
/// - `boundary_1`: the first `ModelCallCompleted` in the log with
///   `stop_reason` `EndTurn` (i.e., turn 1's final model call, which closes
///   the sigil-bearing turn). Crash here = "sigil text persisted, turn 1
///   resumes cleanly".
/// - `boundary_2`: the LAST `ModelCallCompleted` in the log with
///   `stop_reason` `EndTurn` (i.e., turn 2's only model call, which closes
///   the `SkillActivated`-bearing turn). Crash here = "`SkillActivated` +
///   `SystemPromptInjected` persisted, turn 2 resumes cleanly".
fn skill_crash_boundaries(events: &[ConversationEvent]) -> (Option<u64>, Option<u64>) {
    use cogito_protocol::gateway::StopReason;
    let mut endturns: Vec<u64> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if let EventPayload::ModelCallCompleted { stop_reason, .. } = &e.payload {
            if matches!(stop_reason, StopReason::EndTurn) {
                endturns.push((i + 1) as u64);
            }
        }
    }
    let b1 = endturns.first().copied();
    let b2 = endturns.last().copied();
    // If b1 == b2 (e.g. golden produced only one EndTurn), the test
    // becomes a single-boundary check, but the spec demands two distinct
    // events; surface that as the caller's responsibility.
    (b1, b2)
}

/// Count how many events make up the turn 1 prefix in the golden log
/// (everything through the FIRST `TurnCompleted`, inclusive). Used by the
/// fault runner to decide whether to submit turn 2 on the live handle.
fn turn1_event_count(events: &[ConversationEvent]) -> u64 {
    for (i, e) in events.iter().enumerate() {
        if matches!(e.payload, EventPayload::TurnCompleted { .. }) {
            return (i + 1) as u64;
        }
    }
    events.len() as u64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_then_skill_then_tool() {
    let scenario = chaos_scenarios::text_then_skill_then_tool();
    let skills: Arc<dyn cogito_protocol::skill::SkillProvider> = Arc::new(StaticFooSkillProvider);

    let golden = skill_run_to_completion(&scenario, Arc::clone(&skills)).await;
    let turn1_total = turn1_event_count(&golden.events);

    eprintln!(
        "scenario=text_then_skill_then_tool golden_events={} turn1_total={turn1_total}",
        golden.events.len()
    );

    // Skill-specific golden assertions: the resumed and golden logs must each
    // contain exactly one SkillActivated{name=foo} and turn 2's
    // SystemPromptInjected must carry the <skill name="foo"> body.
    assert_skill_activated_once(&golden.events, "foo");
    assert_turn2_suffix_has_skill_body(&golden.events, "foo");

    let (b1, b2) = skill_crash_boundaries(&golden.events);
    let b1 = b1.expect("golden must contain at least one EndTurn ModelCallCompleted (turn 1)");
    let b2 = b2.expect("golden must contain at least one EndTurn ModelCallCompleted (turn 2)");
    assert_ne!(
        b1, b2,
        "scenario must produce two distinct EndTurn ModelCallCompleted events; got both at {b1}"
    );

    eprintln!(
        "  skill chaos boundaries: b1={b1} (post turn-1 final MCC), \
         b2={b2} (post turn-2 final MCC)"
    );

    for &crash_after_n in &[b1, b2] {
        let resumed =
            skill_run_with_y_fault(&scenario, crash_after_n, turn1_total, Arc::clone(&skills))
                .await;
        eprintln!(
            "  skill chaos: crash_after_n={crash_after_n} resumed_len={}",
            resumed.len()
        );

        assert_context_managed_pairing(&resumed);
        assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
        assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
        assert_tool_mapping_equivalent(&golden.events, &resumed);
        assert_final_text_equivalent(&golden.events, &resumed);

        // Skill-specific oracle: post-resume the log still records exactly
        // one SkillActivated{name=foo} (no double-injection on resume).
        assert_skill_activated_once(&resumed, "foo");
        // Skill-specific oracle: the final suffix in the resumed log still
        // carries the skill body (proves SystemPromptInjected was written
        // exactly once after resume).
        assert_turn2_suffix_has_skill_body(&resumed, "foo");
    }
}

// === Sprint 13: plugin_skill_then_tool chaos scenario =======================
//
// Identical mechanics to `text_then_skill_then_tool`, but the activated skill
// is plugin-loaded: it carries `SkillSource::Plugin { plugin_id: "acme" }` and
// is addressed by its namespaced name `acme:review` via the `$acme:review`
// sigil (ADR-0021 §3 — the sigil regex admits `:`). The same two
// `ResumeFromModelCompleted`-compatible crash boundaries are exercised; a crash
// while the plugin skill is mid-activation must still leave exactly one
// `SkillActivated{acme:review}` and one skill-body `SystemPromptInjected` after
// resume. This proves the H06 sigil / H11 injection idempotency path is
// source-agnostic (Plugin scope behaves like User/Repo scope under resume).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_skill_then_tool() {
    let scenario = chaos_scenarios::plugin_skill_then_tool();
    let skills: Arc<dyn cogito_protocol::skill::SkillProvider> =
        Arc::new(StaticPluginSkillProvider);

    let golden = skill_run_to_completion(&scenario, Arc::clone(&skills)).await;
    let turn1_total = turn1_event_count(&golden.events);

    eprintln!(
        "scenario=plugin_skill_then_tool golden_events={} turn1_total={turn1_total}",
        golden.events.len()
    );

    // Plugin-skill golden assertions: exactly one SkillActivated{acme:review}
    // and turn 2's SystemPromptInjected carries the <skill name="acme:review">
    // body.
    assert_skill_activated_once(&golden.events, PLUGIN_SKILL_NAME);
    assert_turn2_suffix_has_skill_body(&golden.events, PLUGIN_SKILL_NAME);

    let (b1, b2) = skill_crash_boundaries(&golden.events);
    let b1 = b1.expect("golden must contain at least one EndTurn ModelCallCompleted (turn 1)");
    let b2 = b2.expect("golden must contain at least one EndTurn ModelCallCompleted (turn 2)");
    assert_ne!(
        b1, b2,
        "scenario must produce two distinct EndTurn ModelCallCompleted events; got both at {b1}"
    );

    eprintln!(
        "  plugin skill chaos boundaries: b1={b1} (post turn-1 final MCC), \
         b2={b2} (post turn-2 final MCC)"
    );

    for &crash_after_n in &[b1, b2] {
        let resumed =
            skill_run_with_y_fault(&scenario, crash_after_n, turn1_total, Arc::clone(&skills))
                .await;
        eprintln!(
            "  plugin skill chaos: crash_after_n={crash_after_n} resumed_len={}",
            resumed.len()
        );

        assert_context_managed_pairing(&resumed);
        assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
        assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
        assert_tool_mapping_equivalent(&golden.events, &resumed);
        assert_final_text_equivalent(&golden.events, &resumed);

        // Plugin-skill oracles: exactly one SkillActivated{acme:review} and the
        // skill body still injected exactly once after resume.
        assert_skill_activated_once(&resumed, PLUGIN_SKILL_NAME);
        assert_turn2_suffix_has_skill_body(&resumed, PLUGIN_SKILL_NAME);
    }
}

/// Assert that the log contains exactly one `SkillActivated` event whose
/// `skill_name` equals `name`.
fn assert_skill_activated_once(events: &[ConversationEvent], name: &str) {
    let count = events
        .iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::SkillActivated { skill_name, .. } if skill_name == name
            )
        })
        .count();
    assert_eq!(
        count, 1,
        "expected exactly one SkillActivated{{name={name}}}, got {count}"
    );
}

/// Assert that the last `SystemPromptInjected.suffix` in the log contains
/// the XML-wrapped skill body (i.e. turn 2 successfully injected the
/// activated skill).
fn assert_turn2_suffix_has_skill_body(events: &[ConversationEvent], name: &str) {
    let last_suffix = events
        .iter()
        .rev()
        .find_map(|e| match &e.payload {
            EventPayload::SystemPromptInjected { suffix, .. } => Some(suffix.clone()),
            _ => None,
        })
        .expect("at least one SystemPromptInjected in log");
    let needle = format!("<skill name=\"{name}\"");
    assert!(
        last_suffix.contains(&needle),
        "expected final suffix to contain skill body for {name}, got: {last_suffix}"
    );
}

// TODO(post-Sprint-3): X path with poisoned-actor detection from outside.
// Today the actor task panic just gets caught by tokio; the test side
// detects "phase 1 is done" by polling the on-disk log + the wrapper's
// written_count. A real X path would expose the actor JoinHandle so the
// driver can `catch_unwind` on it and assert "the task died at boundary N".
// Deferred to a follow-up after Sprint 3 lands.
//
// TODO(post-Sprint-3): expand the set of resumable boundaries once
// (a) `RestartCurrentTurn` is wired in `actor.rs::apply_resume_point`
// (recovers user_input from initial_events) and
// (b) `resume_from_turn_started` in `harness/resume.rs` is fixed to look
// for `ToolUseRecorded` BEFORE `ModelCallCompleted` (matching the actual
// H06 write order). Both are pre-existing v0.1 gaps, not regressions
// introduced by this test.

// === Sprint 8: paused_async_job chaos scenario ==============================
//
// Spec: docs/superpowers/specs/2026-05-24-sprint-8-async-jobs-design.md §10.
//
// Goal: verify that the H08 async-job loop and H03's paused-job resume
// points survive a mid-flight crash. Three crash boundaries are exercised:
//
//   (a) After `JobSubmitted` but BEFORE `TurnPaused`. H03 sees the latest
//       turn boundary is `TurnStarted` (JobSubmitted is not a turn-boundary
//       event), so `resume_from_turn_started` classifies the in-flight turn
//       into `ResumeFromToolDispatching` with the async tool's
//       ToolUseRecorded still pending. The dispatcher re-invokes the tool,
//       which submits a NEW job_id against the fresh `MockJobManager`. The
//       fresh manager auto-fires `complete(new_job_id, Success)` from the
//       same canned future, the turn pauses on the new job, then resumes
//       cleanly to `TurnCompleted`. Prefix immutability holds up to the
//       crash point; the duplicate `JobSubmitted` (one persisted, one
//       new) lives strictly past the prefix.
//
//   (b) After `TurnPaused` but BEFORE `JobCompletedRecorded`. H03 sees
//       `TurnPaused` as the latest turn boundary and scans forward for a
//       matching `JobCompletedRecorded`. None present, so the decision is
//       `ResumePausedJob`. `apply_resume_point` calls `on_complete(job_id)`
//       on the fresh `MockJobManager`, which returns `JobError::UnknownJob`
//       because the persisted `job_id` was minted by the phase-1 manager
//       that died with the actor. The session loop then synthesises a
//       `JobOutcome::Failed { message: "lost across process restart" }` on
//       the session's own Arm 3 channel. The synthetic completion writes
//       `JobCompletedRecorded { Failed }` then `ToolResultRecorded
//       { AsyncFailed }`, respawns the TurnDriver, and drives the second
//       model call. This is Task 13's lost-job synthesis path.
//
//   (c) After `JobCompletedRecorded` but BEFORE `ToolResultRecorded`. H03
//       sees `TurnPaused` as the latest turn boundary AND finds a matching
//       `JobCompletedRecorded { Success }` after it. The decision is
//       `ResumeAfterJobCompletion`. `apply_resume_point` checks whether
//       `ToolResultRecorded` for the async call_id is already in the log
//       (Task 17's dedup); if not, records it once and spawns the
//       TurnDriver with the resolved completion. The second model call
//       drives the turn to `TurnCompleted` carrying the same `"slept"`
//       outcome the golden run produced.
//
// Oracle invariants per boundary:
//
//   (a) and (c) — all 4 standard oracles satisfy. The resumed log's
//       canonical projection beyond the crash point may differ in detail
//       (a re-dispatched JobSubmitted carries a new ULID; the dedup path
//       inserts ToolResultRecorded exactly once) but the four equivalence
//       invariants (prefix, terminal kind, tool mapping, final text) hold.
//
//   (b) — lost-job synthesis produces a tool mapping
//       `(c_async -> (mock_async, {}, AsyncFailed))` while the golden run
//       has `(c_async -> (mock_async, {}, "slept"))`. This is a
//       *legitimate* divergence: the spec explicitly states that the lost
//       job becomes a structured failure on the model side, not a
//       reproduction of the original success. Oracle 3 is therefore
//       relaxed for this boundary; we still verify oracles 1, 2, 4 and
//       additionally assert that the resumed log contains the canonical
//       `Failed { "lost across process restart" }` outcome.

/// Async-tool fixture for the chaos scenario. Each `invoke` call registers
/// a new `JobId` with the bound [`MockJobManager`] and spawns a tokio task
/// that fires `complete(job_id, Success { result: "slept" })` after a
/// short delay. The tool is shaped like `cogito_jobs::SleepTool` but uses
/// `MockJobManager` so the test can swap a fresh manager into phase 2 and
/// trigger Task 13's lost-job synthesis path on demand.
struct AsyncMockSleepTool {
    /// Shared job manager used by both `invoke` (which registers + spawns
    /// the completion future) and the Runtime (which calls
    /// `on_complete`). The same `Arc` MUST be threaded into both halves
    /// per ADR-0008; otherwise the manager that fires `complete()` is
    /// disjoint from the one the actor registered against and the sink
    /// never fires.
    job_mgr: Arc<MockJobManager>,
}

impl AsyncMockSleepTool {
    fn new(job_mgr: Arc<MockJobManager>) -> Self {
        Self { job_mgr }
    }
}

#[async_trait]
impl ToolProvider for AsyncMockSleepTool {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: "mock_async".into(),
            description: "Async-tool fixture for chaos coverage; resolves with 'slept'.".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysAsync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, _name: &str, _args: serde_json::Value, _ctx: ExecCtx) -> InvokeOutcome {
        let job_id = JobId::default();
        self.job_mgr.register(job_id).await;
        // Auto-complete the job from a spawned task. The delay is short
        // enough to keep the test fast but long enough that the dispatcher
        // can record `JobSubmitted` + register the `on_complete` sink
        // before the completion fires. (`MockJobManager::complete` honors
        // the "fire immediately if a sink is already registered, store the
        // outcome otherwise" contract, so the ordering between dispatch
        // and completion does not actually matter for correctness; we
        // still sleep briefly to make the trajectory more realistic.)
        let mgr = Arc::clone(&self.job_mgr);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            mgr.complete(
                job_id,
                JobOutcome::Success {
                    result: ToolResult::text("slept"),
                },
            )
            .await;
        });
        InvokeOutcome::Async(job_id)
    }
}

/// Build the scripted mock model for the `paused_async_job` scenario. Two
/// model calls in turn 1: the first emits `tool_use(mock_async)`, the
/// second (regardless of tool outcome) emits a constant final text +
/// `EndTurn`. The fallback matcher is `InputMatcher::Any` for the second
/// call, gated by the presence of a tool result in the input messages so
/// that ordering is deterministic across the success path (golden +
/// boundary c) and the lost-job synthesis path (boundary b's `AsyncFailed`
/// result).
fn build_async_chaos_model() -> ScriptedMockModel {
    use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};

    // Turn 1 call 1: announce + tool_use(mock_async), stop_reason=tool_use.
    let call1 = OutputScript {
        events: vec![
            ModelEvent::ToolUseStarted {
                block_index: 0,
                call_id: "c_async".into(),
                tool_name: "mock_async".into(),
            },
            ModelEvent::ToolUseCompleted {
                block_index: 0,
                call_id: "c_async".into(),
                tool_name: "mock_async".into(),
                args: serde_json::json!({}),
            },
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            },
        ],
    };
    // Turn 1 call 2 (post-tool): assistant emits a constant reply +
    // EndTurn. Independent of the tool result shape so the lost-job
    // synthesis path produces the same final text as the success path.
    let call2 = OutputScript {
        events: vec![
            ModelEvent::TextDelta {
                block_index: 0,
                chunk: "Done.".into(),
            },
            ModelEvent::TextBlockCompleted {
                block_index: 0,
                text: "Done.".into(),
            },
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            },
        ],
    };
    // The 2nd call's `ModelInput` will contain a ToolResult block (Output
    // "slept" on the success path, Error{AsyncFailed} on the lost-job
    // path). `LastToolResultContains("")` matches any ToolResult block
    // (the empty substring is contained in every string), so it triggers
    // on both shapes. The fallback `Any` matcher dispatches the first
    // call.
    ScriptedMockModel::new(vec![
        (InputMatcher::LastToolResultContains(String::new()), call2),
        (InputMatcher::Any, call1),
    ])
}

/// Wire a `Runtime` for the `paused_async_job` chaos scenario. The
/// `MockJobManager` is shared with `AsyncMockSleepTool` so the tool's job
/// submission and the actor's `on_complete` registration resolve against
/// the same instance. Each phase 1 / phase 2 builds a separate Runtime
/// (and therefore a separate `MockJobManager`), which is exactly what
/// drives boundary (b)'s lost-job synthesis: the phase-2 manager does not
/// know the `job_id` persisted by phase 1.
fn build_async_runtime(store: Arc<dyn ConversationStore>) -> (Arc<Runtime>, Arc<MockJobManager>) {
    let mock = Arc::new(build_async_chaos_model());
    let job_mgr = Arc::new(MockJobManager::new());
    let tool: Arc<dyn ToolProvider> = Arc::new(AsyncMockSleepTool::new(Arc::clone(&job_mgr)));
    let runtime = Runtime::builder()
        .store(store)
        .model(mock as Arc<dyn ModelGateway>)
        .tools(tool)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()
        .expect("runtime builds");
    (runtime, job_mgr)
}

/// Run the `paused_async_job` scenario to natural completion without any
/// injected faults. Returns the golden log + terminal payload.
async fn async_run_to_completion() -> GoldenRun {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let (runtime, _job_mgr) = build_async_runtime(Arc::clone(&store));

    let session_id = SessionId::new();
    let handle = runtime
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");

    handle
        .submit_user_text("run the async tool")
        .await
        .expect("submit_user_text");
    // The paused-async-job flow emits TurnPaused before the auto-completed
    // job drives the turn to TurnCompleted. The generic helper returns on
    // TurnPaused, which would leave the golden log without the post-pause
    // events we need to compare against. Wait for TurnCompleted / Failed /
    // Cancelled specifically.
    //
    // We poll the on-disk log rather than relying solely on the broadcast
    // channel: on resource-constrained CI runners the broadcast receiver
    // can Lagged-error out before the actor finishes writing all events,
    // producing an incomplete golden log (only 21 events instead of 23).
    // The disk is the source of truth.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let events = read_log(store.as_ref(), session_id).await;
        let has_closed_terminal = events.iter().any(|e| {
            matches!(
                e.payload,
                EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. }
            )
        });
        if has_closed_terminal {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "golden run did not reach TurnCompleted within 10s; \
             log has {} events, terminal={:?}",
            events.len(),
            terminal_payload(&events)
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let events = read_log(store.as_ref(), session_id).await;
    let terminal = terminal_payload(&events).clone();

    let out = handle
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got {out:?}"
    );

    drop(tmp);
    GoldenRun { events, terminal }
}

/// Run the `paused_async_job` scenario with a `PanicAt(crash_after_n)` Y-path
/// crash and then resume in a fresh `Runtime` (with a fresh
/// `MockJobManager`). Returns the resumed log.
///
/// `crash_after_n` is a 1-indexed turn-event-seq (matching the existing
/// chaos harness). The trigger fires after `crash_after_n + 1` total
/// appends to account for the seq=0 `SessionStarted` write.
async fn async_run_with_y_fault(crash_after_n: u64) -> Vec<ConversationEvent> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::new();

    // ----- Phase 1: drive until PanicAt fires inside the actor task. -----
    let inner1: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let wrapper1: Arc<FaultInjectingStore<JsonlStore>> =
        Arc::new(FaultInjectingStore::new(Arc::clone(&inner1)));
    let store1: Arc<dyn ConversationStore> = Arc::clone(&wrapper1) as Arc<dyn ConversationStore>;
    let (runtime1, _job_mgr1) = build_async_runtime(store1);

    let handle1 = runtime1
        .open_session(session_id, OpenMode::New)
        .await
        .expect("open New");
    wrapper1
        .set_trigger(FaultTrigger::PanicAt {
            event_no: crash_after_n + 1,
            message: "async chaos fault",
        })
        .await;

    handle1
        .submit_user_text("run the async tool")
        .await
        .expect("submit_user_text");

    // Wait until the panic lands or the turn reaches a terminal. The
    // async tool auto-completes after ~20 ms, so without a crash the turn
    // reaches `TurnCompleted` quickly; with a crash, the actor task dies
    // silently and we detect it via `wrapper.written_count > crash_after_n`.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = read_log(inner1.as_ref(), session_id).await;
        let has_terminal = events.iter().any(|e| {
            matches!(
                e.payload,
                EventPayload::TurnCompleted { .. }
                    | EventPayload::TurnFailed { .. }
                    | EventPayload::TurnPaused { .. }
            )
        });
        // Phase 1 is "done enough" once either: (a) the panic landed
        // (written_count exceeded the boundary), or (b) a terminal hit
        // disk. For an async-paused turn, `TurnPaused` is recorded but
        // does NOT mean the turn is dead — the actor is alive waiting
        // for the auto-complete. We must let the auto-complete fire and
        // drive past TurnPaused so phase-2 boundaries that live past
        // the pause (c) are actually reachable.
        if wrapper1.written_count() > crash_after_n {
            break;
        }
        // If `TurnCompleted` or `TurnFailed` is on disk, the turn is
        // genuinely terminal; bail out.
        let has_real_terminal = events.iter().any(|e| {
            matches!(
                e.payload,
                EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. }
            )
        });
        if has_real_terminal {
            break;
        }
        // If the turn paused but the panic boundary is well past TurnPaused,
        // keep spinning so the auto-complete can fire and the actor can
        // re-enter ToolDispatching past TurnPaused into JobCompletedRecorded
        // / ToolResultRecorded territory.
        let _ = has_terminal;
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    drop(handle1);
    drop(runtime1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ----- Phase 2: fresh Runtime (and fresh MockJobManager) on the same
    // on-disk JSONL, Resume mode. The fresh manager NOT knowing the
    // persisted `job_id` is the trigger for boundary (b)'s lost-job
    // synthesis path. -----
    let inner2: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let store2: Arc<dyn ConversationStore> = inner2.clone();
    let (runtime2, _job_mgr2) = build_async_runtime(Arc::clone(&store2));

    let handle2 = runtime2
        .open_session(session_id, OpenMode::Resume)
        .await
        .expect("open Resume");

    // Wait specifically for a CLOSED terminal (TurnCompleted / TurnFailed
    // / TurnCancelled). The generic `wait_for_terminal_with_store` helper
    // bails on `TurnPaused`, but for the paused-async-job flow
    // `TurnPaused` is an intermediate state — the resume path must drive
    // past it (re-dispatch the tool, or unwind the lost job, or apply the
    // dedup branch). Bail early if the store already contains a closed
    // terminal so we never hang waiting for a broadcast that will never
    // fire.
    let already_closed = read_log(store2.as_ref(), session_id).await.iter().any(|e| {
        matches!(
            e.payload,
            EventPayload::TurnCompleted { .. } | EventPayload::TurnFailed { .. }
        )
    });
    if !already_closed {
        let mut events_rx = handle2.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events_rx.recv().await {
                    Ok(
                        StreamEvent::TurnCompleted { .. }
                        | StreamEvent::TurnFailed { .. }
                        | StreamEvent::TurnCancelled { .. },
                    )
                    | Err(_) => return,
                    Ok(_) => {}
                }
            }
        })
        .await;
    }

    let events = read_log(store2.as_ref(), session_id).await;
    let _ = handle2.shutdown(Duration::from_secs(5)).await;

    drop(tmp);
    events
}

/// Locate the three paused-async-job crash boundaries in the golden log.
/// Returns 1-indexed turn-event-seqs matching `crash_after_n`:
///
/// - `b_post_job_submitted`: index of the `JobSubmitted` event (crash
///   between `JobSubmitted` and `TurnPaused`).
/// - `b_post_turn_paused`: index of the `TurnPaused` event (crash between
///   `TurnPaused` and `JobCompletedRecorded` — lost-job synthesis path).
/// - `b_post_job_completed`: index of the `JobCompletedRecorded` event
///   (crash between `JobCompletedRecorded` and `ToolResultRecorded` — dedup
///   path).
fn async_crash_boundaries(events: &[ConversationEvent]) -> (Option<u64>, Option<u64>, Option<u64>) {
    let mut job_submitted: Option<u64> = None;
    let mut turn_paused: Option<u64> = None;
    let mut job_completed: Option<u64> = None;
    for (i, e) in events.iter().enumerate() {
        let idx_1b = (i + 1) as u64;
        match &e.payload {
            EventPayload::JobSubmitted { .. } if job_submitted.is_none() => {
                job_submitted = Some(idx_1b);
            }
            EventPayload::TurnPaused { .. } if turn_paused.is_none() => {
                turn_paused = Some(idx_1b);
            }
            EventPayload::JobCompletedRecorded { .. } if job_completed.is_none() => {
                job_completed = Some(idx_1b);
            }
            _ => {}
        }
    }
    (job_submitted, turn_paused, job_completed)
}

/// Assert that the resumed log records the canonical lost-job synthesis
/// outcome at least once. Used for boundary (b) where the phase-2
/// `MockJobManager` does not know the persisted `job_id`.
fn assert_lost_job_synthesis_recorded(events: &[ConversationEvent]) {
    let found = events.iter().any(|e| match &e.payload {
        EventPayload::JobCompletedRecorded {
            outcome: JobOutcome::Failed { message },
            ..
        } => message == "lost across process restart",
        _ => false,
    });
    assert!(
        found,
        "expected `JobCompletedRecorded {{ Failed {{ \"lost across process restart\" }} }}` \
         in resumed log; got: {:?}",
        events.iter().map(|e| &e.payload).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_async_job() {
    let golden = async_run_to_completion().await;
    let total = golden.events.len() as u64;
    let (b_submitted, b_paused, b_completed) = async_crash_boundaries(&golden.events);
    let b_submitted = b_submitted.expect("golden must contain a JobSubmitted event");
    let b_paused = b_paused.expect("golden must contain a TurnPaused event");
    let b_completed =
        b_completed.expect("golden must contain a JobCompletedRecorded event in the golden path");
    eprintln!(
        "scenario=paused_async_job golden_events={total} boundaries: \
         post_JobSubmitted={b_submitted} post_TurnPaused={b_paused} \
         post_JobCompletedRecorded={b_completed}"
    );

    // (boundary, expects_lost_job_synthesis) — the synthesis path
    // legitimately diverges from the golden tool result, so oracle 3
    // (tool mapping) is relaxed for that boundary and a dedicated
    // `assert_lost_job_synthesis_recorded` runs instead.
    let cases: [(u64, bool); 3] = [(b_submitted, false), (b_paused, true), (b_completed, false)];

    for (crash_after_n, expects_synthesis) in cases {
        let resumed = async_run_with_y_fault(crash_after_n).await;
        eprintln!(
            "  async chaos: crash_after_n={crash_after_n} \
             expects_lost_job_synthesis={expects_synthesis} resumed_len={}",
            resumed.len()
        );

        assert_context_managed_pairing(&resumed);
        assert_prefix_immutable(&golden.events, &resumed, crash_after_n);
        assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
        assert_final_text_equivalent(&golden.events, &resumed);

        if expects_synthesis {
            // Lost-job synthesis produces ToolResult::Error{AsyncFailed}
            // instead of the golden Output("slept"). Verify the synthesis
            // actually fired (Task 13's canonical signature) in lieu of
            // oracle 3.
            assert_lost_job_synthesis_recorded(&resumed);
        } else {
            assert_tool_mapping_equivalent(&golden.events, &resumed);
        }
    }
}

// === ADR-0028 Task 4: session_spec_mutated_then_resume ======================
//
// Plan: docs/adr/ADR-0028 (per-session-provider-injection).
//
// Goal: a session whose provider surface was swapped mid-run (via
// `SessionHandle::update_session`) must still resume correctly when the
// caller re-supplies the mutated spec on
// `open_session_with(id, OpenMode::Resume, spec_b)`.
//
// Shape (two-turn flow, both turns plain TEXT — no tool calls):
//
//   1. Phase 1 opens with spec_a (`MockToolProvider`, 1 tool) via
//      `open_session_with(id, New, spec_a)`.
//   2. Turn 1 runs to completion (a plain "say hi" text turn).
//   3. `handle.update_session(spec_b)` swaps the live tool provider to
//      `TwoToolMockProvider` (2 tools). The swap is effective next turn.
//   4. Turn 2 ("follow up", also plain text) runs; the crash is injected
//      here. With spec_b active, turn 2's `PromptComposed.surface_size`
//      reflects the 2-tool surface (vs. 1 for turn 1) — the observable
//      footprint of the mid-session swap.
//   5. Phase 2 resumes in a FRESH Runtime sharing the same on-disk store
//      via `open_session_with(id, Resume, spec_b)` — the caller re-supplies
//      the CURRENT spec, exactly as a SaaS server would after reloading a
//      persisted per-session config.
//
// Why both turns are plain text: per the file-level narrowing, only
// boundaries that resolve to `ResumeFromModelCompleted` are exercisable
// (the `ResumeFromToolDispatching` ordering bug and the
// `RestartCurrentTurn` -> `FreshTurn` downgrade are out of scope). A
// plain-text turn's only `ModelCallCompleted` carries `stop_reason=EndTurn`,
// so the crash boundary chosen for turn 2 (its `ModelCallCompleted`)
// resolves to `ResumeFromModelCompleted`. The provider swap changes the
// tool *surface* (observable in `PromptComposed.surface_size`), but the
// turn itself completes via text, keeping us on the supported resume lane.
//
// Oracles: the same 4 chaos oracles (prefix-immutable, terminal-equivalent,
// tool-mapping-equivalent, final-text-equivalent) plus a dedicated
// surface-size check proving the swap survived the resume: turn 2's
// `PromptComposed.surface_size == 2` on BOTH golden and resumed runs.

/// Build the two-turn, plain-text scripted model used by this scenario.
/// Three logical model calls would be wrong — there is exactly one model
/// call per turn (no tool round-trips), so two scripts total. Matchers are
/// first-match-wins:
///
/// 1. `LastUserTextContains("follow up")` -> turn 2 reply (plain text).
/// 2. `LastUserTextContains("say hi")`    -> turn 1 reply (plain text).
fn build_spec_swap_model() -> ScriptedMockModel {
    use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};
    let turn1 = vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "Hi.".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "Hi.".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 2,
            },
        },
    ];
    let turn2 = vec![
        ModelEvent::TextDelta {
            block_index: 0,
            chunk: "Following up.".into(),
        },
        ModelEvent::TextBlockCompleted {
            block_index: 0,
            text: "Following up.".into(),
        },
        ModelEvent::MessageCompleted {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 7,
                output_tokens: 3,
            },
        },
    ];
    ScriptedMockModel::new(vec![
        (
            InputMatcher::LastUserTextContains("follow up".into()),
            OutputScript { events: turn2 },
        ),
        (
            InputMatcher::LastUserTextContains("say hi".into()),
            OutputScript { events: turn1 },
        ),
    ])
}

/// `spec_a`: the build-time default surface (single `read_file` tool). The
/// `tools` override is `Some(MockToolProvider)` to make the contrast with
/// `spec_b` explicit and self-documenting (it is functionally identical to
/// the Runtime default, but we want the open to flow through the spec path).
fn spec_a() -> SessionSpec {
    SessionSpec {
        tools: Some(Arc::new(MockToolProvider) as Arc<dyn ToolProvider>),
        ..Default::default()
    }
}

/// `spec_b`: the post-swap surface. `TwoToolMockProvider` advertises BOTH
/// `read_file` and `write_file`, so turn 2's `PromptComposed.surface_size`
/// is 2 (vs. 1 for turn 1 under `spec_a`). This is the observable footprint
/// of the mid-session `update_session` swap.
fn spec_b() -> SessionSpec {
    SessionSpec {
        tools: Some(Arc::new(TwoToolMockProvider) as Arc<dyn ToolProvider>),
        ..Default::default()
    }
}

/// Wire a `Runtime` for the spec-swap scenario. The build-time tool
/// provider is the single-tool `MockToolProvider`; per-session specs swap
/// it. The model is the two-turn plain-text script.
fn build_spec_swap_runtime(store: Arc<dyn ConversationStore>) -> Arc<Runtime> {
    let mock = Arc::new(build_spec_swap_model());
    let tools = Arc::new(MockToolProvider);
    Runtime::builder()
        .store(store)
        .model(mock as Arc<dyn ModelGateway>)
        .tools(tools as Arc<dyn ToolProvider>)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .build()
        .expect("runtime builds")
}

/// Collect the `PromptComposed.surface_size` for each turn, in order. With
/// the two-turn plain-text flow there is exactly one `PromptComposed` per
/// turn, so `[turn1_size, turn2_size]`.
fn surface_sizes(events: &[ConversationEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::PromptComposed { surface_size, .. } => Some(*surface_size),
            _ => None,
        })
        .collect()
}

/// Oracle: the mid-session swap is observable and survives resume. Turn 1
/// composed its prompt under `spec_a` (1 tool); turn 2 under `spec_b` (2
/// tools). The full sequence must therefore be `[1, 2, ...]` with the
/// LAST entry (turn 2) equal to 2.
fn assert_spec_swap_observed(events: &[ConversationEvent]) {
    let sizes = surface_sizes(events);
    assert!(
        sizes.len() >= 2,
        "expected at least two PromptComposed events (one per turn); saw {sizes:?}"
    );
    assert_eq!(
        sizes[0], 1,
        "turn 1 should compose under spec_a (1 tool); saw {sizes:?}"
    );
    assert_eq!(
        *sizes.last().expect("non-empty"),
        2,
        "turn 2 should compose under spec_b (2 tools) after update_session; saw {sizes:?}"
    );
}

/// Drive both turns to completion with the mid-turn `update_session`
/// swap, no faults. Returns the full golden log.
async fn spec_swap_run_to_completion() -> GoldenRun {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let runtime = build_spec_swap_runtime(Arc::clone(&store));

    let session_id = SessionId::new();
    // Open with spec_a (single-tool surface).
    let handle = runtime
        .open_session_with(session_id, OpenMode::New, spec_a())
        .await
        .expect("open New with spec_a");
    let _events_rx = handle.subscribe();

    // Turn 1: plain text under spec_a.
    handle
        .submit_user_text("say hi")
        .await
        .expect("submit turn 1");
    wait_for_turn_completed(&handle).await;

    // Swap providers between turns. Effective at the next turn boundary.
    handle
        .update_session(spec_b())
        .await
        .expect("update_session spec_b");

    // Turn 2: plain text under spec_b (2-tool surface).
    handle
        .submit_user_text("follow up")
        .await
        .expect("submit turn 2");
    wait_for_turn_completed(&handle).await;

    let events = read_log(store.as_ref(), session_id).await;
    let terminal = terminal_payload(&events).clone();

    let out = handle
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
    assert!(
        matches!(out, ShutdownOutcome::Clean { .. }),
        "expected Clean shutdown, got {out:?}"
    );
    drop(tmp);
    GoldenRun { events, terminal }
}

/// Y-path crash + resume runner for the spec-swap scenario. Phase 1 runs
/// turn 1, swaps to `spec_b`, then runs turn 2 with a `PanicAt(crash_after_n)`
/// fault armed so the crash lands inside turn 2's appends. Phase 2 resumes
/// in a fresh Runtime sharing the same on-disk store, re-supplying `spec_b`
/// on `open_session_with(id, Resume, spec_b)`.
///
/// `turn1_total` is the golden turn-1 prefix length (through the first
/// `TurnCompleted`); the fault is only armed for boundaries past it, so the
/// crash always lands in turn 2.
async fn spec_swap_run_with_y_fault(
    crash_after_n: u64,
    turn1_total: u64,
) -> Vec<ConversationEvent> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::new();

    // ----- Phase 1: turn 1, swap, then turn 2 (crash mid-turn-2). -----
    let inner1: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let wrapper1: Arc<FaultInjectingStore<JsonlStore>> =
        Arc::new(FaultInjectingStore::new(Arc::clone(&inner1)));
    let store1: Arc<dyn ConversationStore> = Arc::clone(&wrapper1) as Arc<dyn ConversationStore>;
    let runtime1 = build_spec_swap_runtime(store1);
    let handle1 = runtime1
        .open_session_with(session_id, OpenMode::New, spec_a())
        .await
        .expect("open New with spec_a");

    // Turn 1 runs cleanly (no fault armed yet). Drive it to terminal on
    // disk before swapping providers.
    handle1
        .submit_user_text("say hi")
        .await
        .expect("submit turn 1");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if log_has_terminal(inner1.as_ref(), session_id).await {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Allow the actor's on_turn_complete hook to release the in-flight guard.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Swap to spec_b, then arm the fault for turn 2. `crash_after_n` is
    // 1-indexed against turn-event seq; the trigger fires after
    // `crash_after_n + 1` total appends (the +1 accounts for the seq=0
    // SessionStarted already written at open).
    debug_assert!(
        crash_after_n > turn1_total,
        "spec-swap fault must target turn 2"
    );
    let _ = turn1_total;
    handle1
        .update_session(spec_b())
        .await
        .expect("update_session spec_b");
    wrapper1
        .set_trigger(FaultTrigger::PanicAt {
            event_no: crash_after_n + 1,
            message: "spec-swap chaos fault",
        })
        .await;

    handle1
        .submit_user_text("follow up")
        .await
        .expect("submit turn 2");

    // Spin until the panic fires (write count passes the boundary) or
    // turn 2 reaches a terminal (degenerate: resumed log == golden).
    let deadline2 = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = read_log(inner1.as_ref(), session_id).await;
        let turns_started = events
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
            .count();
        let terminal_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e.payload,
                    EventPayload::TurnCompleted { .. }
                        | EventPayload::TurnFailed { .. }
                        | EventPayload::TurnPaused { .. }
                )
            })
            .count();
        // Both turns done = >= 2 TurnStarted and >= 2 terminals (one terminal
        // per turn since ISSUE#69 part 2 made TurnCompleted single-emit).
        let both_done = turns_started >= 2 && terminal_count >= 2;
        if both_done || wrapper1.written_count() > crash_after_n {
            break;
        }
        if tokio::time::Instant::now() >= deadline2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    drop(handle1);
    drop(runtime1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ----- Phase 2: fresh Runtime, same store, Resume with spec_b. -----
    let inner2: Arc<JsonlStore> = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));
    let store2: Arc<dyn ConversationStore> = inner2.clone();
    let runtime2 = build_spec_swap_runtime(Arc::clone(&store2));

    // Caller re-supplies the CURRENT (mutated) spec on resume — exactly the
    // SaaS reload-then-resume path ADR-0028 Task 4 is proving.
    let handle2 = runtime2
        .open_session_with(session_id, OpenMode::Resume, spec_b())
        .await
        .expect("open Resume with spec_b");

    wait_for_terminal_with_store(&handle2, store2.as_ref(), session_id).await;

    // If turn 2 had not begun on disk (crash landed before its TurnStarted),
    // re-submit "follow up" so the resumed run observes the same two-turn
    // flow as the golden.
    let mid_events = read_log(store2.as_ref(), session_id).await;
    let turns_started = mid_events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
        .count();
    if turns_started < 2 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = handle2.submit_user_text("follow up").await;
        let deadline3 = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let events = read_log(store2.as_ref(), session_id).await;
            let ts = events
                .iter()
                .filter(|e| matches!(e.payload, EventPayload::TurnStarted { .. }))
                .count();
            let tc = events
                .iter()
                .filter(|e| {
                    matches!(
                        e.payload,
                        EventPayload::TurnCompleted { .. }
                            | EventPayload::TurnFailed { .. }
                            | EventPayload::TurnPaused { .. }
                    )
                })
                .count();
            // One terminal per turn since ISSUE#69 part 2 (single-emit).
            if ts >= 2 && tc >= 2 {
                break;
            }
            if tokio::time::Instant::now() >= deadline3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    let events = read_log(store2.as_ref(), session_id).await;
    let _ = handle2.shutdown(Duration::from_secs(5)).await;
    drop(tmp);
    events
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_spec_mutated_then_resume() {
    let golden = spec_swap_run_to_completion().await;
    let total = golden.events.len() as u64;
    let turn1_total = turn1_event_count(&golden.events);

    // Turn 2's crash boundary: the LAST `ModelCallCompleted` in the log.
    // Since both turns are plain text, every `ModelCallCompleted` carries
    // `stop_reason=EndTurn`, so this boundary resolves to
    // `ResumeFromModelCompleted` (the only supported lane).
    let boundaries = resumable_boundaries(&golden.events);
    eprintln!(
        "scenario=session_spec_mutated_then_resume golden_events={total} \
         turn1_total={turn1_total} resumable_boundaries={boundaries:?}"
    );
    let turn2_boundary = boundaries
        .iter()
        .copied()
        .filter(|&b| b > turn1_total)
        .max()
        .expect("turn 2 must contribute a ResumeFromModelCompleted boundary");
    // turn2_boundary is turn 2's terminal (EndTurn) ModelCallCompleted position.
    // The fault store writes-then-panics, so crashing here lands just after turn
    // 2's TurnCompleted is persisted; resume must reopen the complete log and
    // preserve the swapped-spec footprint without corrupting or duplicating the
    // terminal. Since ISSUE#69 part 2 made TurnCompleted single-emit, exactly one
    // TurnCompleted trails the model call, so this boundary sits at total-1 (the
    // duplicate that used to add slack is gone). Reject only boundary == total
    // (a crash after the very last event, where resume would be a no-op).
    assert!(
        turn2_boundary < total,
        "turn 2 boundary {turn2_boundary} is at/after the log end; \
         no resume work would happen (golden_len={total})"
    );

    // Golden already exercises the swap; assert before crashing.
    assert_spec_swap_observed(&golden.events);

    let resumed = spec_swap_run_with_y_fault(turn2_boundary, turn1_total).await;
    eprintln!(
        "  spec-swap chaos: crash_after_n={turn2_boundary} resumed_len={} (golden_len={total})",
        resumed.len()
    );

    assert_context_managed_pairing(&resumed);
    assert_prefix_immutable(&golden.events, &resumed, turn2_boundary);
    assert_terminal_equivalent(&golden.terminal, terminal_payload(&resumed));
    assert_tool_mapping_equivalent(&golden.events, &resumed);
    assert_final_text_equivalent(&golden.events, &resumed);

    // The crux: the resumed log carries the SAME mutated-surface footprint
    // as the golden — turn 2 still composed under spec_b's 2-tool surface
    // even though the resume re-supplied the spec into a fresh Runtime.
    assert_spec_swap_observed(&resumed);
}

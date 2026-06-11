//! Regression test for the `cancel-token-disconnect` bug.
//!
//! Before the fix, `SessionShared.current_cancel_token` was a sibling clone of
//! the *initial* cancel token, while `SessionState.current_cancel_token` was an
//! `Arc<Mutex<...>>` whose inner token got swapped on every `spawn_turn_driver`.
//! That made `SessionHandle::cancel_turn` reach only the first turn; every
//! subsequent call was a silent no-op because it fired the original sibling
//! and the live turn was waiting on the actor-side replacement.
//!
//! The fix shares ONE `Arc<parking_lot::Mutex<CancellationToken>>` between
//! `SessionState` and `SessionShared`, so mutations on the actor side are
//! visible to the handle.
//!
//! Shape of the test:
//! 1. Open a session with a custom `ModelGateway` that completes turn 1 fast
//!    and parks turn 2 indefinitely until `ctx.cancel` fires.
//! 2. Submit turn 1, wait for `TurnCompleted`.
//! 3. Submit turn 2, wait briefly for it to start.
//! 4. Call `cancel_turn`. The handle's cancel must reach the gateway's
//!    `ctx.cancel`; the gateway then yields `ModelError::Cancelled`, the
//!    turn driver transitions to `Failed`, and the session emits
//!    `TurnFailed`.
//! 5. Assert `TurnFailed` arrives within 500ms.
//!
//! Before the fix: step 5 times out (the cancel never reaches the gateway).
//! After the fix: step 5 passes well under the timeout.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{
    ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits, StopReason, Usage,
};
use cogito_protocol::ids::SessionId;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;
use futures::stream::{self, BoxStream};

/// Gateway: turn 1 completes promptly; turn 2 parks until cancel fires.
#[derive(Debug, Default)]
struct CancelTestGateway {
    call_count: AtomicUsize,
}

#[async_trait]
impl ModelGateway for CancelTestGateway {
    async fn stream(
        &self,
        _input: ModelInput,
        ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            // Turn 1: emit a fast clean reply.
            let events = vec![
                Ok(ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "ack".into(),
                }),
                Ok(ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "ack".into(),
                }),
                Ok(ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                }),
            ];
            Ok(stream::iter(events).boxed())
        } else {
            // Turn 2+: park until ctx.cancel fires, then yield Cancelled.
            // The actor-side cancel token has been swapped between turns;
            // this is exactly what the regression test exercises.
            let cancel = ctx.cancel.clone();
            let s = async_stream::stream! {
                cancel.cancelled().await;
                yield Err(ModelError::Cancelled);
            };
            Ok(s.boxed())
        }
    }

    fn provider_id(&self) -> &'static str {
        "cancel-test-mock"
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits::new("cancel-test-mock", 32_768)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_turn_works_after_first_turn() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let gateway = Arc::new(CancelTestGateway::default());

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    let runtime = Runtime::builder()
        .store(store)
        .model(gateway)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("cancel-test-mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;

    let mut events = handle.subscribe();

    // Turn 1: drive a fast completion, then wait for its single
    // `TurnCompleted` broadcast (one per turn since ISSUE#69 part 2 was
    // fixed — the TurnDriver's FSM transition is the sole emitter). The
    // back-to-back turn-2 submit below is safe even if the actor has not
    // yet run `on_turn_complete`: the session actor is single-threaded and
    // a trigger arriving while `in_flight` is still set is parked in the
    // single-slot `pending_user_input` queue and drained when the turn
    // retires, so it is never dropped.
    handle.submit_user_text("first").await?;
    let saw_complete = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_complete, "turn 1 did not complete within 5s");

    // Turn 2: gateway parks until ctx.cancel fires. Submit, then wait for
    // `TurnStarted` to be broadcast so we know the gateway has actually
    // entered its parked stream; only then is cancel meaningful.
    handle.submit_user_text("second").await?;
    let saw_turn2_started = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnStarted { .. }) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_turn2_started,
        "turn 2 did not emit TurnStarted within 2s"
    );

    // Give the TurnDriver a moment to invoke `gateway.stream()` so the
    // gateway's `cancel.cancelled().await` is actually parked.
    tokio::time::sleep(Duration::from_millis(50)).await;

    handle.cancel_turn().await?;

    // The fix makes cancel_turn fire the *current* per-turn token, which the
    // gateway's stream() is parked on. We should observe a terminal event
    // (TurnFailed via ModelError::Cancelled) for turn 2 well under 500ms.
    let got_terminal = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnFailed { .. } | StreamEvent::TurnCancelled) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        got_terminal,
        "turn 2 did not observe cancel within 500ms — cancel_turn must reach the per-turn token"
    );

    handle.shutdown(Duration::from_secs(2)).await?;
    Ok(())
}

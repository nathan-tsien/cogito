//! Integration test for the single-slot mid-turn user-input queue
//! (Sprint 8 Task 9 / spec §8.4).
//!
//! Shape:
//! 1. Open a session with a custom `ModelGateway` whose turn 1 parks on a
//!    `Notify` until the test fires it; turn 2 returns a fast clean reply.
//! 2. Submit `"first"`, wait for turn 1's `TurnStarted` so we know the
//!    gateway is parked and the actor is in `InFlight::Active`.
//! 3. Submit `"second"` and then `"third"` while turn 1 is still in flight.
//!    The actor must hold them in `pending_user_input` (latest-wins) rather
//!    than start a second turn or drop them.
//! 4. Fire the `Notify` so turn 1 drains. The actor's `on_turn_complete`
//!    must observe `in_flight == None`, take the queued trigger, and start
//!    turn 2 with `user_input == [Text("third")]` — proving single-slot,
//!    latest-wins semantics.
//! 5. Wait for turn 2's terminal event and assert the persisted log shows
//!    exactly two `TurnStarted` events whose `user_input` projects as
//!    `[Text("first")]` and `[Text("third")]` in that order — `"second"`
//!    was overwritten and never reached the log.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cogito_core::runtime::{OpenMode, Runtime};
use cogito_protocol::ExecCtx;
use cogito_protocol::content::ContentBlock;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{
    ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits, StopReason, Usage,
};
use cogito_protocol::ids::SessionId;
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_store_jsonl::JsonlStore;
use cogito_tools::{BuiltinToolProvider, ReadFile};
use futures::StreamExt as _;
use futures::stream::{self, BoxStream};
use tokio::sync::Notify;

/// Gateway whose turn 1 parks on `release` until the test fires it; every
/// subsequent turn returns a fast clean reply.
#[derive(Debug)]
struct QueueTestGateway {
    call_count: AtomicUsize,
    release: Arc<Notify>,
}

#[async_trait]
impl ModelGateway for QueueTestGateway {
    async fn stream(
        &self,
        _input: ModelInput,
        _ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            // Turn 1: park until the test signals, then emit a clean reply.
            // The actor stays in `InFlight::Active` for as long as this
            // stream is pending, which is the window during which the test
            // submits additional triggers that must land in the queue.
            let release = Arc::clone(&self.release);
            let s = async_stream::stream! {
                release.notified().await;
                yield Ok(ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "one".into(),
                });
                yield Ok(ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "one".into(),
                });
                yield Ok(ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage { input_tokens: 1, output_tokens: 1 },
                });
            };
            Ok(s.boxed())
        } else {
            // Turn 2+: fast clean reply so the drained queue produces a
            // terminal TurnCompleted the test can observe.
            let events = vec![
                Ok(ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "two".into(),
                }),
                Ok(ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "two".into(),
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
        }
    }

    fn provider_id(&self) -> &'static str {
        "queue-test-mock"
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits::new("queue-test-mock", 32_768)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_input_queued_during_active_turn_drained_after()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let release = Arc::new(Notify::new());
    let gateway = Arc::new(QueueTestGateway {
        call_count: AtomicUsize::new(0),
        release: Arc::clone(&release),
    });

    let tools = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .build(),
    );

    // Cloned Arc for read-back assertions after shutdown.
    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(gateway)
        .tools(tools)
        .strategy(HarnessStrategy::default_with_model("queue-test-mock"))
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();

    // Submit turn 1 and wait for it to enter the gateway so the actor is
    // firmly in `InFlight::Active` before we enqueue further triggers.
    handle.submit_user_text("first").await?;
    let saw_started = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnStarted) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_started, "turn 1 did not emit TurnStarted within 2s");

    // While turn 1 is parked, push two more triggers. The first lands in
    // the slot; the second overwrites it (latest-wins) and produces a
    // `tracing::warn!`. Neither writes to the event log yet.
    handle.submit_user_text("second").await?;
    handle.submit_user_text("third").await?;

    // Release turn 1. The actor's on_turn_complete must drain the queue
    // and start turn 2 with the LATEST trigger ("third").
    release.notify_one();

    // Drain stream events until we see turn 2's TurnStarted (turn 1's
    // TurnStarted was already consumed above). Observing a second
    // TurnStarted on the broadcast proves the actor drained the queue
    // and called `try_start_turn` from `on_turn_complete`.
    let saw_turn_2_started = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnStarted) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_turn_2_started,
        "turn 2 (from the drained queue) did not emit TurnStarted within 5s — \
         single-slot drain in on_turn_complete may be broken"
    );

    handle.shutdown(Duration::from_secs(5)).await?;

    // Inspect the persisted log: there must be exactly two TurnStarted
    // events with user_input `[Text("first")]` and `[Text("third")]` — the
    // intermediate `"second"` was overwritten and never materialized as a
    // turn (single-slot, latest-wins).
    let persisted: Vec<_> = {
        let mut stream = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = stream.next().await {
            out.push(evt?);
        }
        out
    };
    let turn_starts: Vec<&Vec<ContentBlock>> = persisted
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TurnStarted { user_input, .. } => Some(user_input),
            _ => None,
        })
        .collect();
    assert_eq!(
        turn_starts.len(),
        2,
        "expected exactly two TurnStarted events (one per drained trigger); got {}",
        turn_starts.len()
    );
    assert_eq!(
        turn_starts[0],
        &vec![ContentBlock::Text {
            text: "first".into()
        }],
        "turn 1 user_input must project from the original trigger"
    );
    assert_eq!(
        turn_starts[1],
        &vec![ContentBlock::Text {
            text: "third".into()
        }],
        "turn 2 user_input must be the LATEST queued trigger (\"third\"), \
         not the intermediate \"second\" — single-slot, latest-wins"
    );

    Ok(())
}

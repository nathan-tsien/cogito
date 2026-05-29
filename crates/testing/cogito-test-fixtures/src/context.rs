//! Test helpers for Context-Management trait implementations.
//!
//! Provides:
//! - `InMemoryRecorder` — a minimal `EventRecorder` that stores payloads in memory
//! - `DummyGateway` — a `ModelGateway` that always returns an error (sufficient for tests
//!   that never call the gateway, such as `NoneCompactor`)

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::event::EventPayload;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits};
use cogito_protocol::ids::{EventId, TurnId};
use cogito_protocol::store::{EventRecorder, StoreError};
use futures::stream::BoxStream;

/// Minimal in-memory `EventRecorder` for unit tests.
///
/// Appended payloads are stored in `events` as `(TurnId, EventPayload)` pairs.
/// A monotonic seq counter is maintained for parity with the real recorder.
#[derive(Debug, Default)]
pub struct InMemoryRecorder {
    /// All payloads written through this recorder in append order.
    pub events: Vec<(TurnId, EventPayload)>,
    seq: u64,
}

#[async_trait]
impl EventRecorder for InMemoryRecorder {
    async fn append_payload(
        &mut self,
        turn_id: TurnId,
        payload: EventPayload,
    ) -> Result<(EventId, u64), StoreError> {
        let event_id = EventId::new();
        let seq = self.seq;
        self.seq += 1;
        self.events.push((turn_id, payload));
        Ok((event_id, seq))
    }
}

/// Stub `ModelGateway` that always fails. Suitable for tests of `Compactor`
/// implementations that do not perform model calls (e.g. `NoneCompactor`).
pub struct DummyGateway;

#[async_trait::async_trait]
impl ModelGateway for DummyGateway {
    async fn stream(
        &self,
        _input: ModelInput,
        _ctx: ExecCtx,
    ) -> Result<BoxStream<'static, Result<ModelEvent, ModelError>>, ModelError> {
        Err(ModelError::Network(
            "DummyGateway does not support model calls".into(),
        ))
    }

    fn provider_id(&self) -> &'static str {
        "dummy"
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits::new("dummy", 32_768)
    }
}

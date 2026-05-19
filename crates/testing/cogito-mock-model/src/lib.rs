//! `MockModelGateway` — a scripted `ModelGateway` for testing.
//!
//! Tests pre-load one or more `MockScript`s; each `stream()` call pops the
//! next script and emits its events (or returns its error).

#![warn(clippy::pedantic)]

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::gateway::{ModelError, ModelEvent, ModelGateway, ModelInput};
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
}

impl MockModelGateway {
    /// Construct empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
}

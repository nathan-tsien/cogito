//! Internal helpers for mapping reqwest / network failures into
//! `ModelError`. Public to `crate::*` only.

use cogito_protocol::gateway::ModelError;

/// Map a reqwest transport error to a `ModelError`.
pub(crate) fn from_reqwest(e: &reqwest::Error) -> ModelError {
    if e.is_timeout() {
        ModelError::Network("timeout".into())
    } else if e.is_connect() {
        ModelError::Network(format!("connect: {e}"))
    } else {
        ModelError::Network(e.to_string())
    }
}

pub(crate) mod wire {
    use cogito_protocol::gateway::ModelError;

    /// Wrap a decode-failure message in `ModelError::Decode`.
    pub(crate) fn decode(message: impl Into<String>) -> ModelError {
        ModelError::Decode(message.into())
    }
}

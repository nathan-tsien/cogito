//! `store_writer`: the actor's persistence subtask. Consumes
//! `PersistCommand`s, batches `text_delta` events (200ms or 500 chars),
//! and calls `ConversationStore::append` with fsync per event. See
//! spec §8 for the full state machine.
//!
//! Implementation is Plan 2 (Sprint 1).

use tokio::sync::{mpsc, oneshot};

/// Commands the actor (or `TurnDriver` via `persist_tx`) sends to the
/// store writer subtask.
///
/// Crate-private: this is the internal wire between the actor task
/// and the store writer subtask, not a public API. External callers
/// observe persistence indirectly through the `ConversationStore`
/// trait (defined in `cogito-protocol` per ADR-0004 §3).
#[derive(Debug)]
#[allow(dead_code)] // Plan 2 fills in the producers
pub(crate) enum PersistCommand {
    /// Append one event. If `ack` is `Some`, the writer signals completion
    /// (after fsync) by sending on the oneshot.
    Append {
        /// Opaque event payload. Plan 2 replaces `serde_json::Value`
        /// with the concrete `ConversationEvent` type once that lands
        /// in `cogito-protocol`.
        event: serde_json::Value,
        /// Optional acknowledgement channel; if `Some`, the writer sends
        /// back the append result after the fsync completes.
        ack: Option<oneshot::Sender<Result<(), StoreWriteError>>>,
    },
    /// Force-flush the text-delta buffer immediately.
    Flush {
        /// Acknowledgement channel; the writer signals completion after
        /// the flush fsync.
        ack: oneshot::Sender<Result<(), StoreWriteError>>,
    },
}

/// Errors from the store writer subtask. Surfaced to the caller via the
/// `ack` oneshots above.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Plan 2 fills in the error sites
pub(crate) enum StoreWriteError {
    /// File / network I/O failure when appending.
    #[error("store I/O failed: {0}")]
    Io(String),
    /// Failure while flushing the text-delta buffer (e.g., during
    /// shutdown).
    #[error("buffer flush failed: {0}")]
    Flush(String),
}

/// Plan 2 entry point. The subtask runs as a tokio task spawned from
/// `SessionActor::start`, owns the `ConversationStore` handle, and
/// exits when the `PersistCommand` channel closes.
#[allow(dead_code)] // Plan 2 fills in the spawner
#[allow(clippy::unused_async)] // Plan 2 (Sprint 1) replaces todo!() with real await points
#[allow(clippy::todo)] // intentional stub — Plan 2 fills in the body
pub(super) async fn store_writer_main(_rx: mpsc::Receiver<PersistCommand>) {
    todo!(
        "Plan 2 (Sprint 1): select on _rx.recv() + 200ms tick; \
         apply flush rules per spec §8 (force-flush before non-delta \
         events; batch text deltas; per-event fsync via spawn_blocking)"
    )
}

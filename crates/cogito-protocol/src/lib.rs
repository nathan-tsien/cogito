//! cogito-protocol
//!
//! Protocol layer: events, contracts, and types shared across the workspace.
//!
//! This crate is dependency-free with respect to other cogito crates.
//! Anything that defines the *contract* between components belongs here.
//!
//! Module map (1:1 with the Brain/Hands/Session boundaries in ADR-0004):
//! - [`content`]: `ContentBlock` — wire-format unit shared between model, tools, persisted events
//! - [`error`]: shared error kinds and helpers
//! - [`event`]: `ConversationEvent` + `EventPayload` + `SCHEMA_VERSION` (persisted event log)
//! - [`exec_ctx`]: `ExecCtx` — per-invocation context handed to every tool and hook
//! - [`gateway`]: `ModelGateway` trait + value types (`ModelInput`, `ModelOutput`, `ModelEvent`, …)
//! - [`hook`]: `HookHandler`, `HookProvider`, `HookDecision`, `HookLifecyclePoint` — H09 policy gate
//! - [`ids`]: strongly-typed ULID newtypes (`EventId`, `SessionId`, `TurnId`)
//! - [`job`]: `JobManager` trait, `JobId`, `JobStatus`, `JobCompletionEvent`
//! - [`metrics`]: `MetricsRecorder` trait + `NoOpMetricsRecorder` — pluggable metrics sink (Sprint 5)
//! - [`session`]: `SessionMeta` — per-session pass-through metadata
//! - [`store`]: `ConversationStore` trait + `StoreError` (persisted event log backend contract)
//! - [`context`]: `ContextError` — failure modes for the four Context-Management traits (H11)
//! - [`strategy`]: `HarnessStrategy`, `ToolFilter`
//! - [`stream`]: `StreamEvent` enum (real-time fanout to subscribers)
//! - [`tool`]: `ToolProvider` trait, `ToolDescriptor`, `InvokeOutcome`, `ExecutionClass`
//! - [`turn`]: `TurnOutcome`, `TurnFailureReason`
//! - [`turn_trigger`]: `TurnTrigger` — what caused a new turn to start (ADR-0016)
//!
//! All v0.1 contract modules ship as part of Sprint 0 (Tasks 7-10 of
//! the Sprint 0 closure plan) and Sprint 1 (`event`, `store`).

pub mod content;
pub mod context;
pub mod error;
pub mod event;
pub mod exec_ctx;
pub mod gateway;
pub mod hook;
pub mod ids;
pub mod job;
pub mod metrics;
pub mod session;
pub mod store;
pub mod strategy;
pub mod stream;
pub mod tool;
pub mod turn;
pub mod turn_trigger;

pub use content::ContentBlock;
pub use context::{
    CompactionReplacement, ContextDecisionErrors, ContextError, ToolFilterOverrideMode,
};
pub use event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
pub use exec_ctx::ExecCtx;
pub use gateway::{
    Message, ModelError, ModelEvent, ModelGateway, ModelInput, ModelLimits, ModelOutput,
    ModelParams, StopReason, Usage,
};
pub use hook::{HookDecision, HookHandler, HookLifecyclePoint, HookProvider};
pub use ids::{EventId, SessionId, TurnId};
pub use metrics::{MetricsRecorder, NoOpMetricsRecorder};
pub use session::SessionMeta;
pub use store::{ConversationStore, StoreError};
pub use strategy::{HarnessStrategy, ToolFilter};
pub use tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
pub use turn_trigger::TurnTrigger;

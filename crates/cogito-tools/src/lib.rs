//! cogito-tools — builtin `ToolProvider` implementations + composition utility.
//!
//! Brain never imports this crate directly (per ADR-0004 layer rule); the
//! consumer wires concrete providers into Runtime as `Arc<dyn ToolProvider>`.

#![warn(clippy::pedantic)]

pub mod builtins;
pub mod composite;
pub mod provider;
pub mod workspace;

pub use builtins::{ReadFile, WebFetch, WebFetchConfig};
pub use composite::{CompositeToolProvider, NamingPolicy};
pub use provider::{BuiltinTool, BuiltinToolProvider, BuiltinToolProviderBuilder};
pub use workspace::LocalWorkspace;

//! Brain-facing `ToolProvider` implementation that holds a fixed set of
//! builtin tools constructed at process startup.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult};

/// One builtin tool exposed via `BuiltinToolProvider`. Concrete tools live
/// in `crate::builtins::*`.
#[async_trait]
pub trait BuiltinTool: Send + Sync {
    /// Stable metadata. Constructed lazily and cached by the provider.
    fn descriptor(&self) -> ToolDescriptor;

    /// Execute the tool. Implementations must NEVER panic — turn unrecoverable
    /// failures into `ToolResult::Error { kind: InvocationFailed, ... }`.
    async fn invoke(&self, args: serde_json::Value, ctx: ExecCtx) -> ToolResult;
}

/// A `ToolProvider` that wraps a fixed set of builtin tools.
///
/// Construct via the `builder()` -> `with_tool()` -> `build()` pattern so
/// the descriptor cache is computed once.
pub struct BuiltinToolProvider {
    tools: HashMap<String, Arc<dyn BuiltinTool>>,
    descriptors: Vec<ToolDescriptor>,
}

impl BuiltinToolProvider {
    /// Begin a builder.
    #[must_use]
    pub fn builder() -> BuiltinToolProviderBuilder {
        BuiltinToolProviderBuilder::default()
    }
}

/// Builder for `BuiltinToolProvider`. Order of `with_tool` calls determines
/// the descriptor cache order.
#[derive(Default)]
pub struct BuiltinToolProviderBuilder {
    tools: Vec<Arc<dyn BuiltinTool>>,
}

impl BuiltinToolProviderBuilder {
    /// Register one builtin tool.
    #[must_use]
    pub fn with_tool(mut self, tool: Arc<dyn BuiltinTool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Finalize the provider, building the descriptor cache.
    #[must_use]
    pub fn build(self) -> BuiltinToolProvider {
        let mut tools = HashMap::with_capacity(self.tools.len());
        let mut descriptors = Vec::with_capacity(self.tools.len());
        for t in self.tools {
            let d = t.descriptor();
            descriptors.push(d.clone());
            tools.insert(d.name.clone(), t);
        }
        BuiltinToolProvider { tools, descriptors }
    }
}

#[async_trait]
impl ToolProvider for BuiltinToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        match self.tools.get(name) {
            Some(t) => InvokeOutcome::Sync(t.invoke(args, ctx).await),
            None => InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("unknown tool: {name}"),
                retryable: false,
            }),
        }
    }
}

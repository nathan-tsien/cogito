//! `CompositeToolProvider` — a `ToolProvider` that merges N child
//! providers under a configurable naming policy.

use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::tool::{InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult};

/// How a composite provider handles name conflicts between children.
#[derive(Debug, Clone)]
pub enum NamingPolicy {
    /// First-wins; subsequent providers' duplicate names panic at build time.
    Strict,
    /// Each child's tools are exposed under `prefix[i]/name`; lookup splits
    /// on the first `/`.
    Prefixed(Vec<String>),
}

/// Composite of multiple `ToolProvider`s. Constructed once at startup.
pub struct CompositeToolProvider {
    children: Vec<Arc<dyn ToolProvider>>,
    naming: NamingPolicy,
    descriptors: Vec<ToolDescriptor>,
}

impl CompositeToolProvider {
    /// Build a composite from children.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if `Strict` mode has duplicate names, or if
    /// `Prefixed` has a different number of prefixes than children.
    pub fn new(
        children: Vec<Arc<dyn ToolProvider>>,
        naming: NamingPolicy,
    ) -> Result<Self, String> {
        if let NamingPolicy::Prefixed(ref prefixes) = naming {
            if prefixes.len() != children.len() {
                return Err(format!(
                    "Prefixed naming expects {} prefixes, got {}",
                    children.len(),
                    prefixes.len()
                ));
            }
        }
        let mut descriptors = Vec::new();
        for (i, child) in children.iter().enumerate() {
            for mut d in child.list() {
                d.name = match &naming {
                    NamingPolicy::Strict => d.name,
                    NamingPolicy::Prefixed(prefixes) => format!("{}/{}", prefixes[i], d.name),
                };
                descriptors.push(d);
            }
        }
        if matches!(naming, NamingPolicy::Strict) {
            let mut names: Vec<_> = descriptors.iter().map(|d| d.name.as_str()).collect();
            names.sort_unstable();
            if let Some([w_0, ..]) = names.windows(2).find(|w| w[0] == w[1]) {
                return Err(format!("duplicate tool name under Strict: {w_0}"));
            }
        }
        Ok(Self {
            children,
            naming,
            descriptors,
        })
    }
}

#[async_trait]
impl ToolProvider for CompositeToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        match &self.naming {
            NamingPolicy::Strict => {
                for child in &self.children {
                    if child.list().iter().any(|d| d.name == name) {
                        return child.invoke(name, args, ctx).await;
                    }
                }
            }
            NamingPolicy::Prefixed(prefixes) => {
                let Some((prefix, rest)) = name.split_once('/') else {
                    return InvokeOutcome::Sync(ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: format!("composite expects prefix/name, got {name}"),
                        retryable: false,
                    });
                };
                if let Some(idx) = prefixes.iter().position(|p| p == prefix) {
                    return self.children[idx].invoke(rest, args, ctx).await;
                }
            }
        }
        InvokeOutcome::Sync(ToolResult::Error {
            kind: ToolErrorKind::InvocationFailed,
            message: format!("unknown tool: {name}"),
            retryable: false,
        })
    }
}

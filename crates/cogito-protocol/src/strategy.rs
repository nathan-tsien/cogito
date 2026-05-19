//! `HarnessStrategy` — per-turn behavior knobs read by H10/H04/H05/H09.
//!
//! v0.1 Sprint 2 exposes a factory (`default_with_model`); v0.x Sprint 5
//! adds a YAML-backed registry. The Mid field set is documented in
//! `docs/components/H10-strategy-selector.md` §"v0.1 Sprint 2 scope".

use serde::{Deserialize, Serialize};

use crate::gateway::ModelParams;

/// Tool filter applied by H05 Tool Surface Builder. `Allow` is an explicit
/// whitelist; `All` admits every tool the provider exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolFilter {
    /// Wildcard: every tool the `ToolProvider` lists is admitted.
    All,
    /// Only tools whose name appears in this list are admitted.
    /// Names not present in the provider catalog are silently dropped.
    Allow(Vec<String>),
}

/// Per-turn behavior knobs. v0.1 Sprint 2 Mid field set.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HarnessStrategy {
    /// Identifier written into `EventPayload::TurnStarted { strategy_id }`.
    pub name: String,
    /// System prompt prepended to every `ModelInput` from this strategy.
    pub system_prompt: String,
    /// Which tools are exposed to the model this turn.
    pub allowed_tools: ToolFilter,
    /// Optional explicit tool ordering for prompt-cache stability.
    /// `None` => alphabetical sort by tool name (H05 enforces).
    pub tool_order: Option<Vec<String>>,
    /// Sampling parameters + model id, copied into `ModelInput.params`.
    pub model_params: ModelParams,
    /// Safety budget: maximum number of inner-loop iterations
    /// (Init -> `ToolDispatching` -> Init -> ...) before H01 stops the turn
    /// with `TurnFailureReason::MaxTurnsExceeded`.
    pub max_turns: u32,
}

impl HarnessStrategy {
    /// Convenience factory used by `cogito-cli chat` and tests. Builds a
    /// strategy with sane defaults; caller may further mutate fields.
    #[must_use]
    pub fn default_with_model(model: impl Into<String>) -> Self {
        Self {
            name: "default".into(),
            system_prompt: "You are a helpful assistant.".into(),
            allowed_tools: ToolFilter::All,
            tool_order: None,
            model_params: ModelParams {
                model: model.into(),
                max_tokens: 4096,
                temperature: Some(0.7),
                top_p: None,
                stop_sequences: vec![],
            },
            max_turns: 16,
        }
    }
}

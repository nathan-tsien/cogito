//! cogito-model — `ModelGateway` implementations for external LLM providers.

#![warn(clippy::pedantic)]

mod error;
pub mod sse;

pub mod anthropic;
pub mod openai_compat;
mod provider_config;

pub use anthropic::{AnthropicConfig, AnthropicGateway};
pub use openai_compat::{OpenAiCompatConfig, OpenAiCompatGateway};
pub use provider_config::{ProviderConfig, build_gateway};

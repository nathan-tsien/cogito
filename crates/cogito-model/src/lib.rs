//! cogito-model — `ModelGateway` implementations for external LLM providers.

#![warn(clippy::pedantic)]

mod error;
pub mod sse;

pub mod anthropic;
pub mod openai_compat;

pub use anthropic::{AnthropicConfig, AnthropicGateway};
pub use openai_compat::{OpenAiCompatConfig, OpenAiCompatGateway};

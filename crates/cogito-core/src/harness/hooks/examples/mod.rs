//! Reference `HookHandler` implementations shipped in v0.1.
//!
//! - `sensitive_content` — rejects tool calls whose args contain
//!   well-known secret-shaped strings.
//! - `bash_audit` — records a metric counter for every `bash` tool
//!   invocation. Never rejects.

pub mod bash_audit;
pub mod sensitive_content;

pub use bash_audit::BashAuditHook;
pub use sensitive_content::SensitiveContentHook;

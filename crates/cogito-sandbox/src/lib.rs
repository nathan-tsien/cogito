//! cogito-sandbox
//!
//! Subprocess-based execution sandbox. Provides cwd isolation, resource
//! limits, and timeout enforcement. Not a security boundary — that's a
//! production concern. Goal here is to *behave* like a sandbox so the
//! Harness can be validated against the production contract.

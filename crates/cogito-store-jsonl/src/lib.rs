//! cogito-store-jsonl
//!
//! Per-session JSONL backend for `ConversationStore`. Each session writes to
//! `<root>/sessions/<session_id>.jsonl`; every event is `fsync`'d to disk
//! before the append returns. This is the v0.1 sole `ConversationStore`
//! implementation; future backends (Postgres, HTTP) live in sibling crates.
//!
//! See:
//! - `docs/components/H02-step-recorder.md`
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` §8

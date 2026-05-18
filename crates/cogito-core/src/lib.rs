//! cogito-core
//!
//! Brain (`harness/`) + Runtime layer (`runtime/`). Per ADR-0004 the
//! `harness/` module may import only `cogito-protocol`; the `runtime/`
//! module may import any non-Surface layer (it is the DI shell that
//! wires concrete impls into Brain).
//!
//! See:
//! - `ARCHITECTURE.md`
//! - `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md`

pub mod harness;
pub mod runtime;

//! One file per FSM state's outgoing transition.
//!
//! Each `transit` function MUST call a `step.record_*` method BEFORE
//! returning the next `TurnState` — this is the ADR-0003 / AGENTS.md §1
//! invariant that reviewers check.

pub mod context_managed;
pub mod init;
pub mod model_calling;
pub mod model_completed;
pub mod prompt_built;
pub mod tool_dispatching;

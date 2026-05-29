//! Smoke tests for `ExecCtx` construction and cancellation propagation.

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};

#[test]
fn open_ended_context_is_not_cancelled() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    assert!(!ctx.cancel.is_cancelled());
    assert!(ctx.deadline.is_none());
}

#[test]
fn clone_shares_cancel_token() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    let ctx2 = ctx.clone();
    ctx.cancel.cancel();
    assert!(ctx2.cancel.is_cancelled());
}

#[test]
fn open_ended_defaults_new_fields() {
    let ctx = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    assert_eq!(ctx.subagent_depth, 0);
    assert!(ctx.call_id.is_none());
    assert!(ctx.brain_spawner.is_none());
    // Debug must not panic and must not try to print the spawner internals.
    let s = format!("{ctx:?}");
    assert!(s.contains("ExecCtx"));
}

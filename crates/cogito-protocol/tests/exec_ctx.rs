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

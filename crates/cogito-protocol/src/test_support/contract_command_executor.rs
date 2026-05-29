//! Shared contract every `CommandExecutor` implementation must satisfy.
//! A backend crate (e.g. `cogito-sandbox`) calls these from its own test
//! module against its concrete executor.
//!
//! Marked test-only via the crate's `test-support` feature.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use crate::ExecCtx;
use crate::command::{CommandExecutor, CommandSpec};
use crate::ids::{SessionId, TurnId};

fn spec(command: &str, timeout: Duration) -> CommandSpec {
    CommandSpec {
        command: command.to_string(),
        cwd: None,
        timeout,
        max_output_bytes: 4096,
    }
}

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

/// A successful command returns its stdout and `exit_code == Some(0)`.
pub async fn contract_success(exec: Arc<dyn CommandExecutor>) {
    let out = exec
        .run(spec("echo hello", Duration::from_secs(10)), ctx())
        .await
        .expect("echo should not fail to spawn");
    assert!(out.stdout.contains("hello"), "stdout was {:?}", out.stdout);
    assert_eq!(out.exit_code, Some(0));
    assert!(!out.timed_out);
}

/// A non-zero exit is a normal outcome (NOT an error).
pub async fn contract_nonzero_exit(exec: Arc<dyn CommandExecutor>) {
    let out = exec
        .run(spec("exit 3", Duration::from_secs(10)), ctx())
        .await
        .expect("a command that exits non-zero must not surface as CommandError");
    assert_eq!(out.exit_code, Some(3));
}

/// A command exceeding `timeout` is killed and reports `timed_out`.
pub async fn contract_timeout(exec: Arc<dyn CommandExecutor>) {
    let out = exec
        .run(spec("sleep 30", Duration::from_millis(200)), ctx())
        .await
        .expect("timeout is an outcome, not a CommandError");
    assert!(out.timed_out, "expected timed_out=true, got {out:?}");
}

/// Output beyond `max_output_bytes` is truncated and flagged.
pub async fn contract_truncation(exec: Arc<dyn CommandExecutor>) {
    let mut s = spec(
        "for i in $(seq 1 5000); do echo 0123456789; done",
        Duration::from_secs(20),
    );
    s.max_output_bytes = 256;
    let out = exec.run(s, ctx()).await.expect("spawn ok");
    assert!(out.truncated, "expected truncated=true");
}

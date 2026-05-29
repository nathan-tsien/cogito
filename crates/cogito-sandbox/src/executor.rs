//! `DirectExecutor` — runs commands via `sh -c` on the host. v0.1 only
//! implementation of `CommandExecutor`. Not a security boundary: no
//! namespaces / seccomp / chroot. Mirrors the subprocess race pattern
//! validated by `cogito-jobs::run_tests`.

use std::process::Stdio;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::command::{CommandError, CommandExecutor, CommandOutcome, CommandSpec};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

use crate::config::DirectConfig;
use crate::truncate::head_tail;

/// Host-side, no-isolation executor.
#[derive(Debug, Clone)]
pub struct DirectExecutor {
    cfg: DirectConfig,
}

impl DirectExecutor {
    /// Construct from `DirectConfig`.
    #[must_use]
    pub fn new(cfg: DirectConfig) -> Self {
        Self { cfg }
    }
}

/// Internal terminal state of the `select!` race.
enum Wait {
    /// Child exited (either successfully or `wait` itself returned an error).
    Exited(std::io::Result<std::process::ExitStatus>),
    /// `ExecCtx::cancel` fired; the child was killed.
    Cancelled,
    /// `CommandSpec::timeout` elapsed; the child was killed.
    TimedOut,
}

#[async_trait]
impl CommandExecutor for DirectExecutor {
    async fn run(&self, spec: CommandSpec, ctx: ExecCtx) -> Result<CommandOutcome, CommandError> {
        let cwd = match &spec.cwd {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => self.cfg.root.join(p),
            None => self.cfg.root.clone(),
        };

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&spec.command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !self.cfg.inherit_env {
            cmd.env_clear();
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| CommandError::Spawn(e.to_string()))?;

        let Some(stdout) = child.stdout.take() else {
            return Err(CommandError::Spawn("child stdout pipe missing".into()));
        };
        let Some(stderr) = child.stderr.take() else {
            return Err(CommandError::Spawn("child stderr pipe missing".into()));
        };
        let stdout_task = tokio::spawn(read_to_end(stdout));
        let stderr_task = tokio::spawn(read_to_end(stderr));

        let wait = tokio::select! {
            status = child.wait() => Wait::Exited(status),
            () = ctx.cancel.cancelled() => {
                let _ = child.kill().await;
                Wait::Cancelled
            }
            () = tokio::time::sleep(spec.timeout) => {
                let _ = child.kill().await;
                Wait::TimedOut
            }
        };

        let raw_out = stdout_task.await.unwrap_or_default();
        let raw_err = stderr_task.await.unwrap_or_default();
        let (stdout, t1) = head_tail(&raw_out, spec.max_output_bytes);
        let (stderr, t2) = head_tail(&raw_err, spec.max_output_bytes);
        let truncated = t1 || t2;

        match wait {
            Wait::Exited(Ok(status)) => Ok(CommandOutcome {
                stdout,
                stderr,
                exit_code: status.code(),
                timed_out: false,
                truncated,
            }),
            Wait::Exited(Err(e)) => Err(CommandError::Spawn(format!("wait failed: {e}"))),
            Wait::TimedOut => Ok(CommandOutcome {
                stdout,
                stderr,
                exit_code: None,
                timed_out: true,
                truncated,
            }),
            Wait::Cancelled => Err(CommandError::Cancelled),
        }
    }
}

/// Drain `r` to EOF as lossy UTF-8; errors are treated as EOF.
async fn read_to_end<R: tokio::io::AsyncRead + Unpin>(mut r: R) -> String {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use cogito_protocol::CommandExecutor;
    use cogito_protocol::test_support::contract_command_executor as contract;

    use super::*;

    fn exec() -> Arc<dyn CommandExecutor> {
        Arc::new(DirectExecutor::new(DirectConfig::default()))
    }

    #[tokio::test]
    async fn success() {
        contract::contract_success(exec()).await;
    }
    #[tokio::test]
    async fn nonzero_exit() {
        contract::contract_nonzero_exit(exec()).await;
    }
    #[tokio::test]
    async fn timeout() {
        contract::contract_timeout(exec()).await;
    }
    #[tokio::test]
    async fn truncation() {
        contract::contract_truncation(exec()).await;
    }
}

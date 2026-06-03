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

use crate::config::{DirectConfig, EnvPolicy};
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
        // Apply the environment policy.
        match &self.cfg.env_policy {
            // InheritAll: exact v0.1 behavior. Honor `inherit_env` — when
            // false, start from an empty environment; when true, inherit the
            // parent's. `inherit_env` is ignored by every other policy.
            EnvPolicy::InheritAll => {
                if !self.cfg.inherit_env {
                    cmd.env_clear();
                }
            }
            // Allowlist: default-deny. Start from an empty environment and copy
            // in only the listed keys that actually exist in the parent. Secrets
            // outside the list never reach the child.
            EnvPolicy::Allowlist(keys) => {
                cmd.env_clear();
                for key in keys {
                    if let Ok(val) = std::env::var(key) {
                        cmd.env(key, val);
                    }
                }
            }
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

    use std::time::Duration;

    use cogito_protocol::command::CommandSpec;
    use cogito_protocol::ids::{SessionId, TurnId};

    use crate::default_safe_env_allowlist;

    fn env_spec(command: &str) -> CommandSpec {
        CommandSpec {
            command: command.to_string(),
            cwd: None,
            timeout: Duration::from_secs(10),
            max_output_bytes: 4096,
        }
    }

    fn env_ctx() -> ExecCtx {
        ExecCtx::open_ended(SessionId::new(), TurnId::new())
    }

    // The workspace sets `unsafe_code = "forbid"`, which cannot be overridden
    // by `#[allow]`, so `std::env::set_var` (unsafe in Rust 2024) is off the
    // table. We use `temp_env::async_with_vars`, the established workspace
    // pattern for scoping environment mutation around an async block. nextest
    // still runs each test in its own process, so there is no cross-test race.
    const SECRET_KEY: &str = "COGITO_ENVPOLICY_SECRET_XYZ";

    #[tokio::test]
    async fn allowlist_scrubs_secret_but_keeps_path() {
        temp_env::async_with_vars([(SECRET_KEY, Some("leaked"))], async {
            let cfg = DirectConfig {
                env_policy: EnvPolicy::Allowlist(vec!["PATH".into()]),
                ..DirectConfig::default()
            };
            let exec = DirectExecutor::new(cfg);
            let out = exec
                .run(
                    env_spec(
                        "echo \"SECRET=[$COGITO_ENVPOLICY_SECRET_XYZ]\"; echo \"PATH_LEN=${#PATH}\"",
                    ),
                    env_ctx(),
                )
                .await
                .expect("command should spawn");
            assert!(
                out.stdout.contains("SECRET=[]"),
                "secret should be scrubbed, stdout was {:?}",
                out.stdout
            );
            assert!(
                !out.stdout.contains("leaked"),
                "secret value must not leak, stdout was {:?}",
                out.stdout
            );
            assert!(
                !out.stdout.contains("PATH_LEN=0"),
                "PATH should be present and non-empty, stdout was {:?}",
                out.stdout
            );
        })
        .await;
    }

    #[tokio::test]
    async fn inherit_all_preserves_secret() {
        temp_env::async_with_vars([(SECRET_KEY, Some("leaked"))], async {
            let cfg = DirectConfig {
                env_policy: EnvPolicy::InheritAll,
                inherit_env: true,
                ..DirectConfig::default()
            };
            let exec = DirectExecutor::new(cfg);
            let out = exec
                .run(
                    env_spec("echo \"[$COGITO_ENVPOLICY_SECRET_XYZ]\""),
                    env_ctx(),
                )
                .await
                .expect("command should spawn");
            assert!(
                out.stdout.contains("leaked"),
                "inherit-all must preserve the parent env, stdout was {:?}",
                out.stdout
            );
        })
        .await;
    }

    #[test]
    fn default_safe_env_allowlist_has_path_and_home() {
        let list = default_safe_env_allowlist();
        assert!(list.contains(&"PATH".to_string()));
        assert!(list.contains(&"HOME".to_string()));
        // Sanity: no obvious secret-bearing names in the curated default.
        for name in &list {
            let upper = name.to_uppercase();
            assert!(
                !upper.contains("SECRET")
                    && !upper.contains("TOKEN")
                    && !upper.contains("KEY")
                    && !upper.contains("PASSWORD"),
                "unexpected secret-ish entry: {name}"
            );
        }
    }
}

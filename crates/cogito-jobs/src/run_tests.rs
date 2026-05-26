//! `run_tests` — production async tool wrapping `cargo nextest run`.
//!
//! Shape: a one-tool [`ToolProvider`] named `run_tests` whose schema
//! accepts an optional `package` and optional `filter`. Every invocation
//! spawns `cargo nextest run [args]` via `tokio::process::Command` with
//! `stdout` / `stderr` piped, streams both pipes concurrently in
//! background tasks, and races `child.wait()` against the [`ExecCtx`]
//! cancel token and a deadline (default 10 minutes). On cancel or
//! deadline the child is killed; on a clean exit the captured streams
//! are truncated to 32 KiB head + 32 KiB tail (with an elision marker)
//! and returned as a JSON `Output` value carrying `stdout`, `stderr`,
//! and `exit_code`.
//!
//! Like [`crate::SleepTool`], `RunTestsTool` implements [`ToolProvider`]
//! directly (rather than the `BuiltinTool` adapter in `cogito-tools`)
//! because the dispatch outcome is [`InvokeOutcome::Async`]; the
//! `BuiltinTool` shape is sync-only.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::job::JobOutcome;
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use cogito_protocol::job::LocalJobSubmitter;

/// Tool name exposed to the model.
const TOOL_NAME: &str = "run_tests";

/// Default wall-clock budget for one `cargo nextest run` invocation
/// when [`ExecCtx::deadline`] is absent. Ten minutes is enough for a
/// medium workspace; longer runs should set a turn-level deadline.
const DEFAULT_DEADLINE_SECS: u64 = 600;

/// Maximum bytes preserved from the head of stdout/stderr after truncation.
const TRUNCATE_HEAD_BYTES: usize = 32 * 1024;

/// Maximum bytes preserved from the tail of stdout/stderr after truncation.
const TRUNCATE_TAIL_BYTES: usize = 32 * 1024;

/// Arguments accepted by [`RunTestsTool`]. Both fields are optional.
#[derive(Debug, Default, Deserialize)]
struct RunTestsArgs {
    /// Restrict to a single Cargo package via `-p <pkg>`.
    #[serde(default)]
    package: Option<String>,
    /// Test name filter passed verbatim to `cargo nextest run`.
    #[serde(default)]
    filter: Option<String>,
}

/// Async tool that runs `cargo nextest run` as a [`LocalJobSubmitter`] job.
///
/// Construct via [`RunTestsTool::new`] with the same `Arc<dyn LocalJobSubmitter>`
/// (typically an `Arc<LocalJobManager>` which coerces automatically) the
/// `RuntimeBuilder` will receive — otherwise the job submitted here is
/// invisible to the Brain registering `on_complete`.
pub struct RunTestsTool {
    job_mgr: Arc<dyn LocalJobSubmitter>,
}

impl RunTestsTool {
    /// Build a new `RunTestsTool` bound to `job_mgr`.
    ///
    /// `Arc<LocalJobManager>` coerces to `Arc<dyn LocalJobSubmitter>`
    /// automatically (since `LocalJobManager` impls the trait), so
    /// CLI / test callers can pass either form. See ADR-0025 for why
    /// the parameter is a trait object rather than the concrete type.
    #[must_use]
    pub fn new(job_mgr: Arc<dyn LocalJobSubmitter>) -> Self {
        Self { job_mgr }
    }
}

#[async_trait]
impl ToolProvider for RunTestsTool {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: TOOL_NAME.into(),
            description:
                "Run `cargo nextest run` with optional package / filter and return captured output."
                    .into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Optional Cargo package to restrict the run to (`-p <pkg>`)."
                    },
                    "filter": {
                        "type": "string",
                        "description": "Optional test-name filter passed verbatim to `cargo nextest run`."
                    }
                },
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysAsync,
            outputs_model_visible_multimodal: false,
        }]
    }

    async fn invoke(&self, name: &str, args: serde_json::Value, ctx: ExecCtx) -> InvokeOutcome {
        if name != TOOL_NAME {
            return InvokeOutcome::Sync(ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("unknown tool: {name}"),
                retryable: false,
            });
        }
        let parsed: RunTestsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return InvokeOutcome::Sync(ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("run_tests args: {e}"),
                    retryable: false,
                });
            }
        };
        // Translate `ctx.deadline` (an absolute `Instant`) into a relative
        // `Duration` for `tokio::time::sleep`. If the deadline has already
        // elapsed, fall through to a near-zero duration so the spawned task
        // surfaces a timeout immediately rather than running unbounded.
        let deadline = match ctx.deadline {
            Some(when) => when
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_millis(0)),
            None => Duration::from_secs(DEFAULT_DEADLINE_SECS),
        };
        let cancel = ctx.cancel.clone();
        let job_id = self
            .job_mgr
            .clone()
            .submit_boxed(Box::pin(async move {
                run_cargo_nextest(parsed, deadline, cancel).await
            }))
            .await;
        InvokeOutcome::Async(job_id)
    }
}

/// Spawn `cargo nextest run` and race its completion against the
/// cancellation token and deadline.
async fn run_cargo_nextest(
    args: RunTestsArgs,
    deadline: Duration,
    cancel: CancellationToken,
) -> JobOutcome {
    let mut cmd = Command::new("cargo");
    cmd.arg("nextest").arg("run");
    if let Some(pkg) = &args.package {
        cmd.arg("-p").arg(pkg);
    }
    if let Some(filter) = &args.filter {
        cmd.arg(filter);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return JobOutcome::Failed {
                message: format!("failed to spawn `cargo nextest run`: {e}"),
            };
        }
    };

    // `child.stdout` / `child.stderr` are `Some` because we configured the
    // command with `Stdio::piped()` above. The codebase forbids `expect_used`
    // and `unwrap_used` at deny level, so we surface a structured failure
    // instead of panicking should the invariant ever drift.
    let Some(stdout) = child.stdout.take() else {
        return JobOutcome::Failed {
            message: "cargo nextest: child stdout pipe missing after spawn".into(),
        };
    };
    let Some(stderr) = child.stderr.take() else {
        return JobOutcome::Failed {
            message: "cargo nextest: child stderr pipe missing after spawn".into(),
        };
    };
    // Streaming readers run concurrently with `child.wait()` so a kill on
    // cancel/timeout still lets us collect whatever was written before the
    // signal landed.
    let stdout_task = tokio::spawn(read_to_end(stdout));
    let stderr_task = tokio::spawn(read_to_end(stderr));

    // TODO(subprocess-cancel-orphan): LocalJobManager::cancel(job_id) currently
    // fires the spawned task's AbortHandle, which terminates this future at its
    // next .await BEFORE the `cancel.cancelled()` select arm has a chance to fire
    // child.kill().await. As a result, a manager-driven cancel orphans the cargo
    // nextest subprocess. The `cancel.cancelled()` path here only fires if some
    // other code signals the CancellationToken (e.g., the host runtime's
    // shutdown). Fix: thread a per-job CancellationToken into LocalJobManager
    // (see ADR follow-up), and have cancel() signal it before calling abort().
    // Sprint 8 closure note — see plan TODO.
    let wait_outcome = tokio::select! {
        status = child.wait() => WaitOutcome::Exited(status),
        () = cancel.cancelled() => {
            let _ = child.kill().await;
            WaitOutcome::Cancelled
        }
        () = tokio::time::sleep(deadline) => {
            let _ = child.kill().await;
            WaitOutcome::TimedOut
        }
    };

    // Always join the streaming tasks so the captured bytes are available
    // (even on cancel/timeout paths we want to drain pipes before returning).
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();

    match wait_outcome {
        WaitOutcome::Exited(Ok(status)) => JobOutcome::Success {
            result: ToolResult::Output(vec![serde_json::json!({
                "stdout": truncate(&stdout),
                "stderr": truncate(&stderr),
                "exit_code": status.code(),
            })]),
        },
        WaitOutcome::Exited(Err(e)) => JobOutcome::Failed {
            message: format!("cargo nextest wait failed: {e}"),
        },
        WaitOutcome::Cancelled => JobOutcome::Cancelled,
        WaitOutcome::TimedOut => JobOutcome::Failed {
            message: format!("cargo nextest exceeded deadline of {}s", deadline.as_secs()),
        },
    }
}

/// Internal terminal state for the `select!` race in `run_cargo_nextest`.
enum WaitOutcome {
    /// Child exited (either successfully or `wait` itself returned an error).
    Exited(std::io::Result<std::process::ExitStatus>),
    /// Cancellation token fired; child was killed.
    Cancelled,
    /// Deadline elapsed; child was killed.
    TimedOut,
}

/// Drain `r` to EOF, returning the lossy-UTF-8 representation. Errors are
/// silently treated as EOF — the captured stream is best-effort.
async fn read_to_end<R: tokio::io::AsyncRead + Unpin>(mut r: R) -> String {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).to_string()
}

/// Truncate `s` to `TRUNCATE_HEAD_BYTES` of head + `TRUNCATE_TAIL_BYTES` of
/// tail, joined with an explicit elision marker. Strings within budget pass
/// through unchanged. UTF-8 boundaries are respected by walking backwards
/// from the byte cut to the nearest character boundary.
fn truncate(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() <= TRUNCATE_HEAD_BYTES + TRUNCATE_TAIL_BYTES {
        return s.to_string();
    }
    let head_end = floor_char_boundary(s, TRUNCATE_HEAD_BYTES);
    let tail_start = ceil_char_boundary(s, bytes.len() - TRUNCATE_TAIL_BYTES);
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let elided = bytes.len() - head.len() - tail.len();
    format!("{head}\n... [{elided} bytes elided] ...\n{tail}")
}

/// Round `idx` down to the nearest UTF-8 char boundary in `s`.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `idx` up to the nearest UTF-8 char boundary in `s`.
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::LocalJobManager;
    use cogito_protocol::ids::{SessionId, TurnId};

    #[test]
    fn truncate_passes_short_strings_through() {
        let s = "hello world";
        assert_eq!(truncate(s), s);
    }

    #[test]
    fn truncate_passes_exact_budget_through() {
        let s = "a".repeat(TRUNCATE_HEAD_BYTES + TRUNCATE_TAIL_BYTES);
        assert_eq!(truncate(&s), s);
    }

    #[test]
    fn truncate_collapses_large_strings() {
        let s = "a".repeat(100 * 1024);
        let out = truncate(&s);
        assert!(out.contains("... ["));
        assert!(out.contains("bytes elided"));
        assert!(out.len() < s.len());
    }

    #[test]
    fn truncate_reports_correct_elided_byte_count() {
        let total = TRUNCATE_HEAD_BYTES + TRUNCATE_TAIL_BYTES + 1234;
        let s = "x".repeat(total);
        let out = truncate(&s);
        assert!(out.contains("[1234 bytes elided]"));
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        // Build a string whose naive byte cut would land in the middle of
        // a multi-byte char. Use a 3-byte UTF-8 char ('世' = E4 B8 96)
        // repeated past the budget; the boundary helpers must never split it.
        let ch = "\u{4e16}"; // '世', 3 bytes
        let big = ch.repeat((TRUNCATE_HEAD_BYTES + TRUNCATE_TAIL_BYTES) / 3 + 100);
        let out = truncate(&big);
        // Must still be valid UTF-8 (format! would already have asserted
        // this, but verify the cut respected boundaries by checking the
        // marker is present and round-trips through a string).
        assert!(out.contains("bytes elided"));
        assert!(out.is_char_boundary(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn descriptor_shape() {
        let tool = RunTestsTool::new(LocalJobManager::new());
        let descs = tool.list();
        assert_eq!(descs.len(), 1);
        let d = &descs[0];
        assert_eq!(d.name, "run_tests");
        assert_eq!(d.execution_class, ExecutionClass::AlwaysAsync);
        assert!(!d.outputs_model_visible_multimodal);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invoke_with_bad_args_returns_sync_error() {
        let tool = RunTestsTool::new(LocalJobManager::new());
        // `package` must be a string if present; supplying a non-string
        // type forces serde to reject the args.
        let outcome = tool
            .invoke(
                "run_tests",
                serde_json::json!({ "package": 7 }),
                ExecCtx::open_ended(SessionId::new(), TurnId::new()),
            )
            .await;
        match outcome {
            InvokeOutcome::Sync(ToolResult::Error {
                kind, retryable, ..
            }) => {
                assert_eq!(kind, ToolErrorKind::InvalidArgs);
                assert!(!retryable);
            }
            other => panic!("expected Sync(Error), got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_tool_name_returns_sync_error() {
        let tool = RunTestsTool::new(LocalJobManager::new());
        let outcome = tool
            .invoke(
                "other",
                serde_json::json!({}),
                ExecCtx::open_ended(SessionId::new(), TurnId::new()),
            )
            .await;
        match outcome {
            InvokeOutcome::Sync(ToolResult::Error { kind, .. }) => {
                assert_eq!(kind, ToolErrorKind::InvocationFailed);
            }
            other => panic!("expected Sync(Error), got {other:?}"),
        }
    }
}

//! `bash` — Adaptive shell tool. Synchronous for normal commands; submits
//! a background job when `background: true`. All execution goes through an
//! injected `CommandExecutor`, so whether it is sandboxed is a policy
//! decision made at the Surface layer (see ADR-0027). Implements
//! `ToolProvider` directly (not `BuiltinTool`) because Adaptive dispatch
//! returns either `InvokeOutcome::Sync` or `Async` per call.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cogito_protocol::ExecCtx;
use cogito_protocol::command::{CommandError, CommandExecutor, CommandSpec};
use cogito_protocol::job::{JobOutcome, LocalJobSubmitter};
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use serde::Deserialize;

/// Tool name exposed to the model.
const TOOL_NAME: &str = "bash";

/// Tunables for `bash`. Lives here (owning crate) and is aggregated into
/// `cogito-config`'s `[tools]` section.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BashConfig {
    /// Timeout for synchronous (non-background) commands, seconds.
    pub sync_timeout_secs: u64,
    /// Deadline for background commands, seconds.
    pub background_deadline_secs: u64,
    /// Per-stream output byte budget (head + tail kept).
    pub max_output_bytes: usize,
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            sync_timeout_secs: 30,
            background_deadline_secs: 600,
            max_output_bytes: 32 * 1024,
        }
    }
}

/// Arguments accepted by [`BashTool`].
#[derive(Debug, Deserialize)]
struct BashArgs {
    /// Shell command to run via `sh -c`.
    command: String,
    /// When `true`, dispatch as a background job (Adaptive -> `Async`).
    #[serde(default)]
    background: bool,
    /// Optional working directory (relative to workspace root or absolute).
    #[serde(default)]
    cwd: Option<String>,
    /// Optional override of the synchronous timeout (ignored when background).
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// Adaptive shell tool bound to a `CommandExecutor` + job submitter.
pub struct BashTool {
    executor: Arc<dyn CommandExecutor>,
    job_mgr: Arc<dyn LocalJobSubmitter>,
    cfg: BashConfig,
}

impl BashTool {
    /// Construct a `BashTool`. `executor` is the policy-selected command
    /// executor (e.g. from `cogito_sandbox::build_executor`); `job_mgr` is
    /// the same submitter wired into `RuntimeBuilder::job_manager`.
    #[must_use]
    pub fn new(
        executor: Arc<dyn CommandExecutor>,
        job_mgr: Arc<dyn LocalJobSubmitter>,
        cfg: BashConfig,
    ) -> Self {
        Self {
            executor,
            job_mgr,
            cfg,
        }
    }

    fn spec(&self, args: &BashArgs, timeout: Duration) -> CommandSpec {
        CommandSpec {
            command: args.command.clone(),
            cwd: args.cwd.as_ref().map(std::path::PathBuf::from),
            timeout,
            max_output_bytes: self.cfg.max_output_bytes,
        }
    }
}

/// Convert a `CommandOutcome` to the JSON tool payload shape shared with
/// `run_tests`: `{ stdout, stderr, exit_code }`.
fn outcome_value(o: &cogito_protocol::command::CommandOutcome) -> serde_json::Value {
    serde_json::json!({
        "stdout": o.stdout,
        "stderr": o.stderr,
        "exit_code": o.exit_code,
    })
}

#[async_trait]
impl ToolProvider for BashTool {
    fn list(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: TOOL_NAME.into(),
            description:
                "Run a shell command via `sh -c`. Set background:true for long-running commands \
                 (the turn pauses and resumes when the command finishes)."
                    .into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run via `sh -c`." },
                    "background": { "type": "boolean", "description": "Run as a background job; the turn pauses and resumes on completion." },
                    "cwd": { "type": "string", "description": "Working dir relative to the workspace root (or absolute)." },
                    "timeout_secs": { "type": "number", "description": "Override the synchronous timeout (ignored when background)." }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::Adaptive,
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
        let args: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return InvokeOutcome::Sync(ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("bash args: {e}"),
                    retryable: false,
                });
            }
        };

        if args.background {
            self.invoke_background(args, ctx).await
        } else {
            self.invoke_sync(args, ctx).await
        }
    }
}

impl BashTool {
    async fn invoke_sync(&self, args: BashArgs, ctx: ExecCtx) -> InvokeOutcome {
        let timeout = Duration::from_secs(args.timeout_secs.unwrap_or(self.cfg.sync_timeout_secs));
        let spec = self.spec(&args, timeout);
        let result = match self.executor.run(spec, ctx).await {
            Ok(o) if o.timed_out => ToolResult::Error {
                kind: ToolErrorKind::Timeout,
                message: format!(
                    "command timed out after {}s; pass background:true for long-running commands",
                    timeout.as_secs()
                ),
                retryable: true,
            },
            Ok(o) => ToolResult::Output(vec![outcome_value(&o)]),
            Err(CommandError::Cancelled) => ToolResult::Error {
                kind: ToolErrorKind::Cancelled,
                message: "bash command cancelled".into(),
                retryable: false,
            },
            Err(CommandError::Spawn(e)) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("bash: {e}"),
                retryable: false,
            },
            // `CommandError` is `#[non_exhaustive]`; treat any future variant
            // as a generic invocation failure rather than panicking.
            Err(e) => ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("bash: {e}"),
                retryable: false,
            },
        };
        InvokeOutcome::Sync(result)
    }
    async fn invoke_background(&self, args: BashArgs, ctx: ExecCtx) -> InvokeOutcome {
        let timeout = Duration::from_secs(self.cfg.background_deadline_secs);
        let spec = self.spec(&args, timeout);
        let executor = Arc::clone(&self.executor);
        // Background commands carry the turn's cancel token so a session
        // shutdown / cancel still kills the child.
        let run_ctx = ctx;
        let job_id = self
            .job_mgr
            .clone()
            .submit_boxed(Box::pin(async move {
                match executor.run(spec, run_ctx).await {
                    Ok(o) if o.timed_out => JobOutcome::Failed {
                        message: format!("bash background command exceeded {}s", timeout.as_secs()),
                    },
                    Ok(o) => JobOutcome::Success {
                        result: ToolResult::Output(vec![outcome_value(&o)]),
                    },
                    Err(CommandError::Cancelled) => JobOutcome::Cancelled,
                    Err(CommandError::Spawn(e)) => JobOutcome::Failed {
                        message: format!("bash: {e}"),
                    },
                    // `CommandError` is `#[non_exhaustive]`; treat any future
                    // variant as a generic failure rather than panicking.
                    Err(e) => JobOutcome::Failed {
                        message: format!("bash: {e}"),
                    },
                }
            }))
            .await;
        InvokeOutcome::Async(job_id)
    }
}

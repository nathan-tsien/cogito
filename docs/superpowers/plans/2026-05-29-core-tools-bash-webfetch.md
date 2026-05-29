# bash + web_fetch 核心工具 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 cogito 增加 `bash`(Adaptive:同步 / `background:true` 异步)与 `web_fetch`(同步,HTML→markdown)两个核心工具,并把空的 `cogito-sandbox` 立成"策略选择的 `CommandExecutor`"执行接缝。

**Architecture:** 在 `cogito-protocol` 定义 `CommandExecutor` trait(子进程执行抽象,运行期注入,不序列化);`cogito-sandbox` 提供 v0.1 唯一实现 `DirectExecutor`(`sh -c`,在宿主跑,非安全边界)+ `build_executor` 工厂;`bash` 工具(`cogito-jobs`,直接实现 `ToolProvider`)只依赖该 trait,构造期注入 executor + `LocalJobSubmitter`;`web_fetch`(`cogito-tools`,`BuiltinTool`)用 `reqwest` + `htmd`;`[tools]` 配置段在 `cogito-config` 聚合;两个 Surface(CLI/TUI)接线。设计依据见 `docs/superpowers/specs/2026-05-29-core-tools-bash-webfetch-design.md`。

**Tech Stack:** Rust 2024 / tokio(process、io-util、time)/ reqwest 0.12(rustls)/ htmd 0.2(HTML→markdown)/ async-trait / serde / cargo-nextest。

**重要约定(每个任务都适用):**
- 所有代码注释用英文(CLAUDE.md)。
- 不允许 `unwrap`/`expect`/`panic`/`dbg!`(clippy deny);测试模块用 `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`,照搬现有测试文件的做法。
- 不用 `println!`/`eprintln!`,用 `tracing`。
- 完成后:`make fmt && make fix CRATE=<crate>` 干净、`make test CRATE=<crate>` 绿;最后 `make ci` 全绿(含 fmt-check + clippy + layer-check + test)。
- cargo 命令较慢属正常,**不要按 PID 杀**(会损坏 lock 文件)。

---

## File Structure

| 文件 | 责任 |
|---|---|
| `crates/cogito-protocol/src/command.rs`(新建) | `CommandSpec` / `CommandOutcome` / `CommandError` / `CommandExecutor` trait |
| `crates/cogito-protocol/src/lib.rs`(改) | 注册 `pub mod command;` + re-export |
| `crates/cogito-protocol/src/test_support/contract_command_executor.rs`(新建) | 所有 `CommandExecutor` 实现共用的契约测试 |
| `crates/cogito-protocol/src/test_support/mod.rs`(改) | 注册契约模块 |
| `crates/cogito-sandbox/Cargo.toml`(改) | tokio process/io-util/time 等 feature |
| `crates/cogito-sandbox/src/truncate.rs`(新建) | 头尾字节截断(UTF-8 安全),从 run_tests 提炼 |
| `crates/cogito-sandbox/src/executor.rs`(新建) | `DirectExecutor`(`sh -c` + select race + 截断) |
| `crates/cogito-sandbox/src/config.rs`(新建) | `SandboxConfig`(tagged)+ `DirectConfig` + `SandboxError` |
| `crates/cogito-sandbox/src/lib.rs`(改) | 导出 + `build_executor` 工厂 |
| `crates/cogito-jobs/src/bash.rs`(新建) | `BashConfig` + `BashTool`(Adaptive `ToolProvider`) |
| `crates/cogito-jobs/src/lib.rs`(改) | 导出 `BashTool` / `BashConfig` |
| `crates/cogito-jobs/tests/bash_tool.rs`(新建) | bash 集成测试(sync / background / timeout / 非零退出) |
| `Cargo.toml`(workspace,改) | 加 `htmd` 到 `[workspace.dependencies]` |
| `crates/cogito-tools/Cargo.toml`(改) | 加 reqwest + htmd;dev tokio net/time |
| `crates/cogito-tools/src/builtins/web_fetch.rs`(新建) | `WebFetchConfig` + `WebFetch`(`BuiltinTool`) |
| `crates/cogito-tools/src/builtins/mod.rs`(改) | 导出 `WebFetch` |
| `crates/cogito-tools/tests/web_fetch.rs`(新建) | web_fetch 集成测试(本地 TcpListener 服务) |
| `crates/cogito-config/Cargo.toml`(改) | 加 cogito-sandbox / cogito-jobs / cogito-tools |
| `crates/cogito-config/src/types.rs`(改) | `ToolsConfig` + RuntimeConfig/Partial 字段 + finalize |
| `crates/cogito-config/src/merge.rs`(改) | merge `tools` |
| `crates/cogito-cli/Cargo.toml` + `src/chat.rs`(改) | 注入 WebFetch + BashTool + build_executor |
| `crates/cogito-tui/Cargo.toml` + `src/runtime_build.rs`(改) | 同上,镜像 |
| ADR-0027 + H08/sandbox/配置文档(改/新建) | 决策记录与文档 |

---

## Phase 1 · Protocol 接缝(cogito-protocol)

### Task 1: 定义 CommandExecutor trait + 值类型

**Files:**
- Create: `crates/cogito-protocol/src/command.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: 写 `command.rs`**

```rust
//! `CommandExecutor` — the seam for running a subprocess in a
//! policy-selected environment. Whether execution is isolated is decided
//! by the concrete implementation injected at runtime, NOT by the tool
//! that calls it. v0.4 ADR-0012 (sandbox lifecycle) / ADR-0013
//! (credential isolation) extend this seam with isolating / remote impls.
//!
//! This is a runtime-only trait: it is never serialized into the event
//! log and is not part of the cross-language wire contract, so adding it
//! does not touch `SCHEMA_VERSION`.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::ExecCtx;

/// One command to execute. `env` policy and the root directory are
/// implementation/construction-time concerns (see `SandboxConfig`), so
/// they are deliberately absent here to keep the per-call surface minimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Shell command line. `DirectExecutor` runs it via `sh -c <command>`.
    pub command: String,
    /// Working directory. Relative paths resolve against the executor's
    /// configured root; `None` means "use the root".
    pub cwd: Option<PathBuf>,
    /// Hard wall-clock timeout for this execution.
    pub timeout: Duration,
    /// Per-stream byte budget for stdout/stderr (head + tail kept, middle
    /// elided when exceeded).
    pub max_output_bytes: usize,
}

/// Captured result of a finished (or timed-out) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Captured stdout (possibly truncated; see `truncated`).
    pub stdout: String,
    /// Captured stderr (possibly truncated; see `truncated`).
    pub stderr: String,
    /// Process exit code; `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// `true` when the command was killed because `timeout` elapsed.
    pub timed_out: bool,
    /// `true` when stdout or stderr was truncated to `max_output_bytes`.
    pub truncated: bool,
}

/// Failure modes that prevent producing a `CommandOutcome`.
///
/// A non-zero exit code is NOT an error — it is a normal `CommandOutcome`
/// with `exit_code = Some(n)`. A timeout is also not an error — it is a
/// `CommandOutcome` with `timed_out = true` and whatever output was
/// captured before the kill. Only spawn failure and cooperative
/// cancellation surface here.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandError {
    /// The process could not be spawned (binary missing, permission, …).
    #[error("failed to spawn command: {0}")]
    Spawn(String),
    /// `ExecCtx::cancel` fired; the child was killed and execution aborted.
    #[error("command cancelled")]
    Cancelled,
}

/// Abstraction over "run a subprocess". Implementations live in
/// `cogito-sandbox` (v0.1: `DirectExecutor`) and are injected into tools
/// (e.g. `bash`) at construction time. Brain / H08 never see this trait.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute `spec`, honoring `ctx.cancel`. Returns the captured outcome,
    /// or a `CommandError` for spawn failure / cancellation.
    async fn run(&self, spec: CommandSpec, ctx: ExecCtx) -> Result<CommandOutcome, CommandError>;
}
```

- [ ] **Step 2: 在 `lib.rs` 注册模块 + re-export**

在模块声明区(`pub mod content;` 一带)按字母序插入:

```rust
pub mod command;
```

在 re-export 区(`pub use content::ContentBlock;` 一带)插入:

```rust
pub use command::{CommandError, CommandExecutor, CommandOutcome, CommandSpec};
```

同时在文件顶部的「Module map」doc 注释里加一行(保持文档与代码同步):

```rust
//! - [`command`]: `CommandExecutor` trait + `CommandSpec`/`CommandOutcome` — subprocess execution seam (sandbox policy)
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p cogito-protocol`
Expected: 编译通过(trait/类型仅声明,无逻辑)。

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/command.rs crates/cogito-protocol/src/lib.rs
git commit -m "feat(protocol): add CommandExecutor seam (CommandSpec/Outcome/Error)"
```

---

### Task 2: CommandExecutor 契约测试

**Files:**
- Create: `crates/cogito-protocol/src/test_support/contract_command_executor.rs`
- Modify: `crates/cogito-protocol/src/test_support/mod.rs`

先确认 `test_support` 现有结构:

Run: `sed -n '1,40p' crates/cogito-protocol/src/test_support/mod.rs`
Expected: 看到 `pub mod contract_job_manager;` 之类的声明(契约测试集中在此,门控在 `feature = "test-support"`)。

- [ ] **Step 1: 写契约函数**

```rust
//! Shared contract every `CommandExecutor` implementation must satisfy.
//! A backend crate (e.g. `cogito-sandbox`) calls these from its own test
//! module against its concrete executor.
//!
//! Marked test-only via the crate's `test-support` feature.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use crate::command::{CommandExecutor, CommandSpec};
use crate::ids::{SessionId, TurnId};
use crate::ExecCtx;

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
```

- [ ] **Step 2: 在 `test_support/mod.rs` 注册**

追加一行:

```rust
pub mod contract_command_executor;
```

- [ ] **Step 3: 编译验证(带 feature)**

Run: `cargo build -p cogito-protocol --features test-support`
Expected: 通过。契约函数此刻无人调用,后续 Task 4 在 `cogito-sandbox` 调用它们。

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/test_support/
git commit -m "test(protocol): add CommandExecutor contract suite"
```

---

## Phase 2 · DirectExecutor(cogito-sandbox)

### Task 3: sandbox 依赖 + config 类型 + 工厂骨架

**Files:**
- Modify: `crates/cogito-sandbox/Cargo.toml`
- Create: `crates/cogito-sandbox/src/config.rs`
- Modify: `crates/cogito-sandbox/src/lib.rs`

- [ ] **Step 1: 补 Cargo 依赖**

把 `crates/cogito-sandbox/Cargo.toml` 的 `[dependencies]` 改为(`tokio` 加 process/io-util/time/rt;新增 serde derive 已经有 serde,确认 features):

```toml
[dependencies]
cogito-protocol.workspace = true

tokio = { workspace = true, features = ["process", "io-util", "time", "rt", "macros"] }
async-trait.workspace = true
thiserror.workspace = true
tracing.workspace = true
serde = { workspace = true, features = ["derive"] }

[dev-dependencies]
tokio-test.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "time"] }
cogito-protocol = { workspace = true, features = ["test-support"] }
```

- [ ] **Step 2: 写 `config.rs`**

```rust
//! Sandbox configuration: a tagged-union over the executor kinds
//! `cogito-sandbox` knows how to construct. Per CLAUDE.md, the
//! `match`-on-kind dispatch (`build_executor`) lives in this crate; no
//! surface reproduces it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tagged config selecting a `CommandExecutor` implementation. v0.1 ships
/// only `Direct`; v0.4 (ADR-0012/0013) adds isolating / remote variants.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxConfig {
    /// No isolation: run on the host. The "sandbox off" default.
    #[default]
    Direct(DirectConfig),
}

/// Configuration for `DirectExecutor`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DirectConfig {
    /// Root working directory. Relative `CommandSpec::cwd` resolves under
    /// this. Defaults to the process current dir (`.`).
    pub root: PathBuf,
    /// Whether the child inherits the parent process environment. Defaults
    /// to `true` (v0.1 is not a security boundary).
    pub inherit_env: bool,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            inherit_env: true,
        }
    }
}

/// Error building a `CommandExecutor` from config.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// Reserved for future variants that validate isolation prerequisites
    /// (namespaces, cgroups). `Direct` never errors today.
    #[error("sandbox configuration error: {0}")]
    Config(String),
}
```

- [ ] **Step 3: 在 `lib.rs` 写工厂骨架 + 模块声明**

把 `crates/cogito-sandbox/src/lib.rs` 改为(保留原 `//!` 头注释,追加内容):

```rust
//! cogito-sandbox
//!
//! Subprocess-based execution sandbox. Provides cwd isolation, resource
//! limits, and timeout enforcement. Not a security boundary — that's a
//! production concern (v0.4 ADR-0012/0013). Goal here is to *behave* like
//! a sandbox so the Harness can be validated against the production
//! contract. Implements `cogito_protocol::CommandExecutor`; the executor
//! is selected by `build_executor` and injected into tools (e.g. `bash`)
//! at the Surface layer.

mod config;
mod executor;
mod truncate;

use std::sync::Arc;

use cogito_protocol::CommandExecutor;

pub use config::{DirectConfig, SandboxConfig, SandboxError};
pub use executor::DirectExecutor;

/// Build a `CommandExecutor` from `SandboxConfig`. The only place in the
/// workspace that pattern-matches on the sandbox `kind`; surfaces call
/// this and receive a trait object.
///
/// # Errors
///
/// Returns `SandboxError` for configurations whose prerequisites cannot be
/// satisfied. `Direct` never errors today.
pub fn build_executor(cfg: &SandboxConfig) -> Result<Arc<dyn CommandExecutor>, SandboxError> {
    match cfg {
        SandboxConfig::Direct(c) => Ok(Arc::new(DirectExecutor::new(c.clone()))),
    }
}
```

注:此刻 `executor`/`truncate` 模块还没建,Step 4 之前不可编译——它们在 Task 4 完成。**本任务最后不单独编译**;与 Task 4 一起提交。先继续 Task 4。

- [ ] **Step 4: (合并到 Task 4 提交)**

---

### Task 4: DirectExecutor 实现 + 跑通契约

**Files:**
- Create: `crates/cogito-sandbox/src/truncate.rs`
- Create: `crates/cogito-sandbox/src/executor.rs`

- [ ] **Step 1: 写 `truncate.rs`(UTF-8 安全头尾截断)**

```rust
//! Head+tail byte-budget truncation, UTF-8 safe. Extracted to match the
//! behavior already used by `cogito-jobs::run_tests` so both eventually
//! share one implementation.

/// Truncate `s` so the head and tail together stay within `2 * max` bytes,
/// joined by an elision marker. Returns `(text, truncated)`.
pub fn head_tail(s: &str, max: usize) -> (String, bool) {
    let bytes = s.as_bytes();
    if bytes.len() <= max.saturating_mul(2) {
        return (s.to_string(), false);
    }
    let head_end = floor_char_boundary(s, max);
    let tail_start = ceil_char_boundary(s, bytes.len() - max);
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let elided = bytes.len() - head.len() - tail.len();
    (format!("{head}\n... [{elided} bytes elided] ...\n{tail}"), true)
}

fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn within_budget_passes_through() {
        let (out, trunc) = head_tail("short", 100);
        assert_eq!(out, "short");
        assert!(!trunc);
    }

    #[test]
    fn over_budget_is_elided() {
        let s = "a".repeat(1000);
        let (out, trunc) = head_tail(&s, 10);
        assert!(trunc);
        assert!(out.contains("bytes elided"));
        assert!(out.len() < s.len());
    }
}
```

- [ ] **Step 2: 写 `executor.rs`(失败测试先行 —— 契约调用)**

```rust
//! `DirectExecutor` — runs commands via `sh -c` on the host. v0.1 only
//! implementation of `CommandExecutor`. Not a security boundary: no
//! namespaces / seccomp / chroot. Mirrors the subprocess race pattern
//! validated by `cogito-jobs::run_tests`.

use std::process::Stdio;

use async_trait::async_trait;
use cogito_protocol::command::{CommandError, CommandExecutor, CommandOutcome, CommandSpec};
use cogito_protocol::ExecCtx;
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
    Exited(std::io::Result<std::process::ExitStatus>),
    Cancelled,
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

    use cogito_protocol::test_support::contract_command_executor as contract;
    use cogito_protocol::CommandExecutor;

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
```

- [ ] **Step 3: 跑测试验证失败→通过**

Run: `cargo nextest run -p cogito-sandbox`
Expected: 4 个契约测试 + 2 个 truncate 单测全绿。若 `sh`/`seq`/`sleep` 在 CI 镜像缺失,改用 POSIX 基础命令已足够(均为 coreutils/busybox 标配)。

- [ ] **Step 4: Commit(连同 Task 3)**

```bash
git add crates/cogito-sandbox/
git commit -m "feat(sandbox): DirectExecutor + SandboxConfig + build_executor"
```

---

## Phase 3 · bash 工具(cogito-jobs)

### Task 5: BashConfig + BashTool 骨架(descriptor + 参数解析)

**Files:**
- Create: `crates/cogito-jobs/src/bash.rs`
- Modify: `crates/cogito-jobs/src/lib.rs`

- [ ] **Step 1: 写 `bash.rs`(配置 + 结构 + descriptor + 参数解析)**

```rust
//! `bash` — Adaptive shell tool. Synchronous for normal commands; submits
//! a background job when `background: true`. All execution goes through an
//! injected `CommandExecutor`, so whether it is sandboxed is a policy
//! decision made at the Surface layer (see ADR-0027). Implements
//! `ToolProvider` directly (not `BuiltinTool`) because Adaptive dispatch
//! returns either `InvokeOutcome::Sync` or `Async` per call.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cogito_protocol::command::{CommandError, CommandExecutor, CommandSpec};
use cogito_protocol::job::{JobOutcome, LocalJobSubmitter};
use cogito_protocol::tool::{
    ExecutionClass, InvokeOutcome, ToolDescriptor, ToolErrorKind, ToolProvider, ToolResult,
};
use cogito_protocol::ExecCtx;
use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    background: bool,
    #[serde(default)]
    cwd: Option<String>,
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
```

注意:`invoke_sync` / `invoke_background` 在 Task 6 / Task 7 增加。本步只要结构 + descriptor + 参数解析能编译,先放两个临时桩:

```rust
impl BashTool {
    async fn invoke_sync(&self, _args: BashArgs, _ctx: ExecCtx) -> InvokeOutcome {
        InvokeOutcome::Sync(ToolResult::text("todo"))
    }
    async fn invoke_background(&self, _args: BashArgs, _ctx: ExecCtx) -> InvokeOutcome {
        InvokeOutcome::Sync(ToolResult::text("todo"))
    }
}
```

- [ ] **Step 2: 在 `lib.rs` 导出**

在 `crates/cogito-jobs/src/lib.rs` 追加:

```rust
mod bash;
pub use bash::{BashConfig, BashTool};
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p cogito-jobs`
Expected: 通过(桩实现)。

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-jobs/src/bash.rs crates/cogito-jobs/src/lib.rs
git commit -m "feat(jobs): BashTool skeleton (descriptor + args, Adaptive)"
```

---

### Task 6: bash 同步路径

**Files:**
- Modify: `crates/cogito-jobs/src/bash.rs`
- Create: `crates/cogito-jobs/tests/bash_tool.rs`

- [ ] **Step 1: 写失败测试(同步成功 / 非零退出 / 超时)**

`crates/cogito-jobs/tests/bash_tool.rs`:

```rust
//! Integration tests for `BashTool` against a real `DirectExecutor`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_jobs::{BashConfig, BashTool, LocalJobManager};
use cogito_protocol::command::CommandExecutor;
use cogito_protocol::job::{JobManager, JobOutcome, LocalJobSubmitter};
use cogito_protocol::tool::{InvokeOutcome, ToolErrorKind, ToolProvider, ToolResult};
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::ExecCtx;
use cogito_sandbox::{DirectConfig, DirectExecutor};

fn bash(cfg: BashConfig) -> (BashTool, Arc<LocalJobManager>) {
    let executor: Arc<dyn CommandExecutor> = Arc::new(DirectExecutor::new(DirectConfig::default()));
    let job_mgr = LocalJobManager::new();
    let tool = BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        cfg,
    );
    (tool, job_mgr)
}

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

fn exit_code(result: &ToolResult) -> Option<i64> {
    match result {
        ToolResult::Output(blocks) => blocks
            .first()
            .and_then(|v| v.get("exit_code"))
            .and_then(serde_json::Value::as_i64),
        _ => None,
    }
}

#[tokio::test]
async fn sync_success_returns_stdout_and_zero_exit() {
    let (tool, _jm) = bash(BashConfig::default());
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "echo hi" }), ctx())
        .await;
    let InvokeOutcome::Sync(result) = out else {
        panic!("expected Sync");
    };
    assert_eq!(exit_code(&result), Some(0));
    let ToolResult::Output(blocks) = &result else {
        panic!("expected Output");
    };
    let stdout = blocks[0].get("stdout").and_then(serde_json::Value::as_str).unwrap_or("");
    assert!(stdout.contains("hi"), "stdout={stdout:?}");
}

#[tokio::test]
async fn nonzero_exit_is_not_a_tool_error() {
    let (tool, _jm) = bash(BashConfig::default());
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "exit 7" }), ctx())
        .await;
    let InvokeOutcome::Sync(result) = out else {
        panic!("expected Sync");
    };
    assert!(
        !matches!(result, ToolResult::Error { .. }),
        "non-zero exit must surface as Output, not Error"
    );
    assert_eq!(exit_code(&result), Some(7));
}

#[tokio::test]
async fn sync_timeout_surfaces_timeout_error() {
    let cfg = BashConfig {
        sync_timeout_secs: 1,
        ..BashConfig::default()
    };
    let (tool, _jm) = bash(cfg);
    let out = tool
        .invoke("bash", serde_json::json!({ "command": "sleep 30" }), ctx())
        .await;
    let InvokeOutcome::Sync(ToolResult::Error { kind, .. }) = out else {
        panic!("expected Sync Error");
    };
    assert!(matches!(kind, ToolErrorKind::Timeout), "kind={kind:?}");
}
```

`crates/cogito-jobs/Cargo.toml` 的 `[dev-dependencies]` 加上 `cogito-sandbox`:

```toml
cogito-sandbox = { workspace = true }
```

(确认 `[workspace.dependencies]` 已含 `cogito-sandbox = { path = "crates/cogito-sandbox" }`;若无则补,与其他 crate 同样写法。)

- [ ] **Step 2: 跑测试看失败**

Run: `cargo nextest run -p cogito-jobs --test bash_tool`
Expected: FAIL(桩返回 `"todo"`,exit_code 取不到)。

- [ ] **Step 3: 实现 `invoke_sync`**

把 Task 5 的 `invoke_sync` 桩替换为:

```rust
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
        };
        InvokeOutcome::Sync(result)
    }
```

- [ ] **Step 4: 跑测试看通过**

Run: `cargo nextest run -p cogito-jobs --test bash_tool`
Expected: 3 个同步测试通过(background 测试在 Task 7 加)。

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-jobs/src/bash.rs crates/cogito-jobs/tests/bash_tool.rs crates/cogito-jobs/Cargo.toml
git commit -m "feat(jobs): bash synchronous path"
```

---

### Task 7: bash background(异步)路径

**Files:**
- Modify: `crates/cogito-jobs/src/bash.rs`
- Modify: `crates/cogito-jobs/tests/bash_tool.rs`

- [ ] **Step 1: 加失败测试(background → Async + 完成结果)**

追加到 `bash_tool.rs`:

```rust
#[tokio::test]
async fn background_returns_async_and_completes() {
    let (tool, job_mgr) = bash(BashConfig::default());
    let out = tool
        .invoke(
            "bash",
            serde_json::json!({ "command": "echo bg", "background": true }),
            ctx(),
        )
        .await;
    let InvokeOutcome::Async(job_id) = out else {
        panic!("expected Async");
    };

    // Poll the job manager until the job reaches a terminal outcome.
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(o) = job_mgr.result(job_id).await {
                return o;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("job should complete within 10s");

    let JobOutcome::Success { result } = outcome else {
        panic!("expected Success, got {outcome:?}");
    };
    assert_eq!(exit_code(&result), Some(0));
}
```

- [ ] **Step 2: 跑测试看失败**

Run: `cargo nextest run -p cogito-jobs --test bash_tool::background_returns_async_and_completes`
Expected: FAIL(桩返回 Sync)。

- [ ] **Step 3: 实现 `invoke_background`**

替换 `invoke_background` 桩为:

```rust
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
                }
            }))
            .await;
        InvokeOutcome::Async(job_id)
    }
```

- [ ] **Step 4: 跑全套 bash 测试**

Run: `cargo nextest run -p cogito-jobs --test bash_tool`
Expected: 4 个测试全绿。

- [ ] **Step 5: clippy + fmt**

Run: `make fix CRATE=cogito-jobs && make fmt`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-jobs/src/bash.rs crates/cogito-jobs/tests/bash_tool.rs
git commit -m "feat(jobs): bash background path via LocalJobSubmitter"
```

---

## Phase 4 · web_fetch 工具(cogito-tools)

### Task 8: 依赖 + WebFetch 骨架(descriptor + scheme 校验)

**Files:**
- Modify: `Cargo.toml`(workspace)
- Modify: `crates/cogito-tools/Cargo.toml`
- Create: `crates/cogito-tools/src/builtins/web_fetch.rs`
- Modify: `crates/cogito-tools/src/builtins/mod.rs`

- [ ] **Step 1: 加 htmd 到 workspace deps**

在根 `Cargo.toml` 的 `[workspace.dependencies]` 内,按字母序(`htmd` 在 `h` 段)插入:

```toml
htmd = "0.2"
```

先验证可解析:

Run: `cargo metadata --format-version=1 >/dev/null`
Expected: 无报错(确认 htmd 0.2 存在于 crates.io;若版本号需微调,以 `cargo add -p cogito-tools htmd --dry-run` 给出的最新 0.x 为准)。

- [ ] **Step 2: cogito-tools 依赖**

`crates/cogito-tools/Cargo.toml` 的 `[dependencies]` 追加:

```toml
reqwest = { workspace = true }
htmd = { workspace = true }
```

`tokio` 的 features 追加 `"time"`(reqwest 超时需要):把现有 `features = ["fs", "io-util", "sync"]` 改为 `["fs", "io-util", "sync", "time"]`。

`[dev-dependencies]` 的 `tokio` features 改为含网络与时间(测试要起本地 TCP 服务):

```toml
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "net", "io-util", "time"] }
```

- [ ] **Step 3: 写 `web_fetch.rs`(配置 + descriptor + scheme 校验,fetch 逻辑桩)**

```rust
//! `web_fetch` — fetch an http(s) URL and return its content as markdown
//! (HTML) or text. Synchronous `BuiltinTool`. Does NOT call any model
//! (stays provider-free); URL/SSRF gating is an H09 hook concern.

use async_trait::async_trait;
use cogito_protocol::tool::{ExecutionClass, ToolDescriptor, ToolErrorKind, ToolResult};
use cogito_protocol::ExecCtx;
use serde::Deserialize;

use crate::provider::BuiltinTool;

/// Tunables for `web_fetch`. Lives here (owning crate); aggregated into
/// `cogito-config`'s `[tools]` section.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WebFetchConfig {
    /// Per-request timeout, seconds.
    pub timeout_secs: u64,
    /// Maximum response body bytes read before truncation.
    pub max_bytes: usize,
    /// `User-Agent` header.
    pub user_agent: String,
    /// Maximum redirects to follow.
    pub max_redirects: usize,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_bytes: 1 << 20,
            user_agent: "cogito/0.1".into(),
            max_redirects: 5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    url: String,
}

/// HTML-to-markdown fetcher.
#[derive(Debug, Clone)]
pub struct WebFetch {
    cfg: WebFetchConfig,
}

impl WebFetch {
    /// Construct from config.
    #[must_use]
    pub fn new(cfg: WebFetchConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl BuiltinTool for WebFetch {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "web_fetch".into(),
            description:
                "Fetch an http(s) URL. HTML is converted to Markdown; other text is returned as-is."
                    .into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "http(s) URL to fetch." }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            execution_class: ExecutionClass::AlwaysSync,
            outputs_model_visible_multimodal: false,
        }
    }

    async fn invoke(&self, args: serde_json::Value, _ctx: ExecCtx) -> ToolResult {
        let Args { url } = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvalidArgs,
                    message: format!("web_fetch args: {e}"),
                    retryable: false,
                };
            }
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return ToolResult::Error {
                kind: ToolErrorKind::InvalidArgs,
                message: format!("web_fetch: only http(s) URLs are supported, got: {url}"),
                retryable: false,
            };
        }
        self.fetch(&url).await
    }
}

impl WebFetch {
    async fn fetch(&self, _url: &str) -> ToolResult {
        ToolResult::text("todo")
    }
}
```

- [ ] **Step 4: 导出**

`crates/cogito-tools/src/builtins/mod.rs` 改为:

```rust
//! Builtin tools bundled with `cogito-tools`. Each tool implements the
//! `BuiltinTool` trait.

pub mod read_file;
pub mod web_fetch;

pub use read_file::ReadFile;
pub use web_fetch::{WebFetch, WebFetchConfig};
```

若 `crates/cogito-tools/src/lib.rs` 有顶层 re-export(对照 `ReadFile` 的导出方式),照样补上 `WebFetch` / `WebFetchConfig`。

- [ ] **Step 5: 编译**

Run: `cargo build -p cogito-tools`
Expected: 通过(fetch 为桩)。

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/cogito-tools/Cargo.toml crates/cogito-tools/src/builtins/
git commit -m "feat(tools): web_fetch skeleton + reqwest/htmd deps"
```

---

### Task 9: web_fetch 抓取 + 内容处理 + 本地服务集成测试

**Files:**
- Modify: `crates/cogito-tools/src/builtins/web_fetch.rs`
- Create: `crates/cogito-tools/tests/web_fetch.rs`

- [ ] **Step 1: 写失败测试(本地 TcpListener 服务:html→md / 非文本拒绝 / scheme 拒绝)**

`crates/cogito-tools/tests/web_fetch.rs`:

```rust
//! Integration tests for `web_fetch` against a minimal local HTTP server
//! (raw TcpListener, no extra deps).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_protocol::ExecCtx;
use cogito_tools::builtins::web_fetch::{WebFetch, WebFetchConfig};
use cogito_tools::provider::BuiltinTool;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// Spawn a one-shot server that replies with a fixed `content_type` + body.
/// Returns the bound `http://127.0.0.1:<port>/` URL.
async fn serve_once(content_type: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // drain the request line/headers
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        }
    });
    format!("http://{addr}/")
}

fn ctx() -> ExecCtx {
    ExecCtx::open_ended(SessionId::new(), TurnId::new())
}

fn text_of(r: &ToolResult) -> String {
    match r {
        ToolResult::Text(s) => s.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[tokio::test]
async fn html_is_converted_to_markdown() {
    let url = serve_once("text/html; charset=utf-8", "<h1>Title</h1><p>Body text</p>").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    let md = text_of(&out);
    assert!(md.contains("Title"), "markdown should keep the heading text: {md:?}");
    assert!(md.contains("Body text"), "markdown should keep body: {md:?}");
    assert!(!md.contains("<h1>"), "raw HTML tags must be gone: {md:?}");
}

#[tokio::test]
async fn plain_text_passes_through() {
    let url = serve_once("text/plain", "hello world").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    assert!(text_of(&out).contains("hello world"));
}

#[tokio::test]
async fn binary_content_type_is_rejected() {
    let url = serve_once("image/png", "\x89PNG....").await;
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool.invoke(serde_json::json!({ "url": url }), ctx()).await;
    assert!(matches!(out, ToolResult::Error { kind: ToolErrorKind::InvocationFailed, .. }));
}

#[tokio::test]
async fn non_http_scheme_is_rejected() {
    let tool = WebFetch::new(WebFetchConfig::default());
    let out = tool
        .invoke(serde_json::json!({ "url": "file:///etc/passwd" }), ctx())
        .await;
    assert!(matches!(out, ToolResult::Error { kind: ToolErrorKind::InvalidArgs, .. }));
}
```

注:`text_of` 假设 `ToolResult::Text(String)` 变体。**实现前先确认 `ToolResult` 文本变体的确切形态**:

Run: `sed -n '85,120p' crates/cogito-protocol/src/tool.rs`
Expected: 看到 `ToolResult` 的变体与 `ToolResult::text(...)` 构造器。若文本变体名不是 `Text`,把测试里的 `text_of` 匹配臂改为实际变体(例如 `ToolResult::Text { text }` 或经 `Output`),保持与 `read_file` 返回 `ToolResult::text(...)` 的实际类型一致。

- [ ] **Step 2: 跑测试看失败**

Run: `cargo nextest run -p cogito-tools --test web_fetch`
Expected: FAIL(fetch 返回 "todo")。

- [ ] **Step 3: 实现 `fetch`**

把桩替换为(用 reqwest 异步 client + 流式读取限长 + content-type 分流 + htmd 转换):

```rust
    async fn fetch(&self, url: &str) -> ToolResult {
        use futures::StreamExt as _;

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_secs))
            .user_agent(self.cfg.user_agent.clone())
            .redirect(reqwest::redirect::Policy::limited(self.cfg.max_redirects))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("web_fetch: client build failed: {e}"),
                    retryable: false,
                };
            }
        };

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("web_fetch: request failed: {e}"),
                    retryable: true,
                };
            }
        };

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_html = content_type.contains("text/html");
        let is_text = content_type.starts_with("text/")
            || content_type.contains("json")
            || content_type.contains("xml");
        if !is_html && !is_text {
            return ToolResult::Error {
                kind: ToolErrorKind::InvocationFailed,
                message: format!("web_fetch: unsupported content-type: {content_type}"),
                retryable: false,
            };
        }

        // Read the body with a hard byte cap.
        let mut body: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    body.extend_from_slice(&bytes);
                    if body.len() >= self.cfg.max_bytes {
                        body.truncate(self.cfg.max_bytes);
                        break;
                    }
                }
                Err(e) => {
                    return ToolResult::Error {
                        kind: ToolErrorKind::InvocationFailed,
                        message: format!("web_fetch: body read failed: {e}"),
                        retryable: true,
                    };
                }
            }
        }
        let text = String::from_utf8_lossy(&body).to_string();

        if is_html {
            match htmd::convert(&text) {
                Ok(md) => ToolResult::text(md),
                Err(e) => ToolResult::Error {
                    kind: ToolErrorKind::InvocationFailed,
                    message: format!("web_fetch: html->markdown failed: {e}"),
                    retryable: false,
                },
            }
        } else {
            ToolResult::text(text)
        }
    }
```

`futures` 需在 `cogito-tools` 依赖中(确认 `[dependencies]` 有 `futures`;若无则加 `futures = { workspace = true }`)。`htmd::convert` 的确切签名以 docs.rs 为准:

Run: `cargo doc -p htmd --no-deps 2>/dev/null; echo '检查 https://docs.rs/htmd 顶层 convert 入口'`
Expected: 顶层函数 `htmd::convert(html: &str) -> Result<String, std::io::Error>`。若该版本入口是 `HtmlToMarkdown::builder().build().convert(&text)`,改用之。

- [ ] **Step 4: 跑测试看通过**

Run: `cargo nextest run -p cogito-tools --test web_fetch`
Expected: 4 个测试全绿。

- [ ] **Step 5: clippy + fmt**

Run: `make fix CRATE=cogito-tools && make fmt`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-tools/
git commit -m "feat(tools): web_fetch fetch + HTML->markdown + content-type routing"
```

---

## Phase 5 · 配置 [tools] 段(cogito-config)

### Task 10: ToolsConfig 聚合 + merge + finalize

**Files:**
- Modify: `crates/cogito-config/Cargo.toml`
- Modify: `crates/cogito-config/src/types.rs`
- Modify: `crates/cogito-config/src/merge.rs`

- [ ] **Step 1: 加依赖**

`crates/cogito-config/Cargo.toml` 的 `[dependencies]` 追加:

```toml
cogito-sandbox = { workspace = true }
cogito-jobs = { workspace = true }
cogito-tools = { workspace = true }
```

(这三个都不依赖 cogito-config,无循环;与现有 `cogito-model`/`cogito-mcp` 同模式。)

- [ ] **Step 2: 写失败测试(放在 `types.rs` 的 `mod tests`)**

在 `crates/cogito-config/src/types.rs` 的 `mod tests` 内追加:

```rust
    #[test]
    fn tools_section_parses_and_defaults() {
        let toml_str = r#"
            [tools.bash]
            sync_timeout_secs = 5

            [tools.sandbox]
            kind = "direct"
            root = "/work"
        "#;
        let partial: RuntimeConfigPartial = toml::from_str(toml_str).unwrap();
        let cfg = partial.finalize().unwrap();
        assert_eq!(cfg.tools.bash.sync_timeout_secs, 5);
        // web_fetch absent -> default.
        assert_eq!(cfg.tools.web_fetch.timeout_secs, 30);
        // sandbox root honored.
        let cogito_sandbox::SandboxConfig::Direct(d) = &cfg.tools.sandbox;
        assert_eq!(d.root, std::path::PathBuf::from("/work"));
    }

    #[test]
    fn tools_default_when_section_absent() {
        let partial: RuntimeConfigPartial = toml::from_str("[runtime]\nsession_root='/tmp/x'\n").unwrap();
        let cfg = partial.finalize().unwrap();
        assert_eq!(cfg.tools.bash.max_output_bytes, 32 * 1024);
    }
```

- [ ] **Step 3: 跑看失败**

Run: `cargo nextest run -p cogito-config tools_`
Expected: FAIL(`RuntimeConfig` 无 `tools` 字段,不编译)。

- [ ] **Step 4: 实现 ToolsConfig + 接线**

在 `types.rs` 顶部 imports 处补:

```rust
use cogito_jobs::BashConfig;
use cogito_sandbox::SandboxConfig;
use cogito_tools::builtins::web_fetch::WebFetchConfig;
```

新增类型(放在 `SkillsConfig` 附近):

```rust
/// `[tools]` cogito.toml section: aggregates per-tool config owned by the
/// implementing crates. Whole-section replace on merge (like `[skills]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// `bash` tool tunables (owned by `cogito-jobs`).
    pub bash: BashConfig,
    /// `web_fetch` tool tunables (owned by `cogito-tools`).
    pub web_fetch: WebFetchConfig,
    /// Command-execution backend selection (owned by `cogito-sandbox`).
    pub sandbox: SandboxConfig,
}
```

`RuntimeConfig` 加字段:

```rust
    /// Resolved `[tools]` section. Always present (defaults when omitted).
    pub tools: ToolsConfig,
```

`RuntimeConfigPartial` 加字段:

```rust
    /// Optional `[tools]` section. Whole-section replace on merge.
    pub tools: Option<ToolsConfig>,
```

`finalize()`(在 `merge.rs` 的 `Ok(RuntimeConfig { ... })` 里)加:

```rust
            tools: self.tools.unwrap_or_default(),
```

`merge.rs` 的 `merge_into` 内追加:

```rust
    if let Some(tools_next) = next.tools {
        acc.tools = Some(tools_next);
    }
```

同时修正 `types.rs` 测试里既有的 `RuntimeConfigPartial { ... }` 字面量(`partial_roundtrips_through_json` / `empty_partial_default_is_all_none`):给它们补 `tools: None,` 字段,并在 `empty_partial_default_is_all_none` 加 `assert!(p.tools.is_none());`。

- [ ] **Step 5: 跑看通过**

Run: `cargo nextest run -p cogito-config`
Expected: 新增 2 个 + 既有全绿。

- [ ] **Step 6: clippy + fmt + commit**

```bash
make fix CRATE=cogito-config && make fmt
git add crates/cogito-config/
git commit -m "feat(config): [tools] section aggregating bash/web_fetch/sandbox"
```

---

## Phase 6 · Surface 接线

### Task 11: CLI chat.rs 接线

**Files:**
- Modify: `crates/cogito-cli/Cargo.toml`
- Modify: `crates/cogito-cli/src/chat.rs`

- [ ] **Step 1: 依赖**

`crates/cogito-cli/Cargo.toml` `[dependencies]` 追加 `cogito-sandbox = { workspace = true }`(已有 cogito-jobs / cogito-tools)。

- [ ] **Step 2: 改 `build_tool_provider`**

在 `crates/cogito-cli/src/chat.rs` 顶部 use 区补:

```rust
use cogito_jobs::BashTool;
use cogito_protocol::job::LocalJobSubmitter;
```

把 `build_tool_provider` 里构建 `builtin` / `run_tests` / `local` 的段落(约 453-468 行)替换为:

```rust
    let executor = cogito_sandbox::build_executor(&cfg.tools.sandbox)
        .map_err(|e| anyhow!("build command executor: {e}"))?;

    let builtin: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .with_tool(Arc::new(cogito_tools::builtins::web_fetch::WebFetch::new(
                cfg.tools.web_fetch.clone(),
            )))
            .build(),
    );
    let run_tests: Arc<dyn cogito_protocol::tool::ToolProvider> =
        Arc::new(RunTestsTool::new(Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>));
    let bash: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        cfg.tools.bash.clone(),
    ));
    let local: Arc<dyn cogito_protocol::tool::ToolProvider> = Arc::new(
        CompositeToolProvider::new(vec![builtin, run_tests, bash], NamingPolicy::Strict)
            .map_err(|e| anyhow!("compose builtin + run_tests + bash: {e}"))?,
    );
```

(注意:`RunTestsTool::new` 原本接收 `job_mgr: Arc<LocalJobManager>`,这里改成显式 `Arc<dyn LocalJobSubmitter>`,因为 `job_mgr` 现在要被 bash 也克隆使用。`Arc<LocalJobManager>` → `Arc<dyn LocalJobSubmitter>` 自动 coerce。)

- [ ] **Step 3: 编译**

Run: `cargo build -p cogito-cli`
Expected: 通过。

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-cli/
git commit -m "feat(cli): wire web_fetch + bash (build_executor) into chat tools"
```

---

### Task 12: TUI runtime_build.rs 接线(镜像)

**Files:**
- Modify: `crates/cogito-tui/Cargo.toml`
- Modify: `crates/cogito-tui/src/runtime_build.rs`

- [ ] **Step 1: 依赖**

`crates/cogito-tui/Cargo.toml` `[dependencies]` 追加 `cogito-sandbox = { workspace = true }`。

- [ ] **Step 2: 改 `build_tools_with_banner`**

顶部 use 区补 `use cogito_jobs::BashTool;` 和 `use cogito_protocol::job::LocalJobSubmitter;`(若未引)。把构建 `builtin`/`run_tests`/`local` 段落(约 277-286 行)替换为与 Task 11 Step 2 等价的代码(同样三件:`build_executor`、`WebFetch` 进 builtin、`BashTool` 进 composite),变量类型用本文件已有的 `Arc<dyn ToolProvider>` 别名:

```rust
    let executor = cogito_sandbox::build_executor(&cfg.tools.sandbox)
        .map_err(|e| anyhow!("build command executor: {e}"))?;

    let builtin: Arc<dyn ToolProvider> = Arc::new(
        BuiltinToolProvider::builder()
            .with_tool(Arc::new(ReadFile))
            .with_tool(Arc::new(cogito_tools::builtins::web_fetch::WebFetch::new(
                cfg.tools.web_fetch.clone(),
            )))
            .build(),
    );
    let run_tests: Arc<dyn ToolProvider> =
        Arc::new(RunTestsTool::new(Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>));
    let bash: Arc<dyn ToolProvider> = Arc::new(BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        cfg.tools.bash.clone(),
    ));
    let local: Arc<dyn ToolProvider> = Arc::new(
        CompositeToolProvider::new(vec![builtin, run_tests, bash], NamingPolicy::Strict)
            .map_err(|e| anyhow!("compose builtin + run_tests + bash: {e}"))?,
    );
```

- [ ] **Step 3: 编译**

Run: `cargo build -p cogito-tui`
Expected: 通过。

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-tui/
git commit -m "feat(tui): wire web_fetch + bash into runtime tool build"
```

---

### Task 13: 端到端 e2e(MockModel 驱动 bash 通过 Runtime)

**Files:**
- Create: `crates/cogito-jobs/tests/bash_e2e.rs`

模板:`crates/cogito-jobs/tests/run_tests_happy_path.rs`(已存在,完整可参照)。

- [ ] **Step 1: 写 e2e 测试**

```rust
//! End-to-end: a model turn emits one `bash` tool_use; the turn runs the
//! command through the real DirectExecutor and completes. Mirrors
//! `run_tests_happy_path.rs` but with a synchronous bash command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use cogito_core::runtime::{OpenMode, Runtime};
use cogito_jobs::{BashConfig, BashTool, LocalJobManager};
use cogito_mock_model::MockModelGateway;
use cogito_protocol::command::CommandExecutor;
use cogito_protocol::event::{ConversationEvent, EventPayload};
use cogito_protocol::gateway::ModelGateway;
use cogito_protocol::ids::SessionId;
use cogito_protocol::job::{JobManager, LocalJobSubmitter};
use cogito_protocol::store::ConversationStore;
use cogito_protocol::strategy::HarnessStrategy;
use cogito_protocol::stream::StreamEvent;
use cogito_protocol::tool::{ToolProvider, ToolResult};
use cogito_sandbox::{DirectConfig, DirectExecutor};
use cogito_store_jsonl::JsonlStore;
use futures::StreamExt as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bash_echo_completes_through_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let store = Arc::new(JsonlStore::new(tmp.path().to_path_buf()));

    let job_mgr = LocalJobManager::new();
    let executor: Arc<dyn CommandExecutor> = Arc::new(DirectExecutor::new(DirectConfig::default()));
    let bash: Arc<dyn ToolProvider> = Arc::new(BashTool::new(
        executor,
        Arc::clone(&job_mgr) as Arc<dyn LocalJobSubmitter>,
        BashConfig::default(),
    ));

    let mock = Arc::new(MockModelGateway::new());
    mock.script_tool_then_text("bash", serde_json::json!({ "command": "echo e2e" }), "done");

    let runtime = Runtime::builder()
        .store(Arc::clone(&store) as Arc<dyn ConversationStore>)
        .model(Arc::clone(&mock) as Arc<dyn ModelGateway>)
        .tools(bash)
        .strategy(HarnessStrategy::default_with_model("mock"))
        .job_manager(Arc::clone(&job_mgr) as Arc<dyn JobManager>)
        .build()?;

    let session_id = SessionId::new();
    let handle = runtime.open_session(session_id, OpenMode::New).await?;
    let mut events = handle.subscribe();
    handle.submit_user_text("run echo").await?;

    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(StreamEvent::TurnCompleted) => return true,
                Ok(StreamEvent::TurnFailed { .. }) | Err(_) => return false,
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(completed, "expected TurnCompleted within 30s");

    handle.shutdown(Duration::from_secs(10)).await?;

    let log: Vec<ConversationEvent> = {
        let mut s = store.replay(session_id, 0);
        let mut out = Vec::new();
        while let Some(evt) = s.next().await {
            out.push(evt?);
        }
        out
    };
    let result = log
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResultRecorded { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("ToolResultRecorded missing");
    let ToolResult::Output(blocks) = result else {
        panic!("expected Output");
    };
    let stdout = blocks[0].get("stdout").and_then(serde_json::Value::as_str).unwrap_or("");
    assert!(stdout.contains("e2e"), "stdout={stdout:?}");
    Ok(())
}
```

确认 `crates/cogito-jobs/Cargo.toml` `[dev-dependencies]` 已含 `cogito-core`、`cogito-mock-model`、`cogito-store-jsonl`、`tempfile`、`futures`(run_tests 的 e2e 已用同一组,应已存在)。

- [ ] **Step 2: 跑 e2e**

Run: `cargo nextest run -p cogito-jobs --test bash_e2e`
Expected: 通过(TurnCompleted + stdout 含 `e2e`)。

- [ ] **Step 3: Commit**

```bash
git add crates/cogito-jobs/tests/bash_e2e.rs
git commit -m "test(jobs): bash end-to-end through Runtime with MockModel"
```

---

## Phase 7 · 文档与决策记录

### Task 14: ADR-0027 + 组件/配置文档

**Files:**
- Create: `docs/adr/0027-command-executor-seam-and-builtin-scope.md`
- Modify: `docs/components/H08-tool-dispatcher.md`
- Create: `docs/components/cogito-sandbox.md`
- Modify: `docs/configuration/overview.md`
- Modify: `ROADMAP.md`

- [ ] **Step 1: 写 ADR-0027**

内容直接取自 spec §1.1 / §2 / §3.1–3.5 / §11(两层模型 + spawn 点归属表 + sandbox 策略接缝 + builtin 做小哲学 + MCP/skill 已知边界)。状态 `Accepted 2026-05-29`。参照 `docs/adr/0026-strategy-registry.md` 的版式。

- [ ] **Step 2: 订正 H08 文档**

在 `docs/components/H08-tool-dispatcher.md` 的「v0.1 scope」节,把 `Sync (AlwaysSync) and async (AlwaysAsync) coverage; Adaptive deferred` 改为说明:dispatcher 自 Sprint 8 起按实际 `InvokeOutcome` 路由、`execution_class` 仅作 surface advisory,`Adaptive` 工具(首例 `bash`)开箱即用,无需 dispatcher 改动。补一句 `bash` 同步/background 双路径与 executor 注入在工具内部、H08 不可见。

- [ ] **Step 3: 写 sandbox 组件文档**

`docs/components/cogito-sandbox.md`:`CommandExecutor` 接缝定位、`DirectExecutor`(非安全边界)、`SandboxConfig`/`build_executor` 工厂、spawn 点归属表(引用 spec §3.2)、v0.4 ADR-0012/0013 演进。

- [ ] **Step 4: 更新配置 overview**

在 `docs/configuration/overview.md` 增加 `[tools]` 段(bash/web_fetch/sandbox 字段表 + 默认值),引用 spec §9 的 TOML 样例。

- [ ] **Step 5: ROADMAP 记一笔**

在 Sprint 10 节下加一条:bash + web_fetch 核心工具 + `cogito-sandbox` CommandExecutor 接缝(ADR-0027)为 Sprint 10 期间明示追加项(非原排期)。

- [ ] **Step 6: Commit**

```bash
git add docs/ ROADMAP.md
git commit -m "docs: ADR-0027 + sandbox component + [tools] config + H08 Adaptive correction"
```

---

### Task 15: 全量 CI + 收尾

- [ ] **Step 1: 全量 CI**

Run: `make ci`
Expected: fmt-check + clippy(pedantic + deny unwrap/expect/panic)+ layer-check(确认 cogito-config 新增的 Hands 依赖不违反 ADR-0004)+ 全 workspace 测试,全绿。

- [ ] **Step 2: 若 layer-check 报 cogito-config 依赖问题**

`make ci` 的 layer-check 若对 `cogito-config → cogito-sandbox/jobs/tools` 报警:确认它与既有 `cogito-config → cogito-model/mcp` 同级(cogito-config 是 wiring 聚合层,允许依赖 Hands 的 config 值类型)。若 layer-check 脚本维护一份显式白名单,把这三个依赖加入(脚本路径见 `make ci` 中 layer-check 步骤)。这属于预期内调整,不是设计违例。

- [ ] **Step 3: 最终提交(若 Step 2 改了白名单)**

```bash
git add -A
git commit -m "chore: allow cogito-config -> tool-owning crates in layer-check"
```

---

## Self-Review(已执行)

- **Spec 覆盖**:§2 sandbox 定位→Task 1/3/4;§3.1–3.5 两层模型/spawn 归属/H08/MCP/skill→Task 14(ADR+H08+sandbox doc);§4 CommandExecutor→Task 1;§5 DirectExecutor+工厂→Task 3/4;§6 bash Adaptive→Task 5/6/7;§7 web_fetch+htmd→Task 8/9;§8 安全(scheme 校验在 Task 8,命令/URL 准入交 hook 仅文档)→Task 8/14;§9 [tools] 配置→Task 10;§10 Surface→Task 11/12;§11 文档→Task 14;§12 测试(契约/单测/集成/e2e;chaos 不做)→Task 2/4/6/7/9/13;§13 排除项均未排进任务。
- **占位符**:无 TBD/TODO 残留(`invoke_sync`/`invoke_background`/`fetch` 的桩在同任务内即被替换,并标注)。
- **类型一致**:`CommandExecutor::run -> Result<CommandOutcome, CommandError>`、`CommandSpec{command,cwd,timeout,max_output_bytes}`、`BashConfig{sync_timeout_secs,background_deadline_secs,max_output_bytes}`、`WebFetchConfig{timeout_secs,max_bytes,user_agent,max_redirects}`、`SandboxConfig::Direct(DirectConfig{root,inherit_env})`、`ToolsConfig{bash,web_fetch,sandbox}` 在各任务间一致。
- **已知需实现时确认的两点**(已在对应步骤标注 Run 验证):(1)`ToolResult` 文本变体的确切形态(Task 9 Step 1);(2)`htmd::convert` 的确切入口(Task 9 Step 3)。

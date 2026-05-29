# 核心工具扩充:bash + web_fetch(含 sandbox 执行接缝)

- 状态:已批准(brainstorming 定稿,2026-05-29)
- 范围:v0.1,Sprint 10 期间的**明示追加项**(非原排期)
- 关联:ADR-0027(本设计配套新增)、ADR-0004(分层)、ADR-0012/0013(v0.4 sandbox/凭据,本设计为其预留接缝)、ADR-0018(MCP)、Sprint 8(异步 Job 语义)

## 1. 背景与目标

当前 builtin 工具只有 `read_file`(唯一的 `BuiltinTool`)与异步示范 `run_tests`(`cogito-jobs`)。本次为 agent 补齐两个核心工具:

- `bash`:任意 shell 命令执行(万能逃生口)。
- `web_fetch`:抓取 URL 并把 HTML 转成 markdown 供模型阅读。

参考 Manus / Codex / Claude Code 的实践,三家的核心工具收敛为:文件读 / 文件写编辑 / 检索 / shell / web 抓取 / web 搜索 / 子代理。本次只做 **bash + web_fetch**。

### 1.1 定位:builtin 工具集刻意做小

cogito 是可嵌入的 agent runtime 内核,哲学是"大脑只决策,手由 consumer 提供 / 走 MCP"。因此 builtin 工具集只装两类东西:

1. **每种执行模式的参考实现**,供 consumer 照着写自己的工具:
   - 同步(`BuiltinTool` → `ToolResult`):`read_file`,本次加 `web_fetch`(补"网络工具"参考实现)。
   - 异步 / 长任务(`ToolProvider` → `InvokeOutcome::Async` + `JobManager`):`run_tests`,本次加 `bash` 的 background 分支。
2. **任何 agent 都通用、且不依赖外部供应商选择的原语**:`bash`、`web_fetch`。

### 1.2 为什么本次不做 web_search

`web_search` 必须选定外部供应商(Brave / Tavily / Google / Bing / SerpAPI…),带 API key、计费、各家返回格式不同。这属于"consumer 带来的手"或 **MCP server** 的典型场景,而非 provider-free 原语。若内置须按 tagged-config 工厂(类似 `build_gateway`)实现,成本明显更高。**推迟,走 MCP 或后续工厂**。

### 1.3 为什么本次不先做 write/edit/grep

对纯编码 agent,编辑+检索其实比 web 更核心。但本次用户明确要 bash + web_fetch;且 bash 落地后 `sed`/`grep`/写文件可临时由 shell 兜底。结构化编辑/检索工具列为后续。

## 2. sandbox 定位(本设计的关键认知)

sandbox 是**可选的、策略驱动的隔离执行环境**,**不是任何 tool 的硬依赖**。tool 在 sandbox 开/关两种情况下都要能用。"是否进 sandbox"由部署形态/策略决定:

- **宿主机二进制**:用户可开可不开。关 = 直接在宿主跑;开 = 在隔离环境跑(cwd jail、资源限制、不污染主进程)。
- **SaaS 嵌入 ApiServer**:单进程多租户,不能随便在 API server 宿主 spawn 子进程。sandbox 须换成远程/每租户隔离实现,或直接禁用宿主命令执行。

因此 bash 不能硬依赖"某个具体 sandbox",而应依赖一个**执行抽象**,具体是否隔离由运行期注入的实现决定。

## 3. 架构总览

```
cogito-protocol
  └─ CommandExecutor (trait)            <-- 新增接缝(非序列化,不动 schema_version)
       CommandSpec / CommandOutcome

cogito-sandbox (Hands 内部原语)
  ├─ DirectExecutor: CommandExecutor    <-- v0.1 唯一实现(在宿主跑,非安全边界)
  ├─ SandboxConfig (值类型)
  └─ build_executor(&SandboxConfig) -> Arc<dyn CommandExecutor>   <-- tagged-config 工厂

cogito-jobs (Hands,异步工具之家)
  └─ BashTool: ToolProvider (Adaptive)  <-- 持 Arc<dyn CommandExecutor> + LocalJobSubmitter

cogito-tools (Hands,同步 builtins)
  └─ WebFetch: BuiltinTool              <-- reqwest + htmd

cogito-config
  └─ [tools] 段 (聚合 bash / web_fetch / sandbox 配置)

Surface (cogito-cli chat.rs / cogito-tui runtime_build.rs)
  └─ build_executor -> BashTool::new(...) + WebFetch -> CompositeToolProvider
```

依赖合规性(ADR-0004):`CommandExecutor` 在 protocol;bash/web_fetch 是 Hands;Brain 不直接看见 `cogito-sandbox`。`cogito-jobs` 新增对 `cogito-protocol`(trait)依赖即可拿到 `CommandExecutor`;executor 实例由 Surface 注入。

### 3.1 两层模型:Tool 抽象 vs CommandExecutor

cogito 里"tool"与"子进程执行"是**两个不同层次**,这是理解本设计边界的总纲:

- **Layer 1 · Tool 抽象(Brain 可见,H08 调度的对象)**:`ToolProvider` 是 Brain/H08 **唯一**看得见的东西。模型能调的一切都是 tool——builtin(`read_file`/`web_fetch`)、异步(`run_tests`/`bash`)、MCP(`mcp__server__tool`)、subagent。H08 只做 `provider.invoke(name,args,ctx) -> InvokeOutcome::{Sync,Async}` 然后驱动 FSM。**H08 不知道 CommandExecutor 的存在。**
- **Layer 2 · CommandExecutor(Hands 内部,tool 之下的子进程原语)**:"在策略选定的环境里跑一个子进程"。它在 ToolProvider 边界**之下**,对 Brain/H08 不可见。只有**需要 spawn 子进程**的 tool/子系统才用它;`bash` 在自己的 `invoke` 内部决定同步直跑还是 spawn job,H08 只看到最终的 `InvokeOutcome`。

### 3.2 各子进程 spawn 点的归属

| spawn 点 | 走 CommandExecutor? | 说明 |
|---|---|---|
| `bash` 工具 | **是**(本次设计) | 首位消费者 |
| `run_tests` | 否(当前 raw `tokio::process`) | 已绿代码;收敛去重留后续可选项 |
| MCP stdio transport | 否(rmcp 内部,connect 时一次性 spawn) | 非 per-call;见 §3.4 |
| skill 脚本(今天) | **是**(经 `bash`) | ADR-0023 B-defer:脚本是数据,模型用 bash 跑 |
| skill 脚本(未来 ADR-0023 B/C) | 应当是 | 落地时 funnel through;见 §3.5 |

`CommandExecutor` 是 cogito 自身发起的子进程执行的**预期单一漏斗**。v0.1 仅 `bash` 接入;其余为已知现状/未来项,逐条说明如下。

### 3.3 H08 Tool Dispatcher 如何使用(含 Adaptive 现状订正)

- H08 是 Brain 中**唯一**调用 Hands 的组件。流程:H09 `pre_dispatch` hook(可 `Reject`)→ `catch_unwind(provider.invoke)` → `InvokeOutcome::Sync` 出 `DispatchOutcome::SyncResult`;`Async` 则先记 `JobSubmitted`、再注册 `on_complete` sink、转 `TurnState::Paused`,在 `JobCompletionEvent` 到来时恢复。
- **CommandExecutor 对 H08 不可见。**
- **Adaptive 已被支持,无需额外打通**:`dispatcher.rs` 自 Sprint 8 起按**实际返回的** `InvokeOutcome` 路由,不再用 `execution_class` 校验返回类型——descriptor 上的 `ExecutionClass` "现在纯粹是给 surface(如 H05 过滤)的 advisory"。因此 Adaptive 工具(按参数返回 Sync 或 Async)开箱即跑。`bash` 是**首个真 Adaptive 工具**。`docs/components/H08-tool-dispatcher.md` 里"v0.1 scope: Adaptive deferred"为 Sprint 8 之前的**陈旧表述,本次订正**。

### 3.4 MCP tools 与 CommandExecutor

- MCP 的 **tool 调用**(`tools/call`)是把 JSON-RPC 消息发给一个**已建立的 transport**,**不 spawn 任何进程**。所以 MCP tool invocation 与 CommandExecutor 无关。
- 但 MCP **stdio server** 的进程在 **connect 时一次性 spawn**,且该 spawn 发生在 **rmcp 的 `transport-child-process` 内部**(cogito-mcp 仅把 `command`/`args` 交给 rmcp)。**它不经过 CommandExecutor。**
- **含义(尤其 SaaS)**:CommandExecutor 接缝**不覆盖** MCP stdio 子进程。ApiServer 多租户下,要么只用 streamable-HTTP MCP,要么后续用专门 ADR 把 rmcp 的 command spawn 也包到 executor 之后。**已知边界,本设计仅记录、不在 v0.1 解决。**

### 3.5 skill 脚本执行的归属

- **今天(ADR-0023 = B-defer)**:skill 的 `scripts/` 只是磁盘数据,loader **不执行**;模型用 `read_file` 读、用 `bash` 跑——即 skill 脚本执行**今天已天然走 CommandExecutor**(经 bash),无特殊机制。
- **未来(ADR-0023 Position B 构建期内联 / Position C 脚本即 tool)**:真正"主动执行脚本"时,按同一接缝原则**应 funnel through CommandExecutor**。本设计为 ADR-0023 未决项提供这一落点。

## 4. Protocol:CommandExecutor 接缝

新增于 `cogito-protocol`(运行期 trait,不进事件日志、不参与跨语言 wire,故**不动 `SCHEMA_VERSION`**):

```rust
/// 执行一条命令的抽象。是否隔离由具体实现决定;运行期注入。
/// v0.4 ADR-0012 的 sandbox 生命周期 / ADR-0013 凭据隔离从此接入。
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn run(&self, spec: CommandSpec, ctx: ExecCtx) -> CommandOutcome;
}

pub struct CommandSpec {
    /// 待执行的 shell 命令行(由实现决定如何起 shell,DirectExecutor 用 `sh -c`)。
    pub command: String,
    /// 工作目录。相对则相对于实现持有的 root;None = 用 root。
    pub cwd: Option<PathBuf>,
    /// 本次执行的硬超时。
    pub timeout: Duration,
    /// stdout/stderr 各自保留的字节上限(头尾截断)。
    pub max_output_bytes: usize,
}

pub struct CommandOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,  // 被信号杀死则 None
    pub timed_out: bool,
    pub truncated: bool,
}
```

注:env 策略与 root 是**实现/构造期**关注点(放 `SandboxConfig`),不放进每次调用的 `CommandSpec`,保持调用面最小。

## 5. cogito-sandbox:DirectExecutor + 工厂

把目前的空壳立起来。

### 5.1 DirectExecutor

- 用 `tokio::process::Command` 起 `sh -c <command>`(linux 目标;Windows 留 TODO)。
- cwd = `spec.cwd`(相对则拼到构造期 root)或 root。
- env 策略来自 `SandboxConfig`(v0.1:继承父进程 env;预留 `clear` 开关)。
- piped stdout/stderr,后台并发抽取(复用 `run_tests` 已验证的范式)。
- `child.wait()` 与 `ctx.cancel.cancelled()`、`spec.timeout` 三方竞速;超时/取消则 kill 子进程,`timed_out` 置位。
- 输出头尾各 `max_output_bytes` 截断,带省略标记,`truncated` 置位。
- **非安全边界**(与现有模块 doc 一致):无 namespace/seccomp/chroot。

### 5.2 SandboxConfig + 工厂

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxConfig {
    Direct(DirectConfig),   // v0.1 唯一 tag
    // 未来:LocalJail / Remote(v0.4 ADR-0012)
}

pub struct DirectConfig {
    pub root: PathBuf,             // 默认 = 进程 cwd
    pub inherit_env: bool,         // 默认 true
}

pub fn build_executor(cfg: &SandboxConfig) -> Result<Arc<dyn CommandExecutor>, SandboxError>;
```

遵循 CLAUDE.md tagged-config 工厂规约:`match`-on-tag 落在属主 crate(`cogito-sandbox`),Surface 只调 `build_executor`。

## 6. bash 工具(cogito-jobs,Adaptive)

Adaptive 不能用 `BuiltinTool`(那是 sync-only),故直接实现 `ToolProvider`,与 `RunTestsTool`/`SleepTool` 同列。

### 6.1 构造与依赖

```rust
pub struct BashTool {
    executor: Arc<dyn CommandExecutor>,
    submitter: Arc<dyn LocalJobSubmitter>,   // background 分支用
    cfg: BashConfig,                          // sync_timeout / background_deadline / max_output_bytes
}
impl BashTool { pub fn new(executor, submitter, cfg) -> Self }
```

### 6.2 模型可见 schema

```json
{
  "type": "object",
  "properties": {
    "command":      { "type": "string",  "description": "Shell command to run via `sh -c`." },
    "background":   { "type": "boolean", "description": "Run as a background job; the turn pauses and resumes on completion. Use for long-running commands." },
    "cwd":          { "type": "string",  "description": "Working dir relative to the workspace root (or absolute)." },
    "timeout_secs": { "type": "number",  "description": "Override the synchronous timeout." }
  },
  "required": ["command"],
  "additionalProperties": false
}
```

`execution_class = ExecutionClass::Adaptive`。

### 6.3 调度语义

- `background == true` → 提交 Job,返回 `InvokeOutcome::Async(JobId)`;Job 内部调 `executor.run(spec_with(background_deadline))`,完成时以 `JobOutcome` 交付 `{stdout, stderr, exit_code}`(turn 暂停/恢复沿用 Sprint 8)。
- 否则 → `InvokeOutcome::Sync`:`executor.run(spec_with(sync_timeout))` await。
  - `timed_out` → `ToolResult::Error{ kind: Timeout, retryable: true, message: "command timed out; pass background:true for long-running commands" }`。
  - 否则 → `ToolResult::text`/`Output`,带 `exit_code`。`exit_code != 0` **不**视为工具错误(命令非零退出是正常业务信息,原样返回给模型)。
- 两条路都经同一个 `CommandExecutor`,故 sandbox 开关对二者一致生效。

### 6.4 范围诚实声明

v0.1 的 background = "异步长任务,完成时一次性交付结果"。**不是**可中途轮询/查看输出的分离守护进程(如 `npm run dev` 持续输出)。后者需 v1.x 的 Resource Registry(P4)。background 命令仍受 `background_deadline` 约束。

## 7. web_fetch 工具(cogito-tools,同步 BuiltinTool)

### 7.1 依赖

- `reqwest`(workspace 已有,rustls)。
- `htmd`(**新依赖,已批准**):HTML → Markdown,纯 Rust。需加入 `[workspace.dependencies]`。

### 7.2 schema

```json
{ "type": "object",
  "properties": { "url": { "type": "string", "description": "http(s) URL to fetch." } },
  "required": ["url"], "additionalProperties": false }
```

`execution_class = ExecutionClass::AlwaysSync`。

### 7.3 行为

- scheme 非 `http`/`https` → `ToolResult::Error{InvalidArgs}`。
- `GET`,跟随重定向(上限 `max_redirects`),`reqwest::Client` 带 `timeout` 与 `user_agent`(均来自 config)。
- 按 `max_bytes` 限制响应体(流式读取并截断)。
- 按 `Content-Type` 分流:
  - `text/html` → `htmd` 转 markdown 返回(带是否截断标记)。
  - 其他 `text/*`(含 json/plain) → 原样文本(截断)。
  - 其他(二进制) → `ToolResult::Error{InvocationFailed, "unsupported content-type: <ct>"}`(图片等待 v0.5 多模态)。
- **不调模型**(保持 provider-free,避免耦合 `ModelGateway` / 破坏分层)。

## 8. 安全定位

- 工具自身只做轻量护栏:web 的 scheme 白名单、两者的大小/超时上限。
- **命令准入(挡 `rm -rf /`)与 URL 准入(挡内网 IP / SSRF)= H09 hook 职责**。已有 bash audit hook 范式;web_fetch 的 URL 可由同类策略 hook 拦截。
- v0.1 明确**不是 security boundary**;真正隔离/凭据边界为 v0.4(ADR-0012/0013)。

## 9. 配置([tools] 段)

值类型放属主 crate,`cogito-config::RuntimeConfig` 聚合 + partial/merge/finalize(与 `providers`/`mcp_servers` 同模式)。

```toml
[tools.bash]
sync_timeout_secs    = 30
background_deadline_secs = 600
max_output_bytes     = 32768   # 头尾各

[tools.web_fetch]
timeout_secs  = 30
max_bytes     = 1048576
user_agent    = "cogito/0.1"
max_redirects = 5

[tools.sandbox]
kind = "direct"
root = "."          # 默认进程 cwd
inherit_env = true
```

- `BashConfig`/`WebFetchConfig` 放属主 crate(`cogito-jobs`/`cogito-tools`),`SandboxConfig` 放 `cogito-sandbox`。
- `RuntimeConfig` 新增 `tools: ToolsConfig` 字段;`RuntimeConfigPartial` 加可选 `[tools]`;merge 逐字段覆盖;finalize 给默认值。

## 10. Surface 接线

`cogito-cli/src/chat.rs` 与 `cogito-tui/src/runtime_build.rs` 现状:`BuiltinToolProvider(read_file)` + `RunTestsTool(job_mgr)` 经 `CompositeToolProvider` 组合。改为:

1. `let executor = cogito_sandbox::build_executor(&cfg.tools.sandbox)?;`
2. `BuiltinToolProvider::builder().with_tool(Arc::new(ReadFile)).with_tool(Arc::new(WebFetch::new(cfg.tools.web_fetch)?)).build()`
3. `let bash = Arc::new(BashTool::new(executor, job_mgr.clone(), cfg.tools.bash));`
4. `CompositeToolProvider::new(vec![builtin, run_tests, bash], NamingPolicy::Strict)`(再叠 MCP)。

两个 Surface 改动对称。`build_executor` 调用在 Surface,符合"工厂分发在属主 crate、Surface 只调一次"。

## 11. 文档与决策记录

- **ADR-0027**(新增):核心内容含 (a) **Tool/CommandExecutor 两层模型**(§3.1)与子进程 spawn 点归属表(§3.2);(b) sandbox 作为**策略选择**的 `CommandExecutor` 接缝、v0.4 演进;(c) builtin 工具集做小哲学(reference impl + provider-free 原语;web_search 推 MCP;web_fetch 不调模型的分层理由);(d) **已知边界**:MCP stdio 子进程不经 CommandExecutor(§3.4)、skill 脚本执行归属(§3.5)。
- 更新 `docs/components/H08-tool-dispatcher.md`:**订正**"v0.1 scope: Adaptive deferred"陈旧表述(dispatcher 自 Sprint 8 起按实际 `InvokeOutcome` 路由、`execution_class` 仅 advisory,bash 为首个 Adaptive 工具,无需改 dispatcher 代码);补 bash 同步/background 双路径说明。executor 注入在工具内部,H08 不涉及。
- 更新 `docs/configuration/overview.md`(新增 `[tools]` 段)。
- 新增 `docs/components/cogito-sandbox.md`(CommandExecutor 接缝 + DirectExecutor + 工厂 + v0.4 演进)。
- `ROADMAP.md` Sprint 10 记一笔:本项为 Sprint 10 期间明示追加,非原排期;完成后在 `docs/experiments/` 补实验报告。

## 12. 测试

- `CommandExecutor` 契约测试(共享,所有实现须过):成功退出码、非零退出码、超时置位、cancel 杀进程、输出截断。
- `DirectExecutor` 单测(覆盖上述,真实 `sh -c`)。
- `BashTool`:同步成功、同步超时→Timeout、background→Async 完成路径、cancel、非零退出码原样返回、schema 校验拒绝。
- `WebFetch`:html→markdown、字节截断、scheme 拒绝、非文本 content-type 拒绝、超时(用本地测试 server / mockito)。
- CLI 端到端:`bash` 跑通一条命令、`web_fetch` 抓一个本地页面各一条。
- **不做**:resume-chaos `paused_bash_job` 场景(background bash 崩溃恢复)——价值真实但放后续小 PR,控制本次范围。

## 13. 显式排除(本次不做)

- web_search(走 MCP / 后续工厂)。
- write_file / edit_file / grep / glob(后续核心工具补齐)。
- sandbox 真隔离(namespace/seccomp/chroot/远程)→ v0.4 ADR-0012。
- background bash 的可轮询分离守护进程 → v1.x Resource Registry。
- RunTestsTool 收敛到 CommandExecutor(可选后续去重,本次不动已绿代码)。
- resume-chaos 新场景(后续小 PR)。
- executor 移入 ExecCtx 做每租户选择 → v0.4 随 ExecCtx.tenant。

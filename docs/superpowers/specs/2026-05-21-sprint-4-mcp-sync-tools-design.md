# Sprint 4 · MCP sync tools — 设计 Spec

> **Status**: Draft (2026-05-21) — pending ADR-0018 + plan
> **Sprint**: v0.1 · Sprint 4 (replaces 旧 Sprint 4 "Async Jobs",
> 后者顺延至 Sprint 5;见 2026-05-21 ROADMAP renumber)
> **Authors**: qiannengsheng + AI brainstorm partner
>
> 本文件是 Sprint 4 的**决策讨论轨迹**。可执行契约住在 durable 文档:
> [`ADR-0018`](../../adr/0018-mcp-integration.md)(待写)锁架构 + 兼容
> 性 + 许可证立场;`cogito-mcp` crate 的 module/struct/trait doc-comment
> 锁 API 细节。此 spec 解释 **why** 和 **实施分解**;ADR-0018 与 crate 文档
> 定义 **what**。

---

## 1 · Sprint 目标

让 Brain 第一次拥有一个**多源、真实、丰富**的 sync-tool 目录:除了内
置的 `read_file`,还能挂载任意 MCP server(stdio 子进程 *或* streamable-
HTTP 端点),把它们的工具透明融入 `ToolProvider` 抽象,让 H05 Tool
Surface Builder / H07 Tool Resolver / H08 Tool Dispatcher 不感知 MCP
这层存在。

这个能力**提前**了 v0.2 原计划的 `cogito-mcp`,理由是:

1. 单个 builtin(`read_file`)无法证明 Brain 在多工具、变化的 prompt
   形状下的鲁棒性;有 MCP 后团队能即时把任意工具拼进来测。
2. Async-job 基础设施(旧 Sprint 4)是大件事(`JobManager` + JSONL
   job log + 跨进程 resume + H08 async path),与 MCP 解耦;先 MCP
   后 Async 让并行验证 Brain 能力的窗口提前 1.5–2 天。
3. ADR-0017 刚落地的 layered-merge 配置基础设施,正好为
   `[[mcp_servers]]` 提供天然接入点。

### 1.1 In-scope

1. **新 crate `cogito-mcp`**(原为占位空 crate)。依赖 `rmcp = "1.5"`
   (modelcontextprotocol/rust-sdk,Apache-2.0,普通 upstream)+
   `reqwest` + `tokio` + `thiserror` + `serde`。Features 全开启
   `client` / `transport-child-process` / `transport-streamable-http-client-reqwest`
   / `schemars` / `macros`。
2. **`McpToolProvider`** 实现 `cogito_protocol::tool::ToolProvider`:
   `list()` 聚合所有已连接服务器的工具并返回 qualified `ToolDescriptor`;
   `invoke()` 按 qualified name 路由到对应服务器、调用 `tools/call`。
3. **`cogito-config`** 扩展:`RuntimeConfigPartial` 增 `mcp_servers:
   Option<Vec<McpServerConfig>>` 字段;`McpServerConfig` 是 tagged-enum
   (`transport = "stdio" | "streamable_http"`),复刻 ADR-0017
   `ProviderConfig` 的 tagged-config-factory 模式(CLAUDE.md §"Tagged-
   config factories")。
4. **`cogito-mcp::build_mcp_provider`** 工厂(类比 `cogito-model::
   build_gateway`):吃 `&[McpServerConfig]` → 返回 `Arc<dyn ToolProvider>`
   (或 `None` 当 list 为空,以避免空 provider 污染 composite)。
5. **`cogito-cli chat`** 接入:从 `RuntimeConfig.mcp_servers` 构造
   `McpToolProvider`;与 builtin `BuiltinToolProvider` 一起塞进
   `CompositeToolProvider`(`Strict` 模式,因为 MCP qualified name
   `mcp__server__tool` 与 builtin name 必然不冲突)。
6. **Tool naming**:`mcp__<server>__<tool>` qualified format,sanitize
   非法字符为 `_`,64 字符 cap 后用 SHA-1 前缀截断,server-内冲突 warn+skip。
   抄 Codex `mcp_connection_manager.rs` 的算法,**pattern-only 重写**,
   不复制源码。
7. **Streamable-HTTP transport + bearer 认证**:从 `bearer_token_env_var`
   字段读 env,作为 `Authorization: Bearer <value>` 头注入 reqwest
   client。环境变量缺失 fail-loud(返回 startup 错误,不静默)。
8. **Stdio transport**:子进程 spawn,`kill_on_drop(true)`,`env_clear()`
   + 明确白名单,stderr 接到 tracing(参考 Codex 的做法)。
9. **Eager handshake**:Runtime 启动时连接全部已配置 server,完成
   `initialize` + `tools/list`。失败 server warn+skip(不阻塞 Runtime
   构造),其工具不出现在目录。`startup_timeout_sec` 默认 10s。
10. **Per-server `enabled_tools` / `disabled_tools` 列表**:精确名字
    匹配(server 内 raw 名,非 qualified 名),allowlist 先于 denylist
    应用。
11. **Tool call timeout**:每个 server 配 `tool_timeout_sec`(默认
    60s),与 `ExecCtx::deadline` 取较小值。
12. **测试**:单元(naming 规则、config deserialization、tool result
    mapping)+ 集成(rmcp server-side feature 起 mock server,exercise
    list+call 全链路)+ E2E(对用户提供的 streamable-HTTP 服务,带
    bearer,跑 `cogito chat` 端到端,手动 + scripted)。
13. **ADR-0018**:transport scope、namespacing、许可证立场、deferred
    OAuth、failed-server fault containment 决策。
14. **README + `docs/configuration/overview.md`** 加 MCP 配置段;H05
    doc 注脚 MCP-provided tools 透明性;H07 doc 注脚 MCP 工具的
    schema 直接 forward 不再二次校验。
15. **CHANGELOG** Sprint 4 entry。

### 1.2 Out-of-scope(明确不做,避免 scope creep)

| 不做的事 | 何时做 |
|---|---|
| OAuth login flow(`rmcp` 提供,Codex 用了 922L `oauth.rs` 实现) | 单独 ADR(post-Sprint 4),独立工作 |
| Legacy SSE-only transport(MCP 2025-03-26 已废弃) | 永不(`rmcp` 1.5 不提供) |
| MCP **resources** API(`resources/list`、`resources/read`) | v0.2 storage ADR(对接 `StorageSystem`) |
| MCP **prompts** API | post-v0.2(本质是 strategy 一种,后续讨论) |
| MCP **elicitation**(server → client 反向请求) | 等 Brain UX 需要时(无明确驱动) |
| MCP **sampling**(server → client 反向 LLM 调用) | 显示禁用(违反 ADR-0004 边界:Hand 不能调用 Brain) |
| Tool 名 wildcard / regex 过滤 | 永不(精确名字够用;wildcard 容易破坏可预测性) |
| 跨 server 工具去重(全局唯一性) | 永不(`mcp__server__tool` 天然 server 隔离) |
| MCP server hot reload | 永不(同 ADR-0017 §13;进程重启接入新配置) |
| 工具调用结果的 ContentBlock multimodal 完整支持 | v0.2 multimedia ADR(image/audio block 透传) |
| MCP server 自动重连 | v0.1 不做;断连即 fault-skip,下次进程重启重连 |
| 子进程 sandbox/seccomp | 由调用方做(`cogito-sandbox` v0.4+) |

### 1.3 与旧 Sprint 4(现 Sprint 5)的关系

Async Jobs 工作整体顺延至 Sprint 5。但 `cogito-protocol::tool::
ExecutionClass` 已经预留了 `AlwaysSync`/`AlwaysAsync`/`Adaptive` 三类。
MCP 工具一律映射到 **`AlwaysSync`** —— `rmcp` 的 `tools/call` 是请求
-响应模式,概念上是 sync(尽管底层 HTTP 可能是 streaming);H08 不会
对 MCP 工具发出 `InvokeOutcome::Async`,所以 Sprint 4 不依赖未实现的
H08 async path,也不阻塞 Sprint 5 的 async 工作。

---

## 2 · 决策轨迹(Q1–Q13)

完整 architectural 论证写在 ADR-0018。本节只串关键 fork、不重复论证。

### Q1 · 许可证立场:`rmcp` vs Codex `rmcp-client`

**用 `rmcp`(crates.io 1.5,Apache-2.0,modelcontextprotocol/rust-sdk
官方),不用 Codex 的 `codex-rmcp-client`**。

理由:
- `rmcp` 是普通 upstream dep,加 `Cargo.toml` 等同 `serde`/`tokio`,
  Apache-2.0 自动从 crates.io 元数据 attribution,**无衍生作品负担**。
- Codex(openai/codex,整体 Apache-2.0)的 `rmcp-client` crate 535L
  虽然结构清晰,**抄它的源代码**会让 cogito 整体卷入 Apache-2.0 的
  retention 义务。我们要**模式启发**(state machine 形状、namespacing
  规则、transport 枚举),**不要源代码 lift**。在
  `crates/cogito-mcp/src/lib.rs` 头部和 ADR-0018 References 注明
  "architecture-inspired by openai/codex-rmcp-client (Apache-2.0,
  pattern-only reimplementation)",那是专业 credit。
- 不重新发明:`mcp-types` 这种"在 rmcp 模型之外再造一层中间表示"的
  做法(Codex 0.x 时遗留)我们 greenfield 不需要,直接吃 rmcp 的
  `Tool` / `CallToolResult` 等,在 `cogito-mcp` 边界一次性映射成
  cogito 的 `ToolDescriptor` / `ToolResult`,省一大堆 conversion 代码。

### Q2 · Transport scope

**stdio + streamable-HTTP 同时 v0.1 必须支持**。

streamable-HTTP 是因为用户的 MCP 服务就是这个形式(bearer 认证),不支
持就业务卡死;stdio 是因为绝大多数开源 MCP server(filesystem、git、
shell、playwright 等)都走 stdio,本地开发 + 自动化测试都要它。

SSE 旧 transport 不在 1.5 spec 范围(2025-03-26 起统一到 streamable-
HTTP),`rmcp` 也只暴露 `transport-streamable-http-client-reqwest`,没
有歧义。

OAuth 单独切出,见 Out-of-scope。

### Q3 · 客户端 SDK

**直接用 `rmcp = "1.5"` + 启用 features**: `client`, `transport-child-process`,
`transport-streamable-http-client-reqwest`, `schemars`, `macros`。
不开 `auth`(OAuth 推迟)、不开 `server`(只做客户端)。

### Q4 · Tool namespacing

**`mcp__<server>__<tool>`**,完全采用 Codex 的 convention(算法是公共
知识,非源码;namespacing 规则本身就是 MCP 多 server 部署的事实标准)。

- 分隔符 `__`(双下划线):受 OpenAI Responses API tool name 正则
  `^[a-zA-Z0-9_-]+$` 约束,这是兼容性最广的安全字符集。
- Sanitize:非法字符 → `_`。
- 长度上限 64 字符,超长用 SHA-1 hash 前缀替换尾部以保确定性。
- 同名(qualified 后)duplicate → warn + skip 后来者。
- **Builtin 工具反向保证不冲突**:cogito 内置工具名禁止以 `mcp__` 开
  头(Sprint 4 在 `BuiltinToolProvider::add_tool` 加一行 debug-assert
  防呆,文档化此约定)。

由于 qualified name 在 `McpToolProvider` 内部就完成,挂到
`CompositeToolProvider` 时用 `NamingPolicy::Strict` 而**不**用
`Prefixed("mcp/")`—— 否则双重前缀且分隔符不一致。

### Q5 · Lifecycle:eager 还是 lazy 连接?

**Eager**:Runtime 构造时连接所有配置的 server,完成 `initialize` +
`tools/list`。

理由:
- H05 Tool Surface Builder **每个 turn 调一次 `provider.list()`**。
  Lazy 模式下首次 turn 抖动(handshake 几百 ms~几秒),debug 体验
  极差。
- Eager 模式下 startup 阶段一次性吃掉所有失败,Runtime 启动日志清晰。
- 成本:Runtime 构造从纯本地变成"有网 I/O";对长驻 SaaS 进程影响
  可忽略,对 CLI 一次性命令(`cogito chat`)增加 startup 延迟,
  通过 `startup_timeout_sec` 默认 10s + 并发握手缓解。

并发:所有 server 用 `tokio::task::JoinSet` 并行 init,最长那个决定
整体 startup time。

### Q6 · 单 server 故障 → 影响整体?

**Per-server fault containment**:某个 server 起不来 → warn-log +
skip,Runtime **继续构造**,其工具简单地不在目录里。

理由:dev 体验。常见场景:某个 MCP server 二进制临时缺、网络抖动、
bearer token 过期。整体阻塞太硬。

副作用:Brain 看到的 tool catalog 可能与配置文件预期不一致,但这是
透明的 —— H10 strategy 的 `allowed_tools` 在工具不存在时本就会 warn,
现有路径吸收即可。

Hard-fail mode(配置错误 → 启动失败)留给 v0.4 SaaS-ready 时引入
`strict_mcp_startup: bool` 字段,v0.1 不暴露。

### Q7 · MCP `CallToolResult` → cogito `ToolResult` 映射

MCP `CallToolResult` 形状:
```text
{ content: Vec<ContentBlock>, is_error: bool, structured_content?: Value }
```
ContentBlock variants:`Text { text }` / `Image { data, mime_type }` /
`Resource { uri, ... }` / `Audio`(spec 2025+)。

cogito v0.1 `ToolResult`:`Output(Vec<serde_json::Value>)` 或
`Error { kind, message, retryable }`。

**v0.1 映射规则**:

| MCP 输出 | cogito 映射 |
|---|---|
| `is_error=true` | `ToolResult::Error { kind: InvocationFailed, message: <text blocks 拼接>, retryable: false }` |
| `is_error=false`,全 `Text` | `ToolResult::Output(vec![Value::String(text)])` 每块一项 |
| `is_error=false`,含 `Image`/`Resource` | 同 Output,但 image/resource 块序列化为 `{ "kind": "image", "uri/data": ... }` JSON object;**模型暂时看不到图像** —— v0.2 multimedia ADR 切换到 `Vec<ContentBlock>` 后才完整透传 |
| `is_error=false`,含 `structured_content` | 该字段作为 vec 末尾追加项:`Value::Object({"kind": "structured", "data": ...})` |

`retryable: false` 是保守默认 —— 我们对 MCP server 的内部状态一无所
知,不知道某个错误能不能重试。日后可在 `ToolErrorKind` 加
`McpServerError`(`ToolErrorKind` 已 `#[non_exhaustive]`)细分。

### Q8 · `[[mcp_servers]]` 配置 schema

```toml
# stdio 例
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "uvx"
args = ["mcp-server-filesystem", "/tmp"]
env = { LOG_LEVEL = "info" }            # 可选
startup_timeout_sec = 10                # 可选
tool_timeout_sec = 60                   # 可选
enabled_tools = ["read_file", "search"] # 可选 allowlist
disabled_tools = []                     # 可选 denylist

# streamable-HTTP 例(用户的生产环境)
[[mcp_servers]]
name = "company_api"
transport = "streamable_http"
url = "https://mcp.example.com/v1"
bearer_token_env_var = "COMPANY_MCP_TOKEN"
http_headers = { "X-Tenant" = "acme" }  # 可选 静态头
tool_timeout_sec = 30
```

设计要点:
- **`transport` 字段显式 tag**(不用 Codex 的"字段存在隐式推断"):
  对齐 `ProviderConfig` 的 `kind` tag 模式,deserialize 时报错信号清
  晰,工厂内 `match` 单点完整。
- **`name` 字段必填且全局唯一**:进入 qualified name 时必须确定。
- **bearer secret 不进文件**:只接受 `bearer_token_env_var`(env 变
  量名),禁止 `bearer_token`(明文)字段。这与 ADR-0017 §6 的 secret
  posture 一致 —— secrets 来自 env,文件只占位。
- `${VAR}` 字符串插值在 `cogito-config::FileConfigLoader` 已实现,
  `mcp_servers` 段自动享有(`url`、`http_headers` value 都可包含
  `${VAR}`)。
- `enabled_tools`/`disabled_tools` 用 **server-内 raw 名**,不是
  qualified 名 —— 用户写配置时不需要预知 sanitize 后的样子。
  应用顺序:先 enabled_tools 过滤(若设置),再 disabled_tools 删除。
- 未知字段 fail-loud(`deny_unknown_fields` 在 `McpServerConfig` 上):
  对齐 `cogito-config` 内层 struct 既有策略。
- 反向:`#[serde(default)]` 在 top-level `RuntimeConfigPartial`,`mcp_servers`
  整段缺失合法,默认空 vec。

### Q9 · cogito-config 集成深度

`cogito-config` **不需要**直接 import `cogito-mcp`(避免循环依赖)。
`McpServerConfig` value type 住在 **`cogito-mcp::config`**,
`cogito-config::RuntimeConfigPartial` 通过 `Vec<McpServerConfig>`
**dep on cogito-mcp**(向上依赖,Brain → Hands 方向倒过来,所以
`cogito-config` 是 Surface-邻接层,加 cogito-mcp 不违反 ADR-0004 层
规)。

Layer check(scripts/check-layer.sh)预期不会报错;额外加一行规则
覆盖。

### Q10 · 工厂函数放哪个 crate?

`cogito-mcp::build_mcp_provider(cfgs: &[McpServerConfig]) -> Result<Option<Arc<dyn ToolProvider>>, McpError>`。

CLAUDE.md §"Tagged-config factories"明确:`transport` tag → 工厂调度
留在 `cogito-mcp`,Surface 调用一次拿 trait object,不在 CLI 里 fork
`match`。返回 `Option` 是因为 list 空时不构造空 provider(让
`CompositeToolProvider` 不需要处理空 children case)。

### Q11 · Cancellation + timeout

三层:
1. **Startup timeout(per-server)**:`startup_timeout_sec` 包裹
   `initialize` + 首次 `tools/list`,超时整 server fault-skip。
2. **Tool call timeout(per-server)**:`tool_timeout_sec` 与
   `ExecCtx::deadline` 取较小者,作为 `tools/call` 的截止时间。超时
   返回 `ToolResult::Error { kind: Timeout, retryable: true }`。
3. **Cancellation**:`ExecCtx::cancel: CancellationToken` 由 H08 提
   供;在 `select!` 中包裹 rmcp call,token fire 时丢弃 future。
   `rmcp` 内部使用 tokio cancellation 原生,drop future = 取消请求
   (stdio 不影响后续调用;HTTP 由 reqwest 关连接处理)。返回
   `ToolResult::Error { kind: Cancelled, retryable: false }`。

`ToolErrorKind` 已有 `Timeout` 和 `Cancelled` 两个 variant,正好对应。

### Q12 · 参数 schema 校验

**不再 cogito 这边校验,直接 forward**。MCP `Tool::input_schema` 是
JSON Schema (Draft 2020-12),与 cogito `ToolDescriptor::schema` 同
spec,直接复制到 descriptor。H07 Tool Resolver 用既有 jsonschema crate
做校验,无 MCP-specific 路径。

唯一边界 case:MCP server 给的 schema 是 `Object<unknown_keys>`,
H07 用 strict 模式校验时可能拒掉模型生成的有效参数。
**对策**:为 MCP-source 工具,`ToolDescriptor::schema` 设置一个
"non-strict additional properties"标记 —— 但 v0.1 不引入新字段,而是
对 MCP 工具 schema 做一次"递归注入 `additionalProperties: false` 缺
失时改为 true"的 transform。
**或者**更简单:H07 默认就该接受 unknown keys(JSON Schema 默认行为
就是允许),只在 descriptor 显式 `additionalProperties: false` 时拒。
回查 H07 实现:它走 jsonschema crate 默认 mode,不强加 strict —— 所以
**无需 transform,直接 forward**。

### Q13 · Observability + provenance

V0.1 只走 `tracing`,**不**在 event log 加 `server_name` 字段。
理由:
- `tool_name` 已是 `mcp__<server>__<tool>`,event log 里 server 信息天
  然存在,grep 即可。
- Event log 是 cross-language 契约(ADR-0007),加字段需要 schema 走
  b-档流程,而当前 `EventPayload::ToolUseRecorded` 用 `tool_name:
  String` 已足够 reproduce。
- 后续如果需要更结构化的 provenance(如"哪个 server 哪次调用"分析),
  加 `EventPayload::McpInvocationCompleted` 之类 additive variant,
  Sprint 5+ 再做。

Tracing 字段:`tool.name`(qualified)/ `tool.server`/ `tool.duration_ms`/
`tool.error_kind`,标准 OpenTelemetry-friendly。

---

## 3 · Architecture

### 3.1 Crate 结构

```text
crates/cogito-mcp/
├── Cargo.toml          # rmcp 1.5 + reqwest + tokio + thiserror + async-trait + cogito-protocol
└── src/
    ├── lib.rs          # re-exports + crate-level doc + Codex attribution
    ├── config.rs       # McpServerConfig, McpTransportConfig (tagged enum)
    ├── error.rs        # McpError (thiserror; Startup/Call/Config variants)
    ├── client.rs       # RmcpClient wrapper (state machine Connecting → Ready)
    ├── transport.rs    # PendingTransport enum, build_stdio / build_http
    ├── naming.rs       # qualify_tool_name + sanitize + sha1 truncation
    ├── provider.rs     # McpToolProvider (impl ToolProvider)
    ├── result_mapping.rs # CallToolResult -> cogito ToolResult
    └── factory.rs      # build_mcp_provider(cfgs) -> Option<Arc<dyn ToolProvider>>
```

### 3.2 关键类型与生命周期

```rust
// config.rs
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransportConfig,
    #[serde(default)]
    pub startup_timeout_sec: Option<f64>,
    #[serde(default)]
    pub tool_timeout_sec: Option<f64>,
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case")]
#[non_exhaustive]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        #[serde(default)] args: Vec<String>,
        #[serde(default)] env: Option<HashMap<String, String>>,
    },
    StreamableHttp {
        url: String,
        bearer_token_env_var: Option<String>,
        #[serde(default)] http_headers: Option<HashMap<String, String>>,
    },
}

// client.rs
pub(crate) struct McpServerHandle {
    server_name: String,
    service: Arc<RunningService<RoleClient, ClientHandler>>,
    tools: Vec<ToolInfo>, // pre-qualified
    tool_timeout: Duration,
}

// provider.rs
pub struct McpToolProvider {
    /// raw qualified name -> handle + raw tool name
    routes: HashMap<String, (Arc<McpServerHandle>, String)>,
    descriptors: Vec<ToolDescriptor>,
}

#[async_trait]
impl ToolProvider for McpToolProvider {
    fn list(&self) -> Vec<ToolDescriptor> { self.descriptors.clone() }
    async fn invoke(&self, name: &str, args: Value, ctx: ExecCtx) -> InvokeOutcome { ... }
}
```

### 3.3 数据流(单次 tool call)

```
Brain (H08)
   │ invoke("mcp__company_api__search", args, ctx)
   ▼
McpToolProvider::invoke
   │ 1. routes.get(name) -> (handle, raw_name)
   │ 2. timeout = min(handle.tool_timeout, ctx.deadline.remaining())
   │ 3. select! { call_tool(handle.service, raw_name, args) ; ctx.cancel }
   ▼
rmcp service.call_tool
   │ JSON-RPC `tools/call` over transport
   ▼
remote MCP server
   │ returns CallToolResult
   ▼
result_mapping::to_cogito_result
   │ map content blocks / is_error / structured_content
   ▼
InvokeOutcome::Sync(ToolResult::Output(...) | ToolResult::Error { ... })
```

### 3.4 启动序列(eager handshake)

```text
Runtime::build()
  │ 读取 RuntimeConfig.mcp_servers
  │ build_mcp_provider(cfgs):
  │   1. for each cfg, spawn task:
  │        - build transport (stdio child process | http client)
  │        - rmcp::service::serve_client(handler, transport)
  │        - timeout(startup_timeout_sec) wrapper
  │        - service.list_tools()
  │        - filter by enabled_tools / disabled_tools
  │   2. JoinSet::join_all() — 并发等所有任务
  │   3. 成功的 handle 收进 routes; 失败的 warn-log skip
  │   4. qualify_tools 在所有成功 server 间统一(防 cross-server dup)
  │   5. 返回 McpToolProvider(routes, descriptors) 或 None(全失败/空)
  │
  │ Composite { children: [BuiltinToolProvider, McpToolProvider?] }
  │ 注入 RuntimeBuilder.tools()
```

---

## 4 · 集成接入点

### 4.1 `cogito-config`

`RuntimeConfigPartial` 加字段:
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfigPartial {
    pub runtime: Option<RuntimeSectionPartial>,
    pub providers: Option<Vec<ProviderConfig>>,
    /// Sprint 4 新增。
    pub mcp_servers: Option<Vec<McpServerConfig>>,
}
```
`RuntimeConfig` 同步加 `pub mcp_servers: Vec<McpServerConfig>` 字段
(finalize 时 `.unwrap_or_default()`)。

Layered merge:`mcp_servers` 跟 `providers` 同策略,**整体替换**而非
元素级 merge(per ADR-0017 §3 已锁定模式),`Some(_)` 覆盖。

### 4.2 `cogito-cli chat`

`ChatConfigInputs` 不变 —— CLI flags 不引入 `--mcp-*`(配置项足够多
之后再考虑)。Runtime 构造在 `chat.rs` 主路径里加一行:

```rust
let mcp_provider = cogito_mcp::build_mcp_provider(&cfg.mcp_servers).await?;
let tools: Arc<dyn ToolProvider> = match mcp_provider {
    Some(mcp) => Arc::new(CompositeToolProvider::new(
        vec![builtin, mcp],
        NamingPolicy::Strict,
    )?),
    None => builtin,
};
```

### 4.3 ToolProvider plumbing

- 不修改 `cogito-protocol::tool::ToolProvider` trait —— MCP 工具完全
  通过既有抽象暴露。
- 不修改 `H05/H07/H08` 任何代码 —— 它们对工具来源不敏感。
- 不修改 event log schema —— `EventPayload::ToolUseRecorded` 字段
  `tool_name: String` 容纳 qualified 名。

### 4.4 Brain 不变量

ADR-0004 layer rule 检查:
- `cogito-mcp` 是 Hand crate(像 `cogito-tools`、`cogito-jobs`)。
- Brain (`cogito-core::harness`) **不**得 `use cogito_mcp::*`。
- `cogito-core::runtime` 可以 dep `cogito-mcp` 吗?**不可以** —— Runtime
  接受 `Arc<dyn ToolProvider>` 注入,具体类型由 Surface(`cogito-cli`)
  组装。所以 `cogito-mcp` 只被 `cogito-cli`(以及未来 `cogito-tui`、
  消费者 Server)依赖,与 `cogito-tools` / `cogito-model` 同层。
- `cogito-config` 依赖 `cogito-mcp` **仅为 value type**(`McpServerConfig`):
  按 ADR-0004 这属于"Hand → Hand 共享 value type",层规允许。
  (如果分级检查脚本不识别这种,加白名单。)

---

## 5 · Testing strategy

### 5.1 单元

**`cogito-mcp`**:
- `naming::qualify` 表驱动测:正常名、含空格、含 `.`、含 `/`、空字符
  串、>64 长度、collision 触发 SHA-1 截断。
- `config` deserialization:两种 transport variant、`bearer_token`
  明文字段被拒(序列化时不存在 / 反序列化时 unknown_field 报错)、
  unknown_field 报错、`mcp_servers` 整段缺失合法。
- `result_mapping::to_cogito_result`:7 个 case 覆盖 §Q7 映射表每行
  + structured_content + is_error。
- `error::McpError` Display 不泄露 secret(bearer token 不进 message)。

**`cogito-config`**:
- `RuntimeConfigPartial` toml roundtrip with `mcp_servers`。
- merge:CLI(`mcp_servers: None`)+ file(`mcp_servers: Some([...])`)
  → finalize 后保留 file 内容。
- 顺承 ADR-0017 §3 array-replace 策略测试。

### 5.2 集成(`crates/cogito-mcp/tests/`)

启 in-process **rmcp server-side**(rmcp `server` feature 开发依赖),
exercise 完整握手 + list + call。两条路径:

| 测试 | Transport | 验证 |
|---|---|---|
| `stdio_handshake_and_call` | stdio,spawn cargo test bin 起 server | initialize → list 返 3 个 tool → call 一个 → 验 ToolResult 形状 |
| `http_handshake_and_call_with_bearer` | streamable-HTTP via 本地 reqwest+axum mock | bearer 头出现在请求 / 缺失 env 报 startup error / 调用成功 |
| `failed_server_fault_contained` | stdio,故意 command 错 | startup 仅 warn 不阻塞,其他 server 工具仍可用 |
| `tool_timeout_fires` | stdio,server 用 `tokio::time::sleep(2s)` 延迟 | `tool_timeout_sec = 1` → 返 `ToolResult::Error { kind: Timeout }` |
| `cancel_token_aborts_call` | stdio,长 sleep | trigger `ExecCtx::cancel` → 返 `ToolResult::Error { kind: Cancelled }` |
| `enabled_tools_filters` | stdio,3 tool 暴露 | `enabled_tools = ["a"]` → list 只 1 项 |
| `name_collision_sanitize_and_dedupe` | stdio,server 暴露 `tool.dot` 和 `tool_dot` | qualified 后冲突 → warn skip 第二个 |

### 5.3 端到端(against 用户的 streamable-HTTP server)

**不进 CI**(secret + 外部依赖)。提供 `just chat-mcp-smoke` 命令:
- 接受 env `COGITO_MCP_TEST_URL` 和 `COGITO_MCP_TEST_TOKEN`。
- 用临时 `cogito.toml` 配 1 个 streamable-HTTP server,启 `cogito chat`,
  发"列出可用工具"prompt,Brain 返工具描述。
- README 一段记录预期输出。

CI 用 in-process mock(§5.2)代替,无 secret 泄露。

### 5.4 覆盖矩阵

| 风险 | 单元 | 集成 | E2E |
|---|---|---|---|
| naming sanitize / collision | ✓ | ✓(name_collision) | — |
| transport stdio | — | ✓(stdio_handshake) | — |
| transport HTTP + bearer | — | ✓(http_handshake) | ✓(用户 server) |
| eager handshake | — | ✓(各 handshake test) | ✓ |
| fault containment | — | ✓(failed_server) | — |
| timeout | — | ✓(tool_timeout) | — |
| cancel | — | ✓(cancel_token) | — |
| enabled/disabled_tools | ✓ | ✓ | — |
| result mapping | ✓(7 cases) | 间接(call 测) | ✓ |
| secret 不泄露 | ✓(error Display) | — | manual 抽 log |

---

## 6 · 风险 + open questions

### 6.1 已识别风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| `rmcp` 1.5 API 不稳(Codex 还在 0.12) | breaking change 拖后腿 | Cargo.lock 锁版本;若 API 大变,本 spec 不必随之改,只换内部实现 |
| streamable-HTTP transport 在某些 server 上 SSE-fallback 失败 | 用户的 server 不通 | E2E 测试就是验它;失败立即 ADR-0018 补一段 fallback 策略 |
| stdio 子进程在 macOS / Windows 路径解析差异 | dev 体验 | Codex 的 `program_resolver.rs` 解决此问题;Sprint 4 v0.1 暂用 `which` crate + 显式 `command` 字符串,Windows 不在 v0.1 验证矩阵 |
| MCP server 返超长 tool schema 把 prompt 撑爆 | model context 爆炸 | H05 已有 `tool_order` + Strategy `allowed_tools` 控制;v0.1 文档化建议:多于 50 个工具的 server 用 `enabled_tools` allow-list |
| `bearer_token_env_var` 配错环境变量名 → secret 静默缺失 | 调用失败但 message 含 endpoint URL | fail-loud:env 缺失时整 server startup 报错,不进 fault-skip(与 §Q6 总体 skip 策略冲突,但 secret 错配是配置 bug,值得 hard-fail) |
| 多 server 并发 startup 时序竞争 | 偶发 startup 失败 | `JoinSet::join_all()` 各自独立,无共享状态,设计上不竞争;集成测试中跑两次确认 |
| rmcp client 在 server panic / 关连接时悬挂 | turn 卡死 | tool_timeout + cancel 双重保护;tracing 标注 server disconnect 事件以便 debug |

### 6.2 Open questions(spec 落地前需要 align)

1. **`bearer_token_env_var` 缺失:hard-fail 还是 warn-skip?**(§6.1
   表中倾向 hard-fail,与 §Q6 server-level skip 不一致。
   **推荐 hard-fail**:secret 错配是配置 bug,值得醒目报错。)
2. **是否在 `cogito-config` 里加 `strict_mcp_startup: bool` 字段?**
   v0.1 不暴露(§Q6 决定),但要不要预留字段为 v0.4 留位?
   **推荐预留** —— 名字进 schema,值固定为 false,doc 注明 "Sprint
   4 ignored; v0.4 SaaS-ready 启用 hard-fail mode"。
3. **H07 对 MCP server 给的 schema 是否信任?** §Q12 的答案是"信任,
   直接 forward",但如果 MCP server 给一个故意宽松的 schema(允许任
   意字段),H07 会 happily 接受。是否需要在 cogito 边界做一次最低限
   schema sanity 检查(如必须是 `type: "object"`)?
   **推荐 v0.1 不做** —— H07 行为现状;遇到坏 schema 再补。
4. **MCP server 暴露的 tool description 是否有长度上限?** 长 description
   会撑爆 prompt。**推荐 v0.1 不截断**,文档化"建议 server 端控制
   description 长度"。
5. **stdio server 的 `cwd` 字段是否引入?** Codex 配置里有,允许指定子
   进程工作目录。我们 v0.1 默认继承父进程 cwd,**不**暴露 cwd 字段。
   遇到需要时再加。

---

## 7 · 实施分解 preview(plan 文档会展开)

7 个 Task,按依赖关系排序。Plan 文档(`docs/superpowers/plans/
2026-05-21-sprint-4-mcp-sync-tools.md`)展开每个 Task 的:目标 / 文件
列表 / 验证步骤 / 退出条件。

| # | Task | 主要文件 | 验证 |
|---|---|---|---|
| T1 | `cogito-mcp` 骨架 + `McpServerConfig` value types + `McpError` | `cogito-mcp/Cargo.toml`, `src/{lib,config,error}.rs` | `cargo test -p cogito-mcp`(config roundtrip) |
| T2 | `naming::qualify` + 单测(表驱动,12+ case) | `src/naming.rs` + tests | 覆盖 §Q4 全部边界 |
| T3 | `transport::{build_stdio, build_streamable_http}` + `client::McpServerHandle` 状态机 | `src/{transport,client}.rs` | 集成测试 `stdio_handshake_and_call` + `http_handshake_and_call_with_bearer` |
| T4 | `result_mapping::to_cogito_result` + 单测(7 case) | `src/result_mapping.rs` | §Q7 映射表全覆盖 |
| T5 | `McpToolProvider` impl + `build_mcp_provider` 工厂 + eager 并发握手 | `src/{provider,factory}.rs` | 集成测试 `failed_server_fault_contained` + `enabled_tools_filters` |
| T6 | `cogito-config` 加 `mcp_servers` 字段 + finalize + tests | `cogito-config/src/{types,merge}.rs` + tests | toml roundtrip + merge 覆盖 |
| T7 | `cogito-cli chat.rs` 接入 + E2E smoke + README 段 + ADR-0018 + CHANGELOG | `cogito-cli/src/chat.rs`, `docs/adr/0018-*.md`, `README.md`, `docs/configuration/overview.md`, `CHANGELOG.md` | 手动 E2E 对用户 server,`just ci` 绿 |

T1–T2 可并行起步(纯本地);T3 依赖 T1;T4 独立;T5 依赖 T3+T4;
T6 独立(纯 config);T7 依赖 T5+T6。

**估时**:1.5–2 个工作日(spec 锁定后)。

---

## 8 · 参考

- ROADMAP §"Sprint 4 · MCP sync tools"(2026-05-21 renumber commit)
- ADR-0004 §Brain/Hands/Session boundaries(`cogito-mcp` 层位)
- ADR-0007 §Event log cross-language contract(为什么 v0.1 不加
  MCP-specific event)
- ADR-0017 §6(secret interpolation;`bearer_token_env_var` 复用)
- ADR-0017 §3(layered partial merge;`mcp_servers` array-replace)
- CLAUDE.md §"Tagged-config factories"(factory 放 `cogito-mcp`,不
  在 Surface fork)
- MCP spec 2025-06-18(rmcp 1.5 targets):
  https://modelcontextprotocol.io/specification/2025-06-18
- Codex `codex-rmcp-client` (Apache-2.0,architecture inspiration):
  `agents/codex/codex-rs/rmcp-client/` 本地路径
- Codex `core/src/mcp_connection_manager.rs`(naming algorithm
  reference,pattern-only)

# 调研报告:Subagent 父→子 context 交接设计 — Claude Code / Codex / Manus 横向对比

**日期**: 2026-06-12
**目的**: 为 ADR-0011 v0.3 amendment(四工具 subagent 生命周期)中"父 agent 如何为子 agent 设置背景"的设计决策提供业界依据。
**方法**: 三个并行 web 调研 agent,分别覆盖一家;官方文档优先,逆向工程/二手笔记标注来源;不确定项标 UNVERIFIED。
**适用范围**: 截至 2026-06 的公开信息。Claude Code / Codex 迭代很快,字段名可能随版本漂移。

---

## 1. Claude Code(Anthropic)

主来源: 官方文档 <https://code.claude.com/docs/en/sub-agents>(2026-06-12 抓取);
系统提示词逆向 <https://weaxsey.org/en/articles/2025-10-12/>;
CLAUDE.md 注入机制逆向 <https://agiflow.io/blog/claude-code-internals-reverse-engineering-prompt-augmentation/>。

### 1.1 父→子边界上传递什么(Task/Agent 工具 schema)

工具原名 `Task`,v2.1.63 改名 `Agent`(`Task` 仍是别名)。输入字段:

| 字段 | 用途 |
|---|---|
| `description` | "A short (3-5 word) description of the task" — 仅用于 UI/转录标注,不进入子方 |
| `prompt` | "The task for the agent to perform" — 成为子的任务消息,**父方内容进入子方的唯一通道** |
| `subagent_type` | 选择内置 agent(`general-purpose` / `Explore` / `Plan`,另有 `statusline-setup`、`claude-code-guide` 等辅助型)或按 `name` 选自定义 agent |
| `model`(可选,按次) | 按次模型覆盖。解析顺序: `CLAUDE_CODE_SUBAGENT_MODEL` 环境变量 → 按次 `model` 参数 → frontmatter `model` → 主对话模型 |
| `isolation: "worktree"`(可选) | 子在独立 git worktree 中改文件 |

调用内置 Explore 时父方还会指定 thoroughness 档位(quick / medium / very thorough)。

### 1.2 子是否看到父的对话历史

**默认完全看不到 — fresh, isolated context。** 官方原文:

> "Each subagent starts with a fresh, isolated context window. It does not see your
> conversation history, the skills you've already invoked, or the files Claude has
> already read. Claude composes a delegation message that summarizes the task, and
> the subagent works from there."

唯一例外: **fork**(v2.1.117+,`/fork` 命令或 `CLAUDE_CODE_FORK_SUBAGENT=1`)——
继承到当前为止的全部对话(同 system prompt、同工具、同模型、同消息历史),并复用父的
prompt cache。命名 subagent 永远 fresh 启动。

### 1.3 harness 自动注入什么(与父模型无关)

官方 "What loads at startup" 列出非 fork 子的初始 context 恰好包含:

1. **System prompt**: 子 agent 自己的提示词 + Claude Code 追加的环境细节(**不是**完整的
   Claude Code 主系统提示词)。环境细节含工作目录("A subagent starts in the main
   conversation's current working directory")及平台/git 仓库信息;字段全集官方未逐项列出
   (UNVERIFIED,但与观察到的 `<env>` 块一致)。
2. **Task message**: 父模型撰写的委派提示词。
3. **CLAUDE.md 与记忆**: 主对话加载的**全部记忆层级**(`~/.claude/CLAUDE.md`、项目规则、
   `CLAUDE.local.md`、managed policy files)。机制上以 `<system-reminder>` 块注入消息流
   (agiflow 逆向)。**例外: 内置 Explore 和 Plan 跳过 CLAUDE.md**("to keep research
   fast and inexpensive"),且没有任何 frontmatter 字段可以改变这个跳过行为。
4. **Git status**: 父 session 启动时的快照。非 git 目录或 `includeGitInstructions: false`
   时缺省;Explore/Plan 无条件跳过。
5. **预载 skills**: frontmatter `skills` 列出的 skill **全文**注入(不只是描述)。内置
   agent 不预载。
6. 启用 `memory` 时: 记忆目录指引 + `MEMORY.md` 前 200 行或 25KB 进 system prompt。

### 1.4 子的 system prompt 来源与委派路由

- **自定义 agent**: Markdown 文件(项目 `.claude/agents/*.md`、用户 `~/.claude/agents/`、
  plugin `agents/`、managed settings、`--agents` CLI JSON;优先级 managed > CLI >
  project > user > plugin)。YAML frontmatter 是配置;**Markdown 正文即子的 system prompt**。
- **frontmatter 字段**(仅 `name`、`description` 必填): `name` / `description` / `tools` /
  `disallowedTools` / `model`(默认 `inherit`)/ `permissionMode` / `maxTurns` / `skills` /
  `mcpServers` / `hooks` / `memory` / `background` / `effort` / `isolation` / `color` /
  `initialPrompt`。
- **路由 — `description` 字段是路由信号。** 官方: "Claude uses each subagent's
  description to decide when to delegate tasks";鼓励在 description 里写 "use
  proactively" 促成主动委派。机制上 agent description 被呈现在父模型可见的 Agent 工具
  schema/提示词里,供父模型选 `subagent_type`。用户可用 `@agent-<name>` 强制路由 ——
  但 @-mention 只决定调谁,**子收到的 prompt 仍由父模型撰写**。
- 内置 agent 提示词(逆向,weaxsey.org): general-purpose 为 "You are an agent for
  Claude Code... Do what has been asked; nothing more, nothing less.";Explore 为
  "You are a file search specialist..."。

### 1.5 子的工具范围

- 默认**继承**主对话的全部内置工具 + MCP 工具。
- `tools` = 白名单;`disallowedTools` = 黑名单,先减后解析,两表同列者移除。
- 无论 frontmatter 怎么写,subagent 永远拿不到: `Agent`、`AskUserQuestion`、
  `EnterPlanMode`、`ExitPlanMode`(除非 `permissionMode: plan`)、`ScheduleWakeup`、
  `WaitForMcpServers`。因此 **subagent 不能再 spawn subagent(无嵌套)**。
- `mcpServers` frontmatter 可以授予子方父方没有的 MCP server(inline 定义在子启动时连接、
  结束时断开)——inline-only server 的工具描述不占父方 context。
- 权限: 子继承父的权限上下文;`permissionMode` 可覆盖,但父处于
  `bypassPermissions`/`acceptEdits`/`auto` 时父优先。可用
  `permissions.deny: ["Agent(name)"]` 禁用特定 subagent。
- frontmatter 里的 `PreToolUse` hooks 提供比工具级更细的控制(如 Bash 只放行只读 SQL)。

### 1.6 子→父返回 + 官方委派提示词指引

- **返回 = 子的最终 assistant 消息,单条文本报告,无结构化 schema**(可恢复类型另附
  agent ID;Explore/Plan 一次性、无 ID)。
- 父模型收到的系统提示词指引(逆向,weaxsey.org):
  - "Each agent invocation is stateless. You will not be able to send additional
    messages to the agent, nor will the agent be able to communicate with you outside
    of its final report."
  - "your prompt should contain a highly detailed task description for the agent to
    perform autonomously and you should specify exactly what information the agent
    should return back to you in its final and only message to you."
  - "The result returned by the agent is not visible to the user. To show the user
    the result, you should send a text message back to the user with a concise
    summary of the result."
  - "Launch multiple agents concurrently whenever possible … use a single message
    with multiple tool uses."
- 官方文档指引: "The only channel from parent to subagent is the Agent tool's prompt
  string, so include any file paths, error messages, or decisions the subagent needs
  directly in that prompt";用 Explore/Plan 时若子必须遵守某条 CLAUDE.md 规则,要在委派
  提示词里**重述**;警告大量 subagent 各自返回详细结果会显著消耗父 context。
- 注意: GitHub issue anthropics/claude-code#11892 指出 "stateless" 措辞已与后来的
  resume 功能矛盾。

### 1.7 与运行中子的多轮交互

- 历史上 fire-and-forget。现状更细:
- **完成后 resume**: "Resumed subagents retain their full conversation history... picks
  up exactly where it stopped." 经 `SendMessage` 工具按 agent ID 续聊 —— 但
  `SendMessage` 仅在 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` 时可用;已停止的 subagent
  收到 `SendMessage` 会后台自动恢复。Explore/Plan 不可恢复。子转录持久化于
  `~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl`,独立于主对话
  compaction,按 `cleanupPeriodDays`(默认 30 天)清理。
- **运行中**: 父模型没有通道向运行中的命名 subagent 发消息。后台 subagent 自动拒绝权限
  询问、不能反问澄清(该工具调用失败但子继续跑)。**人类用户**可经 fork 面板与运行中的
  fork 交互(Enter 打开其转录并发后续消息),Ctrl+B 把任务转后台 —— 这是用户级,不是
  父模型级。
- 子有自己的 auto-compaction(约 95% 容量触发)与 `maxTurns` 预算。

### 1.8 不确定项

- 子收到的 "environment details" 块的逐项内容(platform 串、OS 版本、日期等)官方未枚举
  — UNVERIFIED。
- 内置 agent 提示词原文与 Task 工具 usage notes 原文来自逆向,多个独立来源相互印证,
  但可能随版本漂移。

---

## 2. Codex CLI(OpenAI)

主来源: 官方 subagents 文档 <https://developers.openai.com/codex/subagents>;
配置参考 <https://developers.openai.com/codex/config-reference>;
开源实现 <https://github.com/openai/codex>(`codex-rs/core/src/tools/handlers/multi_agents/`
下 `spawn.rs` / `send_input.rs` / `wait.rs` / `resume_agent.rs` / `close_agent.rs`、
`multi_agents_spec.rs`、`multi_agents_v2.rs`);相关 issues 见文中编号。

### 2.1 是否有 subagent;名称与机制

**有,一等公民且默认开启。** 官方称 **subagents / multi-agent collaboration tools**
(历史上叫 "collab" 模式;配置门 `features.multi_agent`,更早是实验旗 `collab_tools`)。

- **v1 五工具**: `spawn_agent`、`send_input`、`resume_agent`、`wait_agent`、`close_agent`
  (stable,默认开启)。
- **MultiAgentV2 新一代**: `spawn_agent`(按 task 命名)、`send_message`、`followup_task`、
  `wait_agent`(mailbox 风格)、`list_agents`、`interrupt_agent`。
- **模型驱动编排**;CLI runtime 负责线程调度、审批转发、结果收集。只有被明确要求时才
  spawn。TUI 里 `/agent` 在活跃 agent 线程间切换。
- `/review` 模式也会 spawn 隔离的子评审线程(曾有继承父方实时 sandbox override 的 bug,
  issue #15305)。
- 批量变体 **`spawn_agents_on_csv`**(每 CSV 行一个 worker,worker 调
  `report_agent_job_result` 汇报)。
- Codex cloud 任务是隔离单线程;cloud 能否 spawn collab subagent: UNVERIFIED。

### 2.2 父→子传什么;历史继承

v1 `spawn_agent` schema(`multi_agents_spec.rs`):

| 字段 | 语义 |
|---|---|
| `message` | "Initial plain-text task for the new agent. Use either message or items."(自由文本) |
| `items` | "Structured input items. Use this to pass explicit mentions (for example app:// connector paths)."(结构化替代) |
| `agent_type` | 选择配置好的 role |
| `fork_context`(bool) | "True forks the current thread history into the new agent; false or omitted starts with only the initial prompt." 代码: `fork_mode: args.fork_context.then_some(SpawnAgentForkMode::FullHistory)` + `fork_parent_spawn_call_id` |
| `model` / `reasoning_effort` / `service_tier` | 按子覆盖("Omit to inherit the parent effort") |

- **v1 默认 = fresh 子 + 仅初始 prompt;fork 是 opt-in。**
- **V2 翻转了默认**: `fork_turns` 取代 `fork_context`(取值 `none` / `all` / 正整数串),
  **默认 `all` = 全量历史 fork**;全量 fork 的子继承父的 agent type/model/effort,除非
  `fork_turns: "none"` 否则拒绝覆盖(issue #20077)。
- **已知失效模式**: fork 出的 subagent 有时继续干父的活而不是被委派的任务
  (issue #24150);有用户要求给 `fork_context` 加硬开关,因为它可能快照巨大的过期父历史
  (issue #14981)。

### 2.3 harness 自动注入

- 子"从该 turn 的 **effective config** 启动": 运行时态 —— provider、approval policy、
  sandbox、cwd —— 被继承;spawn 时**重放父 turn 的实时运行时 overrides**(如
  `/permissions` 变更、sandbox 选择)。代码:
  `apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())`、
  `environments: Some(turn.environments.to_selections())`。
- 基础指令: `build_agent_spawn_config(&session.get_base_instructions() ...)` —— 子拿到
  与父相同的 base system instructions。
- 子是同 cwd 下的一个普通新 session,走标准 session init,注入 `<environment_context>`
  (cwd、sandbox、approval、shell)与 `<user_instructions>`(AGENTS.md)—— 与 rollout
  trace 分析一致(<https://dev.to/milkoor/reverse-engineering-codex-cli-rollout-traces-3b9b>),
  但官方 subagents 页未明说 AGENTS.md 重注入: 部分 UNVERIFIED。
- 限制: `spawn_agent` 把子的 `approval_policy` 钳死为 `never`(子不能直接向人请求审批,
  issue #12713)。

### 2.4 子 system prompt / role 定义

两条路:

- **`.codex/agents/` 下自定义 agent TOML**: 必填 `name`、`description`、
  `developer_instructions`;可选 `nickname_candidates`、`model`、
  `model_reasoning_effort`、`sandbox_mode`、`mcp_servers`、`skills.config`(缺省字段
  继承父 session)。agent 文件作为**配置层**叠在被 spawn session 的 config 上。
- **`config.toml` role 键**: `agents.<name>.description`("role guidance shown to
  Codex when choosing and spawning that agent type")、`agents.<name>.config_file`、
  `agents.<name>.nickname_candidates`。spawn.rs 经 `apply_role_to_config` 应用。
- role 的 `developer_instructions` 成为子的 developer message,叠在共享 base system
  prompt 之上;父传 `agent_type` 选 role。

### 2.5 子的工具范围

- 经 role 配置层: 自定义 agent 可设自己的 `sandbox_mode`、`mcp_servers`、
  `skills.config`,决定子的工具面;缺省继承父。
- 嵌套有界: `agents.max_depth` 默认 **1**(子不能再 spawn 孙;spawn.rs
  `exceeds_thread_spawn_depth_limit`,报错 "Agent depth limit reached. Solve the task
  yourself.");`agents.max_threads` 默认 **6** 并发。
- Claude-Code 式逐工具 allowlist 字段: 未见文档 — UNVERIFIED/缺失。

### 2.6 子→父返回;阻塞与运行中输入

- **spawn 异步非阻塞**: `spawn_agent` 立刻返回子 `thread_id`(v1)或规范 `task_name`
  (v2)+ nickname。父继续干活,用 **`wait_agent`** 显式同步 —— v1: "Wait for agents
  to reach a final status. Completed statuses may include the agent's final message"
  (多目标 = 先完成者先返回,带 `timeout_ms`);v2: mailbox 更新等待。
- **返回值是子的最终 assistant 消息**,经 `wait_agent` 交付;CSV 任务经
  `report_agent_job_result`。
- **父可以与运行中的子对话**: v1 `send_input`("Send a message to an existing agent.
  Use interrupt=true to redirect work immediately";否则排队);v2 `send_message`(排队,
  不触发新 turn)、`followup_task`(子空闲则触发 turn)、`interrupt_agent`。
  `resume_agent` 重开已关闭的子;`close_agent` "closes an agent and any open
  descendants."
- 官方顶层描述 "waits for all requested subagent results before returning a
  consolidated response" 指的是模型在一个用户 turn 内的编排模式,底层即上述异步工具。

### 2.7 持久化的父↔子链接

**有。**

- 每线程一个 rollout 文件 `~/.codex/sessions/YYYY/MM/DD/rollout-TIMESTAMP-UUID.jsonl`;
  首行 `session_meta` 记录线程来源。subagent rollout 在
  `session_meta.payload.source.subagent.thread_spawn` 下记录 **`parent_thread_id`**
  (spawn.rs 传 `parent_thread_id: Some(session.thread_id)`)。通用字段
  `session_meta.payload.thread_source` 区分线程来源(issue #23001)。
- 用户发起的 session fork 类似地在新 `SessionMeta` 记 `forked_from_id`。
- 链接被实际使用: app-server `thread/archive` 会连带迁移 spawn 出的后代线程 rollout;
  app-server `collabToolCall` 条目暴露 `senderThreadId` / `receiverThreadId` /
  `newThreadId`。

### 2.8 OpenAI Agents SDK: `Agent.as_tool` vs handoff(相邻设计)

来源: <https://openai.github.io/openai-agents-python/tools/>、
<https://openai.github.io/openai-agents-python/handoffs/>。

- **`Agent.as_tool`**: 管理者保持控制;嵌套 agent 以工具调用运行。默认输入是**单个生成
  字符串**(`{"input": "..."}`);经 `parameters=`(Pydantic/dataclass)可结构化,呈现于
  嵌套 `RunContextWrapper.tool_input`。子**不**收到父对话历史 —— 只有工具输入。输出可用
  `custom_output_extractor` 重塑;另有 `max_turns`、`run_config`、`on_stream`、
  `needs_approval`。官方: "If you want structured input for a nested specialist
  without transferring the conversation, prefer `Agent.as_tool(parameters=...)`."
- **Handoff**: 控制权转移 —— "it's as though the new agent takes over the conversation,
  and gets to see the **entire previous conversation history**"(默认)。可用
  **`input_filter`** 修剪(函数收 `HandoffInputData`: `input_history` /
  `pre_handoff_items` / `new_items` / 可选 `input_items` / `run_context`,返回新的
  `HandoffInputData`;预制过滤器在 `agents.extensions.handoff_filters`,如
  `remove_all_tools`)。`input_type=` 添加结构化参数 schema(reason/priority 元数据),
  校验后传给 `on_handoff` —— **不替代**下一个 agent 的主输入。
  `RunConfig.nest_handoff_history` 可把先前转录折叠为单个 `<CONVERSATION HISTORY>`
  摘要块。
- 总结: **as_tool = 仅输入串、父方控制、同步工具调用;handoff = 全历史转移 + 可过滤。**
  Codex 的 `spawn_agent` 是杂交: v1 默认 as_tool 式 fresh + opt-in handoff 式全量
  fork —— v2 把默认翻成了全量 fork。

---

## 3. Manus(Monica / Butterfly Effect)

主来源(官方): 核心博文 "Context Engineering for AI Agents: Lessons from Building
Manus"(Yichao "Peak" Ji,2025-07,<https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus>);
Wide Research 发布文 <https://manus.im/blog/introducing-wide-research>、
<https://manus.im/blog/manus-wide-research-solve-context-problem>、
<https://manus.im/docs/features/wide-research>;Peak Ji 的 X 帖。
二手(可信但是笔记非逐字): LangChain 2025-10 webinar 笔记
<https://rlancemartin.github.io/2025/10/15/manus/>、
<https://www.philschmid.de/context-engineering-part-2>。
Manus 闭源,以下区分官方陈述与转述。

### 3.1 总体架构与观点演化

- **第一阶段(2025-07 官方博文)**: 单 agent 循环 —— "After receiving a user input,
  the agent proceeds through a chain of tool uses to complete the task." 该文完全没有
  subagent/multi-agent。
- **第二阶段(2025-08 Wide Research;2025-10 webinar)**: 演化为 planner + executor
  子 agent,但明确定位为 **context 隔离,不是角色分工**:
  - "While humans organize by role (designer, engineer, project manager) due to
    cognitive limitations, LLMs don't necessarily share these same constraints."
    (webinar 转述)
  - 实际存在的角色: **planner agent**(派任务、**为 sub-agent 定义输出 schema**)、
    **executor sub-agents**(干活)、**knowledge manager**(审查对话、决定什么持久化到
    文件系统)。
- **反 naive-multi-agent 的原话(官方)**:
  - Wide Research 发布文: "Unlike traditional multi-agent systems based on predefined
    roles (like 'manager', 'coder', or 'designer'), every subagent in Wide Research is
    a fully capable, general-purpose Manus instance."
  - Peak Ji on X(2025-12,<https://x.com/peakji/status/2002023656881611156>): "Each
    sub-agent has the same action space as the main agent, which fundamentally
    distinguishes it from multi-agent designs that aim to differentiate agents by
    persona or action space." 并表示不反对 multi-agent 本身,取决于用例;引用 Google
    Research "Towards a Science of Scaling Agent Systems" 与自家实验一致。
  - "the sub-agents do not communicate with each other, all coordination flows through
    the main controller. This prevents context pollution and maintains independence."
  - webinar(转述): 把并发谚语 "Share memory by communicating, don't communicate by
    sharing memory" 应用到 agent;性能提升大多来自**删**复杂度 —— 撤掉管理型 agent,
    换成简单的结构化交接 / agent-as-a-tool;6 个月重构约 5 次;"if performance doesn't
    improve with stronger models, your harness may be hobbling the agent."
  - Peak Ji(2025-07,X): "Context engineering can also overfit … we never commit to
    an architecture based on static benchmarks."

净演化: 通用单循环 → planner + 通用 executor 子 agent,目的纯粹是 context 隔离与并行,
中心化协调、无 agent 间互聊、无 persona 角色。

### 3.2 Wide Research 子 agent 收到什么 context

- 主控 "analyzes your request and breaks it down into independent, parallelizable
  sub-tasks."
- 每个 sub-agent 是 "a fully capable, general-purpose Manus instance",拥有: 完整虚拟机
  环境、全量工具库(搜索/浏览/代码执行/文件处理)、独立网络连接、**"A fresh, empty
  context window"**。
- 即 fan-out 时**不传父全量 context** —— 每个子拿到一份精心构造的 sub-task brief +
  fresh context + 自己的 VM,"focus exclusively on its assigned item"。
- 子 VM 是否与父共享文件系统: UNVERIFIED(每子"完整 VM"暗示隔离,结果经主控回流)。
- 非 Wide-Research 的普通委派是**双模式**(见 3.4)。

### 3.3 与交接相关的 context-engineering 原则(官方 2025-07 博文)

- **File-system-as-context**: "the file system [is] the ultimate context in Manus:
  unlimited in size, persistent by nature, and directly operable by the agent itself"
  —— 外化记忆;压缩必须可恢复(丢网页正文但保留 URL/文件路径)。**这是 context 继承的
  替代品**: 子 agent 不需要父的 token 历史,可以从文件重读状态。webinar 补充: 工具结果
  卸载到文件系统,用 glob/grep 检索,无向量索引;knowledge-manager agent 决定什么落盘。
- **todo.md 复诵**: "By constantly rewriting the todo list, Manus is reciting its
  objectives into the end of the context… avoiding 'lost-in-the-middle'." **重要演化**:
  到 2025-10 webinar 已**弃用** todo.md —— 约 1/3 的动作耗在更新清单上;改为 planner
  子 agent 返回结构化 **Plan 对象**,"injected into the context only when needed"。
- **KV-cache 友好的 append-only context**: "Make your context append-only";单 token
  差异即从该处失效缓存;稳定前缀、确定性序列化。KV-cache 命中率被称为生产 agent 最重要
  的单一指标。对交接的影响: 与其修改共享 context,不如 spawn 全新 context(便宜、可缓存)
  并传 brief —— 又一个继承的替代品。
- **Mask, don't remove(工具屏蔽)**: 用 "context-aware state machine … masks the token
  logits during decoding" 而非动态增删工具定义(那会失效 KV-cache)。后期补充分层动作
  空间: 原子函数 < 20 个(bash、文件操作、代码执行);MCP 工具以 sandbox 内 CLI 形式
  暴露而非 schema 工具。

### 3.4 planner 如何向 executor 传达子任务

(webinar 转述,二手)

- **Agent-as-a-tool / 函数调用式交接**: "The main agent invokes
  `call_planner(goal=\"...\")`, the harness spins up a temporary sub-agent loop and
  returns a structured result" —— MapReduce 式,明确类比 Claude Code 的 Task 工具。
- **双模式**:
  - 简单任务(产出离散、无共享文件依赖): "Planner passes instructions via function
    call" —— 结构化工具调用参数里装自然语言 brief。
  - 复杂任务(有共享文件依赖): "Planner shares its full context with the sub-agent.
    The sub-agent still has its own action space (tools) and instructions, but
    receives the full context that the planner also has access to."
- planner 还**定义 executor 必须填的输出 schema**,以约束解码(constrained decoding)
  强制执行。

### 3.5 已公开的 schema / 字段名

公开极少:

- `call_planner(goal="...")` —— 函数名与 `goal` 参数,webinar 笔记;拼写可能是笔记者
  转述 — UNVERIFIED。
- planner 返回的结构化 **Plan object** —— 字段名未公开 — UNVERIFIED。
- sub-agent 调用的 **"submit results"** 工具,填 planner 定义的 schema,约束解码保证
  合规 —— 确切工具名/字段未公开。
- compaction schema: 工具调用有 "full" 与 "compact" 两种表示,compact 保留文件路径/URL
  引用;摘要用 "a defined schema [to] ensure consistent summary objects across
  trajectories" —— 字段名未公开。
- 未发现任何交接 JSON 的官方 schema/API 文档/可信泄露。(注: 2025-03 流传的 "Manus
  system prompt 泄露" 属单 agent 时代的提示词/工具,与 planner/executor 交接无关,且
  部分存疑 — UNVERIFIED。)

### 3.6 子→父返回什么

- **符合 planner 定义 schema 的结构化结果对象**,经 "submit results" 工具 + 约束解码
  交付;子的完整轨迹**不**回流父 context。
- Wide Research: "Once all sub-agents have reported back, the main controller
  synthesizes the results into a single, coherent, and comprehensive report" ——
  只有完成的子任务产出返回;子间无通信。
- 子 VM 产出的文件工件是否传回父 VM: UNVERIFIED(文件中心设计下合理,但未声明)。

---

## 4. 横向对比表

| 维度 | Claude Code | Codex CLI | Manus |
|---|---|---|---|
| 工具面 | `Agent`(原 `Task`)单工具,fire-and-forget(+实验性 SendMessage resume) | v1 五工具: spawn / send_input / resume / wait / close;v2 六工具 mailbox 风格 | agent-as-a-tool 函数调用(`call_planner` 等),Wide Research 批量 fan-out |
| 默认 context | fresh,唯一通道 = `prompt` 字符串 | v1 fresh(仅 `message`);**v2 默认全量 fork 父历史** | fresh + 构造的 brief |
| 全量继承选项 | fork(`/fork`,继承全部+复用 KV cache;命名 subagent 不可) | `fork_context`(v1 opt-in)/ `fork_turns`(v2 默认 all;**有"子继续干父的活"失效模式**) | 复杂任务(共享文件依赖)时 planner 共享全量 context |
| 结构化输入 | 无(纯自由文本) | `items`(结构化 input items,可传 mention/路径) | 函数调用参数;**planner 给 executor 定义输出 schema** |
| harness 自动注入 | 子自己的 system prompt + 环境块(cwd/git 快照)+ **全部 CLAUDE.md 层级** + 预载 skills(Explore/Plan 跳过 CLAUDE.md+git) | base instructions + `environment_context` + AGENTS.md + 父运行时 overrides(sandbox/approval/cwd) | 文件系统即 context(子自行重读);全工具库 + 独立 VM |
| role 定义载体 | `.claude/agents/*.md`(frontmatter 配置 + 正文即 system prompt) | `.codex/agents/*.toml`(`developer_instructions` 叠在共享 base 之上)或 config.toml `agents.<name>` | 无 persona 角色;子与父同 action space |
| 委派路由信号 | agent `description` 字段(注入父可见的工具 schema) | `agents.<name>.description`("choosing and spawning" 指引) | planner 自行分解,无 role 选择 |
| 工具作用域 | 默认继承;`tools` 白名单 + `disallowedTools` 黑名单;子永远禁 spawn(无嵌套) | role 配置层(sandbox/mcp/skills);`max_depth` 默认 1,`max_threads` 默认 6 | 全量工具(同父 action space);分层动作空间 <20 原子函数 |
| 运行中交互 | 父模型不可(用户可经 fork 面板);完成后可 SendMessage resume(实验) | **可**: send_input(可 interrupt)/ send_message / followup_task / interrupt_agent | 不可,子间也不互聊,全经主控 |
| 返回 | 子最终消息(自由文本)+ 指引父"明确说清子该返回什么" | 子最终消息,经 wait_agent(先完成者先回) | **结构化结果对象**(planner schema + 约束解码) |
| 持久化父子链接 | 子转录独立 jsonl(`subagents/agent-{id}.jsonl`) | **`parent_thread_id`** 写入子 rollout 的 `session_meta`(与 cogito SessionMeta 同路) | 闭源未知 |
| 深度/并发护栏 | 不可嵌套(子无 Agent 工具);`maxTurns` | `max_depth=1`、`max_threads=6`、子 approval 钳 `never` | 中心化主控,子不再分解 |

## 5. 对 cogito v0.3 的设计启示

1. **草案方向被业界验证**: Codex v1 五工具与 cogito 草案四工具几乎同构(cogito 暂无
   `resume_agent`);`parent_thread_id` 持久化与 cogito 已落地的
   `SessionMeta.parent_session_id` 同路。Codex `max_depth` 默认 1 / cogito 默认 3,
   量级一致。
2. **harness 注入环境事实是三家共识**: Claude Code(env 块 + CLAUDE.md 层级)、Codex
   (`environment_context` + AGENTS.md)、Manus(文件系统即 context)都不依赖父模型转述
   环境。cogito 当前子 agent 只有 role system prompt + input,缺这一层。
3. **自由文本 brief 是主通道,三家皆然**: 结构化输入是补充(Codex `items`、SDK
   `parameters=`),没有一家把主通道换成多槽位表单。"打包一切进 prompt"的嘱咐写在
   **工具 description** 里(Claude Code 的做法,cogito v0.2 已同款)。
4. **结构化的真正价值在输出侧**(Manus 独有创新): planner 给 executor 定义结果 schema
   并约束解码。轻量版做法是 spawn 参数加 `expected_output`,拼进子首 turn。
5. **role description 进父可见工具 schema 是标准实践**: Claude Code 用它做路由,Codex
   用它做"选谁+怎么 spawn"的指引。cogito 的 strategy(ADR-0026)可加 description
   暴露给 `spawn_agent` 的工具 schema。
6. **全量 fork 父历史须谨慎**: Codex v2 翻转默认后出现"子继续父的活"失效模式
   (issue #24150)和 kill-switch 诉求(#14981);Claude Code 把 fork 与命名 subagent
   严格分开;Manus 只在"共享文件依赖"场景共享全量 context。fresh-only 是稳妥默认,
   fork 可作未来 opt-in。
7. **"父模型撰写委派提示词"这一机制本身需要系统提示词指导**: Claude Code 在父的系统
   提示词里写了详细的委派写法指引("highly detailed task description… specify exactly
   what information the agent should return")。cogito 对应物是 `spawn_agent`/`delegate`
   的工具 description,v0.2 已有雏形,可按此标准加强。
8. **并发护栏不只深度**: Codex 另有 `max_threads`(并发子数)护栏;cogito 草案只有
   深度限制,fan-out 数量护栏值得考虑。

## 6. 来源清单

**Claude Code**
- <https://code.claude.com/docs/en/sub-agents>(官方,主来源)
- <https://weaxsey.org/en/articles/2025-10-12/>(系统提示词/Task schema 逆向)
- <https://agiflow.io/blog/claude-code-internals-reverse-engineering-prompt-augmentation/>(CLAUDE.md 注入机制)
- <https://github.com/anthropics/claude-code/issues/11892>(stateless 与 resume 矛盾)

**Codex**
- <https://developers.openai.com/codex/subagents>(官方 subagents 文档)
- <https://developers.openai.com/codex/config-reference>(`[agents]`、`features.multi_agent`)
- <https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/handlers/multi_agents_spec.rs>(v1+v2 工具 schema)
- <https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/handlers/multi_agents/spawn.rs>(fork/继承/深度逻辑)
- Issues: #20077(fork_turns 默认 all)、#24150(fork 失效模式)、#14981(kill-switch 诉求)、#12713(approval 钳 never)、#15305(/review 子线程 bug)、#23001(thread_source)
- <https://openai.github.io/openai-agents-python/tools/>、<https://openai.github.io/openai-agents-python/handoffs/>(Agents SDK)
- <https://dev.to/milkoor/reverse-engineering-codex-cli-rollout-traces-3b9b>(rollout trace 分析)
- <https://deepwiki.com/openai/codex/4.4-session-resumption-and-forking>(forked_from_id)
- <https://github.com/ryoppippi/ccusage/issues/950>(真实 rollout 文件中的 parent_thread_id)

**Manus**
- <https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus>(2025-07 核心博文)
- <https://manus.im/blog/introducing-wide-research>(2025-08)
- <https://manus.im/blog/manus-wide-research-solve-context-problem>
- <https://manus.im/docs/features/wide-research>
- <https://x.com/peakji/status/2002023656881611156>(同 action space 澄清)
- <https://x.com/peakji/status/1948060791636410404>(context engineering 过拟合)
- <https://rlancemartin.github.io/2025/10/15/manus/>(LangChain webinar 笔记,二手)
- <https://www.philschmid.de/context-engineering-part-2>(同 webinar,二手)

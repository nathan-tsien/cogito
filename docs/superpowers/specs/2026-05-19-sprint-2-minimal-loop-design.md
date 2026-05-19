# Sprint 2 · Minimal Loop — 设计 Spec

> **Status**: Accepted (2026-05-19)
> **Sprint**: v0.1 · Sprint 2 (per [ROADMAP.md](../../../ROADMAP.md))
> **Authors**: qiannengsheng + AI brainstorm partner

本文件是 Sprint 2 的**决策讨论轨迹**。可执行契约住在 durable 文档里
（`ARCHITECTURE.md` / `docs/components/H0X-*.md` / `docs/adr/`），每节末尾给出链接。
此 spec 解释 **why**；durable 文档定义 **what**。

---

## 1 · Sprint 目标

让 `cogito chat` 端到端跑通：用户输入文本 → Anthropic 或 OpenAI-Compat 模型流式响应
→ 模型调用 `read_file` 工具 → 工具结果回灌后续轮 → 直到 `end_turn`。

事件全量入 JSONL，崩溃可由日志恢复（resume 真实逻辑 Sprint 3 做；
Sprint 2 只保证"事件序列完整、可被未来 H03 消费"）。

私有部署测试场景要求第二适配器：OpenAI-Compatible Chat Completions API
（vLLM / SGLang / Azure OpenAI / 自家网关都按这个走）。原 ROADMAP Sprint 5
的 OpenAI Responses API 适配器 **不在** Sprint 2 范围。

---

## 2 · 决策轨迹（Q1–Q5）

### Q1 · `ModelGateway` trait 形态与 `ModelEvent` 变体

**讨论的备选**：

- **A · 纯 Stream**：`async fn stream(input, ctx) -> BoxStream<Result<ModelEvent, ModelError>>`。
  Codex 风格；gateway 只产事件流，H06 累积 `ModelOutput`。
- **B · Callback**：`async fn call(input, ctx, sink: impl Sink<ModelEvent>) -> ModelOutput`。
  gateway 自驱循环，H06 失去 stream 控制权。
- **C · Handle 组合体**：`async fn call -> ModelCall { events: BoxStream, ... }`。
  为未来边带元数据预留位置；v0.1 是 YAGNI。

**决策 A**。理由：

1. H06 现有签名（`demux<S: Stream<Item = ModelEvent>>(...) -> ModelOutput`）天然 1:1。
2. Anthropic SSE 的 `message_delta` 本就携带 `stop_reason + usage`，让它作为流末尾事件
   最自然——映射成 `ModelEvent::MessageCompleted` 即可。
3. 取消语义 = `drop(stream)`，跟 ADR-0006 §3 的 cooperative cancel 一致。
4. v0.2 加边带元数据再升 C 是一个 PR 的成本，现在不预付。

**`ModelEvent` 变体集 · 谁负责 buffer**：

- **X · Gateway 预聚合（采纳）**：gateway adapter 在收到 `content_block_stop` /
  `finish_reason: tool_calls` 时发出 `*Completed { full_payload }`；H06 无状态。
- **Y · H06 累积**：H06 自己按 block_index 维护 buffer；gateway 只发 delta + 边界 marker。

**决策 X**。理由：

1. tool_use 的 `input_json_delta` 反正必须 gateway buffer（部分 JSON 不可解析），
   text 一并 gateway 收口是对称设计。
2. H06 单一职责：normalize + 写 H02 + 累积 `ModelOutput`，不维护逐块缓冲。
3. Anthropic SSE `content_block_stop` 与 OpenAI `finish_reason: tool_calls` 都是
   清晰的边界事件，gateway 自然能识别。
4. "重复传输 text" 的成本在每块 KB 级，可忽略。

**最终变体**：

```rust
#[non_exhaustive]
pub enum ModelEvent {
    TextDelta { block_index: u32, chunk: String },               // 推 broadcast
    TextBlockCompleted { block_index: u32, text: String },       // 触发 H02 落盘
    ToolUseStarted { block_index: u32, call_id: String, name: String },
    ToolUseCompleted { block_index: u32, call_id: String, name: String, args: serde_json::Value },
    MessageCompleted { stop_reason: StopReason, usage: Usage },
}
```

**关键 schema 决定**：

- **Message 用 `Vec<ContentBlock>`** 而非 H04 旧文档里 `User(String) / Assistant{text, tool_calls} / ToolResult{call_id, result}` 三元组。
  - tool_result 走 `ContentBlock::ToolResult` 内嵌进 `Message::User` 消息（跟 Anthropic wire 1:1）。
  - OpenAI Chat Completions 适配器在 encode/decode 时拆并合（拆出独立的 `role: "tool"` 消息）。
- **`ToolDescriptor` 直接复用**进 `ModelInput.tools`——adapter 序列化时映射成 provider 格式。
- **`ExecCtx.deadline` 用 `Instant`** 而非 `Duration`：actor 算一次截止时刻传下去，工具检测 `Instant::now() > deadline` 简单。
- **`DispatchOutcome::AsyncJob` v0.1 留 stub 不剔除**——避免 Sprint 4 二次修改 enum 形状。

→ 具体契约：[ARCHITECTURE.md §Trait contracts](../../../ARCHITECTURE.md#trait-contracts-in-cogito-protocol) ·
   [docs/components/H06-stream-demux.md](../../components/H06-stream-demux.md)

### Q2 · `HarnessStrategy` v0.1 字段集

三档：Min（4 字段）/ Mid（+ tool_order + max_turns）/ Wide（+ length_budget + allow_async_tools + parallel_dispatch）。

**决策 Mid**。理由：

- `max_turns` Sprint 2 真需要：没有它一旦 tool loop 失控直接耗光 token / 死循环；
  default 16 够用。
- `tool_order` 极便宜，对 prompt cache 命中率有实质影响；现在加比 Sprint 5 加省事。
- `length_budget` Sprint 2 还没真截断逻辑（H04 v0.1 doc 写 "oldest-first" 但具体策略推到 H11/ADR-0008）→ 留给 Sprint 7。
- `allow_async_tools` Sprint 2 根本没异步工具 → Sprint 4 跟 JobManager 一起加。
- `parallel_dispatch` v0.1 必须 false → 不显式存。

`ToolFilter` 选 `enum { All, Allow(Vec<String>) }` 而非 `Option<Vec<String>>`：
"wildcard" 和 "empty list" 语义不同，显式枚举避免歧义。

→ 具体契约：[docs/components/H10-strategy-selector.md](../../components/H10-strategy-selector.md) ·
   `cogito-protocol::strategy`

### Q3 · cogito-model 适配器实现

三个备选：A · 手卷 reqwest+SSE ｜ B · 用 community `anthropic-sdk` ｜ C · 抽独立 wire crate。

**决策 A，含 `eventsource-stream`**。理由：

- workspace 已有 `reqwest` (with `stream` feature) + `tokio-stream` + `futures` + `async-stream`。
- Rust 没官方 Anthropic SDK；第三方 crate 维护节奏不一，schema 不对位反正要写 mapping。
- AGENTS.md "不引 framework / 慎加依赖" 明确建议自卷。
- `eventsource-stream` 0.2 是 ~500 行的纯 parser，依赖只有 `nom` + `futures`，比自卷 SSE 行解析稳。

**OpenAI-Compatible 范围扩展**（来自用户私有部署测试需求）：

- Sprint 2 同时上 **Anthropic + OpenAI-Compat (Chat Completions)** 两个 adapter，都满足同一 `ModelGateway`。
- OpenAI Chat Completions（不是 Responses API）——这是 vLLM / SGLang / Azure / 自家网关的通用契约。
- Sprint 5 多模型策略时再考虑 OpenAI 官方的 Responses API（届时新增 adapter 或升级）。

**关键 schema 映射**：

| Cogito 内部 | Anthropic | OpenAI Chat Completions |
|---|---|---|
| `Message::User { content: [Text] }` | 直 1:1 | `{role: "user", content: text}` |
| `Message::Assistant { content: [Text, ToolUse] }` | 直 1:1 | `{role: "assistant", content: text, tool_calls: [...]}` |
| `Message::User { content: [ToolResult] }` | User 消息内嵌 `tool_result` block | 拆成独立的 `{role: "tool", tool_call_id, content}` 消息（必须紧跟生成它的 assistant 消息后） |
| `ModelEvent::TextDelta` | `content_block_delta.text_delta` | `choices[].delta.content` |
| `ModelEvent::ToolUseStarted` | `content_block_start { tool_use }` | `choices[].delta.tool_calls[i].id+function.name` 首块 |
| `ModelEvent::ToolUseCompleted` | `content_block_stop` (tool_use 块) | `finish_reason: tool_calls` 一次性收口所有 tool_calls |

**OpenAI-compat tool_calls 收口陷阱**：`finish_reason: tool_calls` 一发就把"本消息所有 tool_call"
全部封口；adapter 内部按 `call_id` buffer，finish 时一次性 emit 多个 `ToolUseCompleted` + 一个 `MessageCompleted`。

**block_index 处理**：Anthropic 原生提供；OpenAI-Compat 适配器伪造（text → 0；tool_calls 按
`tool_calls[i].index` 顺序 1, 2, ...）。

**配置类型**：

```rust
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,              // default "https://api.anthropic.com"
    pub anthropic_version: String,     // default "2023-06-01"
    pub timeout: Duration,             // default 5min
}

pub struct OpenAiCompatConfig {
    pub api_key: Option<String>,        // None ⇒ 不带 auth header（私有部署适用）
    pub base_url: String,               // 必填，如 "http://vllm:8000/v1"
    pub auth_header: String,            // default "Authorization"
    pub auth_scheme: String,            // default "Bearer"
    pub timeout: Duration,
}
```

→ 具体契约：`cogito-model::anthropic` · `cogito-model::openai_compat`

### Q4 · `SessionActor::actor_main` select! 拓扑

三种：I · 单 select 含 `biased` 条件 arm｜II · 两路 select + try_poll turn｜III · `FuturesUnordered<JoinHandle>`。

**决策 I**。理由：

- v0.1 一个 actor 只跑一个 turn，类型最直白。
- `biased` 把"turn 完成事件优先于新输入"的 FIFO 语义钉死——避免 turn 收尾同时收到 input 时的语义歧义。
- `wait_active_turn` 一个 helper 搞定 Option-dance。

```rust
loop {
    tokio::select! {
        biased;
        outcome = wait_active_turn(&mut state), if state.has_active_turn() =>
            state.on_turn_complete(outcome)?,
        Some(cmd) = mailbox_rx.recv() => state.on_command(cmd).await?,
        Some(evt) = job_completion_rx.recv() =>
            mailbox_tx.send(evt.into()).await?,   // 转 SessionCommand::JobCompleted
    }
}
```

**配套决定**：

1. **JoinError 映射**：TurnDriver task panic 被 catch_unwind 兜住，到 `turn_join.await` 几乎拿不到 JoinError；
   边缘情况映射为 `TurnOutcome::Failed { reason: ActorPanicked }`。
2. **`SessionCommand::Shutdown` 处理**：drain 模式——fire cancel token，等 turn_join 完成或 deadline，
   再退出（ADR-0006 §3 cooperative）。

→ 具体契约：[ADR-0006](../../adr/0006-runtime-h01-execution-model.md) · `cogito-core::runtime::actor`

### Q5 · `TurnState` FSM 表达

ADR-0006 §"Consequences" 已定调 typed-state enum。剩余三个细分决策：

**Q5a · 字段切分**：P · 纯 per-variant ｜ **H · Hybrid（采纳）**

```rust
#[derive(Clone)]
pub struct TurnCtx {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub exec_ctx: ExecCtx,
    pub strategy: HarnessStrategy,
}

pub enum TurnState {
    Init { ctx: TurnCtx, resume: ResumeDecision },
    ContextManaged { ctx: TurnCtx, context_decision: ContextDecision },
    PromptBuilt { ctx: TurnCtx, input: ModelInput, surface: Vec<ToolDescriptor> },
    ModelCalling { ctx: TurnCtx, stream: BoxStream<...>, accumulator: ModelOutputBuilder, surface: Vec<ToolDescriptor> },
    ModelCompleted { ctx: TurnCtx, output: ModelOutput, surface: Vec<ToolDescriptor> },
    ToolDispatching { ctx: TurnCtx, pending: VecDeque<ToolInvocation>, completed: Vec<(String, ToolResult)>, surface: Vec<ToolDescriptor> },
    Completed { final_assistant_content: Vec<ContentBlock> },
    Paused { job_id: JobId, paused_at_event_id: EventId },
    Failed { reason: TurnFailureReason },
}
```

**TurnCtx 纪律**：只放满足两条件的字段——(a) 整个 turn lifetime 不变；(b) 至少被 3 个 transition 用到。
未来 v0.4 加 `tenant` 进 `ExecCtx`，自动也进 ctx；不需要改其他变体。

**Q5b · 转换代码位置**：R · 单 run() 大 match ｜ M · 方法挂 enum

**决策 R**。理由：

- transition 不是状态的"行为"——它需要 `&TurnDeps`（持有 ModelGateway / ToolProvider / StepRecorder），
  这些 deps 属于 H01 / Runtime，让状态自己 step 是逻辑倒置。
- 单一 `run()` 让 FSM 拓扑可视；"先写 event 再迁移"纪律集中执行。
- 未来文件涨大时拆 `transitions/<state>.rs` 自然，拆 impl 块在 Rust 里不优雅。

**Q5c · Resume 入口形态**：S · H03 直接构造 TurnState ｜ T · H03 出 ResumeDecision ｜ **Hybrid（采纳）**

```rust
pub enum ResumeDecision {
    FreshTurn,
    ResumeFromToolDispatching { pending: Vec<ToolInvocation>, completed: Vec<(String, ToolResult)>, surface_snapshot: Vec<ToolDescriptor> },
    ResumeFromModelCompleted { output: ModelOutput, surface_snapshot: Vec<ToolDescriptor> },
}

pub fn replay(events: &[ConversationEvent]) -> Result<ResumeDecision, ResumeError> {
    Ok(ResumeDecision::FreshTurn)    // Sprint 2 stub；Sprint 3 真做
}

pub async fn enter_turn(decision: ResumeDecision, ctx: TurnCtx, deps: TurnDeps) -> TurnOutcome {
    let initial = match decision {
        ResumeDecision::FreshTurn => TurnState::Init { ctx, resume: ResumeDecision::FreshTurn },
        ResumeDecision::ResumeFromToolDispatching { pending, completed, surface_snapshot } =>
            TurnState::ToolDispatching { ctx, pending: pending.into(), completed, surface: surface_snapshot },
        ResumeDecision::ResumeFromModelCompleted { output, surface_snapshot } =>
            TurnState::ModelCompleted { ctx, output, surface: surface_snapshot },
    };
    run(initial, &deps).await
}
```

理由：保住 S 的"TurnState 即其语义"的不变性 + 拿到 T 的"H03 输出可日志/debug"的好处；
翻译层 `enter_turn` 就 ~30 行 mechanical mapping。

**`TurnDriver` 命名三层级**（在 H01 文档详述）：

| 层级 | 名字 | 是什么 |
|---|---|---|
| 设计概念 | `H01 Turn Driver`（大写） | 11 组件之一 |
| Rust 实现 | `harness::turn_driver` module（小写蛇形） | 含 `TurnState` / `run()` / `enter_turn()` / `transitions/*` |
| tokio runtime 实体 | "TurnDriver task" | SessionActor 每次 spawn 的短生命任务 |

→ 具体契约：[docs/components/H01-turn-driver.md](../../components/H01-turn-driver.md) ·
   `cogito-core::harness::turn_driver`

---

## 3 · Sprint 2 交付清单

| Crate / 文件 | 交付 | 估计 LoC |
|---|---|---|
| `cogito-protocol` | `gateway` / `strategy` / `exec_ctx` 模块；事件 payload 扩展（`ContextManageEntered`/`Completed`、`PromptComposed`、`ModelCallStarted`/`Completed` 等） | ~600 |
| `cogito-model` | `AnthropicGateway` + `OpenAiCompatGateway` + 共享 SSE helper | ~900 |
| `cogito-tools` | `BuiltinToolProvider` + `CompositeToolProvider` + `read_file` | ~250 |
| `cogito-core::harness::turn_driver/` | state · deps · run · enter_turn · transitions/*7 | ~700 |
| `cogito-core::harness::{prompt,tool_surface,tool_resolver,dispatcher,stream_demux,strategy,hooks,resume}` | H04 · H05 · H07 · H08 sync · H06 · `default_with_model` · hooks no-op · H03 stub | ~600 |
| `cogito-core::runtime::actor` | `actor_main` (Topology I) · `try_start_turn` · `on_turn_complete` · Shutdown drain | ~400 |
| `cogito-core::runtime::{builder,handle}` | `open_session` / `send_user` / `cancel_turn` / `shutdown` 接通 `todo!()` | ~150 |
| `cogito-cli chat` | 子命令 + REPL + 手动 E2E 冒烟 | ~250 |
| `cogito-mock-model` | 脚本化 `ModelEvent` 流 | ~200 |
| Durable docs | ARCH/H01-H10 跟踪本 sprint 实际形状（详见下节） | prose |

**新依赖**（加 workspace `Cargo.toml`）：
- `eventsource-stream = "0.2"`
- `tokio-util` 给 `cogito-protocol` 增 `sync` feature（CancellationToken）

**不在 Sprint 2 范围**：
- H03 真实 replay 逻辑（Sprint 3）
- H08 async path · 真 JobManager（Sprint 4）
- H11 真实现（永远 pass-through 直到 ADR-0008）
- H09 真 hook（Sprint 2 只留 pre_prompt / pre_dispatch / post_model / post_turn / on_error 的 no-op 插槽）
- chaos test（Sprint 3）
- TUI（Sprint 6）

---

## 4 · 事件 payload 新增

复用 Sprint 1 已有；新增（加 `EventPayload` enum 变体，`#[non_exhaustive]` 不破坏 schema）：

| 新增 payload | 何时写 | 字段 |
|---|---|---|
| `ContextManageEntered { turn_id }` | Init → ContextManaged | turn_id |
| `ContextManageCompleted { turn_id }` | ContextManaged → PromptBuilt | turn_id（v0.1 pass-through 直接收口） |
| `PromptComposed { turn_id, model: String, surface_size: u32 }` | ContextManaged → PromptBuilt | model_id + 工具数（**不**含完整 prompt） |
| `ModelCallStarted { turn_id, model: String }` | PromptBuilt → ModelCalling | model_id |

**关键取舍**：`PromptComposed` **不存完整 prompt**。事件日志若要做 byte-level resume 必须存，但代价是每轮 +KB-MB 日志体积。按 ADR-0007 "event log 是状态恢复源不是 prompt 缓存"原则，**不存**，resume 时 H04 从历史事件重新 compose 即可。

---

## 5 · PR 切分（9 个）

依赖关系下，关键路径：P1 → P5 → P6 → P7 → P8 → P9。其余可并行。

| # | PR | 主体 | LoC | 依赖 |
|---|---|---|---|---|
| **P1** | `cogito-protocol`: gateway/strategy/exec_ctx + payload 扩展 + 文档同步 | protocol + docs | ~800 | 无 |
| **P2** | `cogito-tools`: BuiltinToolProvider + read_file + CompositeToolProvider · `cogito-mock-model` 同步 | tools + testing | ~450 | P1 |
| **P3** | `cogito-model`: shared SSE + AnthropicGateway（含 SSE fixture 回放） | model | ~450 | P1 |
| **P4** | `cogito-model`: OpenAiCompatGateway（含 vLLM/SGLang fixture） | model | ~450 | P1 |
| **P5** | `cogito-core::harness`: H04 + H05 + H07 + strategy::default + hooks no-op | core/harness | ~400 | P1 |
| **P6** | `cogito-core::harness`: H06 + H08 sync + dispatcher | core/harness | ~450 | P1, P2, P5 |
| **P7** | `harness::turn_driver/`: state + deps + run + enter_turn + transitions/* + H01 文档大幅细化 | core/harness + docs | ~700 | P1, P5, P6 |
| **P8** | `runtime::actor`: actor_main (Topology I) + `open_session` + `SessionHandle` 接通 | core/runtime | ~550 | P3 (或 P4) + P7 |
| **P9** | `cogito-cli chat` + 手动 E2E + Sprint 2 closure（CHANGELOG + ROADMAP 勾选） | cli + docs | ~300 | P8 |

P3 / P4 / P5 互不依赖，可在 P1 合并后并行。

---

## 6 · 文档同步清单

durable 优先；spec 只放讨论轨迹。本 design branch（`design/sprint-2-minimal-loop`）一并落地以下文档：

| 文件 | 改什么 |
|---|---|
| `ARCHITECTURE.md` | §"Turn state machine" 加 FSM primer 链接 · §"Trait contracts" 添 `ModelGateway` / `HarnessStrategy` / `ExecCtx` 行 |
| `docs/components/H01-turn-driver.md` | 加 "What is a Finite State Machine here?" 章节 · 加 "Module structure" + "Call graph" 章节 · "Implementation note" 大幅细化 · 移除 v0.1 🚧（Sprint 2 实现） |
| `docs/components/H02-step-recorder.md` | 修正内部矛盾：invariant #2 + impl note 中"200ms / 500 char window"的残留旧表述（与下方 "Text block lifecycle" + AGENTS.md §2 冲突），统一为 per-content_block-boundary |
| `docs/components/H04-prompt-composer.md` | Message 改 `Vec<ContentBlock>`（弃 `User(String)/Assistant{text, tool_calls}/ToolResult{call_id, result}` 三元组）· 校准 history projection 表 |
| `docs/components/H05-tool-surface.md` | 标注 `tool_order` 字段已是 Sprint 2 落地范围 |
| `docs/components/H06-stream-demux.md` | **删除 invariant #3 中"200ms / 500 char window"过时表述**（Sprint 1 已改成 per-content_block-boundary）· 改成 "gateway-preaggregated `*Completed` events trigger H02 record" |
| `docs/components/H07-tool-resolver.md` | 标注 `ResolvedCall` / `ToolInvocation` 住在 `cogito-core::harness::tool_resolver`，不在 protocol |
| `docs/components/H08-tool-dispatcher.md` | 标注 sync 路径 Sprint 2 落地、Async path 仍 Sprint 4 · `DispatchOutcome` 住 harness |
| `docs/components/H09-hook-pipeline.md` | 标注 Sprint 2 只接 no-op 插槽，真 hook policy Sprint 6 |
| `docs/components/H10-strategy-selector.md` | 修正 v0.1 scope：Sprint 2 提供 `default_with_model` factory，YAML loader 仍 Sprint 5 · 字段集对齐 Mid 版本 |
| `docs/adr/0006-runtime-h01-execution-model.md` | 末尾 Follow-on 加 Sprint 2 备注：`tokio_util::sync::CancellationToken` 进 cogito-protocol 的层级决定 |
| `CHANGELOG.md` | "Sprint 2 · Minimal Loop" 章节（P9 才填） |
| `ROADMAP.md` | Sprint 2 各勾选项 ✅（P9 才勾） |

**ADR 评估**：Sprint 2 没引入新原则性决策——`ModelGateway` / `ExecCtx` 等都是 ADR-0006 已规划的具象化。
**不新增 ADR**，但在 ADR-0006 末尾加 Follow-on 备注记录两件事：
1. `tokio_util` 进 protocol 的层级允许。
2. Sprint 2 把 OpenAI-Compat (Chat Completions) 提前到 v0.1（而非 Sprint 5）以满足私有部署测试需求。

---

## 7 · 测试策略

| 组件 | 单测 | 集成 / 快照 |
|---|---|---|
| H04 compose | 空 history / 单轮 / 多轮 / 含 tool_result / 超长度 | Insta snapshot：金标准事件日志 → ModelInput JSON |
| H05 surface | `ToolFilter::All` / `Allow(...)` / 空 / 不存在的名字 | order-stable property |
| H06 demux | 文字 / tool_use / 混合 / 截断 | 合成 ModelEvent 流 → H02 事件序列金标准 |
| H07 resolve | 合法 / 缺字段 / 多字段 / 错类型 / 非 JSON / 未知工具 / 重复 call_id | snapshot：LLM-facing message 固定 |
| H08 dispatch | 成功 / 未知工具 / panic / 取消 / pre_dispatch reject | 集成 `BuiltinToolProvider + read_file` |
| H01 turn_driver | 每 transition 单测（mock deps） | 全 turn 集成 with `cogito-mock-model`：纯文本 / 单工具 / 多工具 / 超 max_turns |
| SessionActor | `try_start_turn` / `on_turn_complete` 单测 | open_session → send_user → 完成；cancel_turn 期间 ModelCalling；shutdown drain；ctrl-C |
| AnthropicGateway | encode/decode snapshot | **录制 SSE fixture** 重放 |
| OpenAiCompatGateway | 同上（vLLM + SGLang 各录一份） | 同上 |
| `cogito-cli chat` | n/a | **手动** E2E 冒烟：真 Anthropic 1 次 + 真 vLLM 1 次（CI 不跑） |

---

## 8 · 风险表

| 风险 | 触发 | 缓解 |
|---|---|---|
| Anthropic tool_use 的 `input_json_delta` 在 content_block_stop 前可能解析失败 | 模型生成不完整 JSON | adapter 收 stop 后 try_from_str；失败 emit `ToolUseCompleted { args: Value::Null }`，H07 反正会判 SchemaMismatch |
| OpenAI-Compat `finish_reason` 字符串各家不同 | 老版 vLLM / SGLang / Azure | adapter 大小写不敏感匹配 + unknown → `MaxTokens`（最保守降级） |
| 高频 TextDelta backpressure | 模型 500+ chunks/sec | persist 通道 capacity 256；Anthropic 实际 ~20-30 chunks/sec。Sprint 7 加 P99 latency benchmark 守 |
| cancel_turn 在 ModelCalling 期间触发，HTTP 流是否真断 | reqwest stream drop 行为 | reqwest 0.12 drop stream 会 abort connection；写 cancel-mid-stream 集成测试守 |
| Multi-handle session 行为 | 两个 handle 同时 send_user | builder.rs 已规划 `SessionAlreadyOpen`；测试覆盖 |
| Anthropic prompt caching 不支持 | v0.1 不传 cache_control | 文档明示 Sprint 5/6；ModelInput 设计预留 `cache_breakpoints: Option<...>` 字段不破坏现有接口 |

---

## 9 · 引用与背景

- [ROADMAP.md](../../../ROADMAP.md) §"Sprint 2 · Minimal Loop"
- [ARCHITECTURE.md](../../../ARCHITECTURE.md) §"The 11-component Brain" / §"Turn state machine" / §"Trait contracts"
- [AGENTS.md](../../../AGENTS.md) §"Inviolable design principles" #1-#7
- [ADR-0003](../../adr/0003-state-machine-turn-driver.md) — FSM Turn Driver 立项
- [ADR-0006](../../adr/0006-runtime-h01-execution-model.md) — Runtime + H01 execution model（含 2026-05-19 `ContextManaged` amendment）
- [ADR-0007](../../adr/0007-event-log-as-cross-language-contract.md) — Event log 跨语言契约
- `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` — ADR-0006 的完整讨论
- [docs/components/H01-turn-driver.md](../../components/H01-turn-driver.md) — Sprint 2 实施时大幅细化

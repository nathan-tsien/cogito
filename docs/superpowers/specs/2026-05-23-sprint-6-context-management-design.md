# Sprint 6 · Context Management — 设计 Spec

> **Status**: Draft (2026-05-23) — pending review;待 ratify 后落地 ADR-0008
> **Sprint**: v0.1 Sprint 6 (per ROADMAP.md — Sprint 6: Context Management / ADR-0008 + C2 trait freeze + first Compactor, 2–2.5 day estimate)
> **Authors**: qiannengsheng + AI brainstorm partner
> **Predecessors**: rebalance spec (2026-05-22 §2.1 C2 决策) + H11 doc(架构槽位 + open question 列表) + ADR-0006 amendment(`ContextManaged` FSM state)
>
> 本文件是 Sprint 6 的**决策与契约 spec**。最终可执行契约住在:
> ADR-0008(规范级) + 各 trait / event 类型(`cogito-protocol`) +
> `cogito-context` 实现 + `docs/components/H11-context-manage.md`(组件文档)。
> 此 spec 解释 **why** 与 **怎么落地**;ADR / 类型 / 实现 / 组件文档定义 **what**。

---

## 1 · Status / Scope

### 1.1 In-scope(Sprint 6 必交付)

- **ADR-0008** 起草并 ratify:Context Management 总体设计、trait freeze、event 契约、Compactor 选型策略
- **`cogito-protocol`** 新增类型(全部 additive,不触发 `SCHEMA_VERSION` bump,符合 ADR-0007 §"forward-compat"):
  - 4 个 trait:`Compactor` / `HistoryProjector` / `SystemPromptInjector` / `ToolFilterOverrider`
  - 4 个事件 variant:`ContextCompacted` / `SystemPromptInjected` / `ToolFilterOverridden` / `ContextDecisionRecorded`
  - 4 个 tagged config enum + `ContextConfig` 顶层结构,作为 `HarnessStrategy.context` 字段
  - `ContextPipeline` 装配体类型 + 辅助类型(`CompactionInput` / `CompactionApplied` / `CompactionReplacement` / `ToolFilterOverrideMode` / `ContextError` 等)
- **`cogito-context`**(新 umbrella crate):
  - 4 个 no-op 默认实现(`NoneCompactor` / `StandardProjector` / `NoneInjector` / `NoneOverrider`)
  - `TruncateCompactor`(v0.1 唯一非平凡 Compactor)
  - `build_pipeline(&ContextConfig) -> ContextPipeline` 工厂(per CLAUDE.md §"Tagged-config factories")
- **`cogito-core::harness`**:
  - H11 `transitions::context_managed.rs` 由 pass-through 升级为真实工作流(4 trait orchestration + 失败降级 + 总结条事件)
  - H04 通过 `HistoryProjector` 投影历史(§ 2 算法);兼容旧无压缩 session
  - H05 在 PromptBuilt 阶段读取本轮 `ToolFilterOverridden` 事件,与 `strategy.allowed_tools` 求交集 / 替换
- **`cogito-core::runtime`**:`open_session` 调 `build_pipeline` 装配 `ContextPipeline` 并注入 `SessionShared`
- **`cogito-protocol::store`** 行为约束(StepRecorder 写入校验):
  - `record_context_compacted` 校验 §5.5 invariants(turn 边界 / 自指禁止 / 单 turn 一压缩)
  - 不变量违反 → `StoreError::InvariantViolated`,Compactor 降级,H11 不阻断 turn
- **测试**:§13 整套
- **文档**:`docs/components/H11-context-manage.md` 从 placeholder 升级为正式组件文档;`docs/data-model/jsonl-v1.md` 加 4 个新 variant 描述;H04 / H05 component doc 注脚说明新读取/调用点

### 1.2 Out-of-scope(Sprint 6 不做)

- 任何 **summarize 类** Compactor:留 v0.2(需要 summarization model 注入约定、cascading max_depth、prompt template 设计)
- CLI / strategy.yaml 加载层暴露 `context` 配置:留 Sprint 9(multi-model strategy + YAML registry)
- Hook(H09)上的 `pre_context` / `post_context` lifecycle 点:留 v0.2(Sprint 5 已完成 H09 实化,Sprint 6 仅在失败路径走 `pre_prompt` 钩子既有路径)
- Skill / Plugin 提供 `SystemPromptInjector` 实现:留 Sprint 7(Skill)/ Sprint 12(Plugin)
- `ContextBlock::Image` 多模态摘要替换:留 v0.5(Storage + Multimodal 主题版本)
- Token 估算精度提升(provider tokenizer 抽象):**永不入** cogito-protocol;留给个别 Compactor 实现按需做(避免拖入 tiktoken 等大依赖)
- v0.4 Postgres backend 物理分表:本 spec 只在 §4 给出 backend partitioning guidance,落地是 v0.4 工作

### 1.3 与既有 ADR 的关系

- **不破**:ADR-0004 / ADR-0006 / ADR-0007 / ADR-0019
- **延展**:ADR-0006 的 `ContextManaged` 状态从"pass-through 占位"升级为"真实工作流";延展不需要 amendment(原本就声明留 ADR-0008 填充)
- **接管**:ADR-0008 原列为 "TBD post-Sprint-2 spike",rebalance 升级为 v0.1 Sprint 6 正式 Accepted;本 spec 即 ADR-0008 起草输入

---

## 2 · 背景:为什么 Sprint 6 是 v0.1 关键路径

### 2.1 上游约束:Skill 注入与 H04 投影都需要 trait freeze

- **Sprint 7 (Skill loader)** 需要在每轮 prompt 中注入 `SKILL.md` 内容——这是典型 `SystemPromptInjector` 用例。Skill loader 必须有稳定 trait 才能 ship 实现。
- **Sprint 9 (Multi-model + TUI)** 需要按 strategy 切换 Compactor(长上下文 model 用一种,短上下文 model 用另一种);需要 `ContextConfig` 在 strategy.yaml 中可声明。
- **v0.2 (Plugin)** 让团队成员以**用户态**方式贡献 Compactor / Injector / Overrider 实现(打包成 plugin);ADR-0008 的 trait surface 一旦冻结,Plugin 作者面对的是稳定 API。

不在 Sprint 6 把这些 trait 确定下来,后续每个版本都会要求 amendment——**这是 rebalance 把 ADR-0008 升级为 v0.1 正式工作**的核心理由(rebalance §2.1 C2 决策)。

### 2.2 团队协作:边界清晰是头等需求

Context Management 内涵复杂——压缩策略、投影策略、注入策略、tool 收窄策略各成体系。若 cogito-core 内部用单一巨型函数实现,新成员无法在不读完整个模块的前提下增加新策略。Sprint 6 的核心交付不是 truncate 本身,**而是 4 个 trait + 4 个事件的契约表面**——团队成员能照着抄即可在 v0.1 之后写第 5 个 Compactor 或第 2 个 Injector。

ADR-0008 ratify 之后,**新增任何 context 子组件都不需要再回头改 ADR**——这是"trait freeze"的实质意义。

### 2.3 已实化的 H09 Hook Pipeline 不替代 Context Manage

Sprint 5 已实化 H09 hook(`docs/components/H09-hook-pipeline.md`),hook 是**纯策略 gate**(Allow / Modify / Reject),无 I/O 权限。Context Manage 是 hook 不能做的工作:

| 维度 | H09 Hook | H11 Context Manage |
|---|---|---|
| I/O 权限 | 无 | 有(Compactor 可调 ModelGateway) |
| 写事件 | 无(纯 gate) | 有(4 类事件) |
| 决策粒度 | per request / per tool call | per turn |
| 失败语义 | Reject = 阻断 | Degrade = 不阻断 |
| 扩展点 | 5 个生命周期点(pre_prompt 等) | 4 个 trait,每个独立扩展 |

二者互补,**不能用 Hook 替代 Context Manage**——这也是 H11 单独存在的合法性来源(H11 doc §"Why this component exists" 的第 1/2/3 条)。

---

## 3 · 设计原则:边界即不变量

四个 trait 的切分依据是**工作种类与不变量差异**,不是按数据流分段。每一刀对应一个硬约束:

| Trait | 工作种类 | 允许做什么 | 不允许做什么 | 不变量来源 |
|---|---|---|---|---|
| **Compactor** | 策略 + I/O | 调 `ModelGateway` 做摘要;通过 H02 写 `ContextCompacted` 事件;读 token usage | 改 system prompt / tool 列表;直接改 H04 输出 | AGENTS.md §3(状态在 Store)+ §6(Brain 通过 Protocol 看 Hand) |
| **HistoryProjector** | 纯函数 | 读事件日志(含 Compactor 写下的压缩事件);按 strategy 投影成 `Vec<Message>` | I/O;写事件;读外部状态 | H04 既有纯函数 invariant(`docs/components/H04-prompt-composer.md`) |
| **SystemPromptInjector** | 纯函数(可 async,因 Sprint 7 Skill 可能跨 fs I/O) | 拼接日期 / locale / tenant / Skill 内容到 system 段;写 `SystemPromptInjected` 事件 | 改 history 投影规则;改 tool 列表 | 单一职责:产文本,不感知历史结构 |
| **ToolFilterOverrider** | 纯函数(可 async,同上) | 在 strategy.allowed_tools 之上做交集 / 替换;写 `ToolFilterOverridden` 事件 | 改 history / system prompt | 单一职责:产 filter,不感知历史结构 |

**合并任何两个都破坏其中一边的 invariant**:
- Compactor + HistoryProjector 合一 → Projector 被赋予 I/O 权限,破坏纯函数
- HistoryProjector + SystemPromptInjector 合一 → Injector 被迫感知历史投影,职责不清
- Compactor + Injector 合一 → 一个 trait 同时做"压缩历史"和"产 system 文本",新人难以理解

第四个 trait `ToolFilterOverrider` 的合法性:H11 doc 列出的四个 context 关注点中,**tool surface override 没有家**(strategy.allowed_tools 是静态,H05 不接受动态输入)。第 4 trait 把它接住——v0.2 Plugin / v0.3 Subagent 一定要它,**现在 ~80 LoC 多花在 Sprint 6,胜过未来 amend ADR-0008**。

---

## 4 · 数据流:Strict event-sourcing + 协议-后端分层

### 4.1 (P) 模式:四个 trait 全部通过事件流通信

H11 ContextManaged 转移期间,每个 trait 的输出**通过持久化事件**承载,而非内存值传递。理由:

- **审计完整**:任何一轮都能从事件日志反推"模型当时看到了什么"。v0.4 SaaS 多副本时,审计路径不需改造。
- **Plugin 可观测性**:v0.2 Plugin 提供的 Injector / Overrider 行为可直接 grep JSONL 诊断,无需读代码。
- **Resume 简单**:H03 不需要为 ContextManaged 引入新 ResumePoint——状态全在事件中。
- **跨语言契约**:Go / Python reader 看到一条流,按需 filter。

代价:每轮 ContextManaged 固定 2-3 个 trait 事件 + 2 个 FSM 事件 + 1 个总结条 = 5-6 事件,每事件 ~80-200 字节。1000 轮会话约 1MB 元数据。v0.1 dev/debug 完全可接受。

**例外**:`Compactor` 仅在实际压缩时写 `ContextCompacted`(no-op 时不写——压缩是 positive action,无操作无事件)。其余三 trait(Injector / Overrider / H11 自身的 Decision 条)**每轮必写**(即便内容为空/Inherit)。理由:Injector/Overrider 的"运行了但没改变"和"没运行(配置为 None)"在审计上**有差**——前者表明 trait 实现存在但本轮决定无作为,后者表明系统未配置;事件存在与否是这个差的唯一可靠信号。

### 4.2 (S4) 模式:协议单流 + 后端可物理分表

**协议层**:`ConversationStore::replay` 始终返回单条按 seq 排序的事件流。Brain / H03 / chaos test 全部按单流处理。

**后端层**:
- **v0.1 JSONL backend**:单文件(`sessions/<id>.jsonl`),所有事件混合写入。零变更。
- **v0.4 Postgres backend (预留路线)**:可选择将 `EventPayload::category() == HarnessMeta` 的事件路由到 `harness_meta_events` 表,与 `conversation_events` 表分离;`replay()` 在 SQL 层 `UNION ALL ORDER BY seq` 返回单流。Brain 不感知。

`EventPayload::category()` 辅助方法(本 Sprint 加,~50 LoC):

```rust
#[non_exhaustive]
pub enum EventCategory {
    /// User / model conversation events. The "what was said" record.
    Conversation,
    /// Harness FSM markers and decisions. Not part of dialog.
    HarnessMeta,
    /// Context management decisions (compaction + per-turn injection).
    ContextDecision,
}

impl EventPayload {
    pub fn category(&self) -> EventCategory {
        match self {
            Self::SessionStarted { .. }
            | Self::TurnStarted { .. }
            | Self::AssistantMessageAppended { .. }
            | Self::ToolUseRecorded { .. }
            | Self::ToolResultRecorded { .. }
            | Self::ThinkingBlockRecorded { .. } => EventCategory::Conversation,

            Self::ContextManageEntered { .. }
            | Self::ContextManageCompleted { .. }
            | Self::PromptComposed { .. }
            | Self::ModelCallStarted { .. }
            | Self::ModelCallCompleted { .. } => EventCategory::HarnessMeta,

            Self::ContextCompacted { .. }
            | Self::SystemPromptInjected { .. }
            | Self::ToolFilterOverridden { .. }
            | Self::ContextDecisionRecorded { .. } => EventCategory::ContextDecision,
        }
    }
}
```

ADR-0008 增章节 "Backend partitioning guidance":列出三类 category 的语义,作为后端实现的官方分类。**Brain 永远不依赖 category()**——它仅供后端 / 分析工具 / cross-language reader 使用。

---

## 5 · 长会话多压缩投影算法

本节是 ADR-0008 的核心算法部分,描述 `StandardProjector` 必须实现的行为。

### 5.1 输入

```
events: &[ConversationEvent]   // 全量按 seq 排序
strategy: &HarnessStrategy
current_turn: TurnId           // 即将进入 ModelCalling 的 turn
```

### 5.2 算法

```
1. covered ← {}  (RangeSet)
   for ev in events:
       if ev.payload is ContextCompacted { replaced_seq_range }:
           covered.add(replaced_seq_range)
   ※ 注意:covered 集合包含**所有** ContextCompacted 的 range,
     即使某个 ContextCompacted 事件本身后来被另一个覆盖。

2. system_suffix ← 取 events 中 turn_id == current_turn 的最新一条
                  SystemPromptInjected.suffix(无则空)
   system_text ← strategy.system_prompt + ("\n\n" + suffix if suffix else "")
   messages ← [System(system_text)]

3. 按 seq 顺序遍历 events,维护 assistant_buf 缓冲:
   for ev in events:
       if ev.seq in covered:
           continue                            ← 被覆盖,跳过

       match ev.payload:
           ContextCompacted { replacement }:
               flush(assistant_buf, messages)
               match replacement:
                   Drop: pass
                   Summary { text }:
                       messages.push(User(
                         "<conversation_summary>\n" + text + "\n</conversation_summary>"
                       ))
           TurnStarted { user_input }:
               flush(assistant_buf, messages)
               messages.push(User(user_input))
           AssistantMessageAppended { text }:
               assistant_buf.push_text(text)
           ToolUseRecorded { call_id, tool_name, args }:
               assistant_buf.push_tool_use(call_id, tool_name, args)
           ToolResultRecorded { call_id, result }:
               flush(assistant_buf, messages)
               messages.push(ToolResult(call_id, result))
           ThinkingBlockRecorded { text, signature }:
               assistant_buf.push_thinking(text, signature)   ← 见 ADR-0019 §4 顺序约定
           _:
               pass                            ← 所有其他 meta 事件忽略

4. flush(assistant_buf, messages)             ← 处理尾部未 flush 的 assistant
5. return messages
```

### 5.3 多压缩具象例(ADR-0008 必有此 trace)

会话经过两次压缩(turn 21 用 truncate,turn 61 用假设的 summarize):

```
seq=1     SessionStarted
seq=2-79  turns t1-t20 各种 conversation 事件
seq=80    ContextManageEntered{t21}
seq=81    ContextCompacted{
            turn_id:t21, range:[2,79], produced_by:"truncate",
            replacement: Drop,
            token_estimates: { before:5200, after:800 }
          }
seq=82    SystemPromptInjected{t21, suffix:"Today is 2026-05-23.", contributors:["date"]}
seq=83    ToolFilterOverridden{t21, mode: Inherit, contributors:[]}
seq=84    ContextDecisionRecorded{t21, compactions:[81], system_prompt_event:82, tool_filter_event:83, errors:{}}
seq=85    ContextManageCompleted{t21}
seq=86-399  turns t21-t60 conversation + 历次 ContextManaged 元事件
seq=400   ContextManageEntered{t61}
seq=401   ContextCompacted{
            turn_id:t61, range:[86,399], produced_by:"summarize",
            replacement: Summary {
              text:"User explored weather then follow-up forecasts; key facts: …",
              model:"claude-haiku-4-5",
            },
            token_estimates: { before:8400, after:2300 }
          }
seq=402   SystemPromptInjected{t61, suffix:"Today is 2026-05-23.", contributors:["date"]}
seq=403   ToolFilterOverridden{t61, mode: Inherit, contributors:[]}
seq=404   ContextDecisionRecorded{t61, compactions:[401], system_prompt_event:402, tool_filter_event:403, errors:{}}
seq=405   ContextManageCompleted{t61}
seq=406   PromptComposed{t61, ...}
```

H04 为 turn t61 投影时:

- `covered = [2,79] ∪ [86,399]`
- 按 seq 走:
  - seq 1: meta,忽略
  - seq 2-79: covered,跳过(t1-t20 内容消失,truncate.Drop 不替换)
  - seq 80: meta(ContextManageEntered),忽略
  - seq 81: 自身不在 covered,但 replacement=Drop → 不插入消息
  - seq 82-85: meta,忽略
  - seq 86-399: covered,跳过(t21-t60 内容被 summary 替换)
  - seq 400: meta,忽略
  - seq 401: 自身不在 covered,replacement=Summary → 发射一条 user 消息
  - seq 402-405: meta,忽略

模型实际收到的消息:

```
[System]   "You are a helpful assistant.\n\nToday is 2026-05-23."
[User]     "<conversation_summary>
            User explored weather then follow-up forecasts; key facts: …
            </conversation_summary>"
[User]     (turn t61 的用户输入,当 t61 TurnStarted 写入 log 时投影到这里)
```

### 5.4 级联语义:set-union 自然表达

- 一个新 `ContextCompacted` 的 range **可以包含**先前 `ContextCompacted` 事件的 seq;此时被覆盖的先前压缩在投影时不发射 replacement,但其 range 仍参与 covered 集合的 union。
- 这意味着:**新压缩"吸收并替代"旧压缩的呈现,但不"释放"旧压缩覆盖过的历史事件**。
- 不引入 max_depth 概念——通过 set-union 表达即可。
- truncate Compactor 的常见行为:`covered` 单调扩张,投影里只剩最近若干 turn + 当前 turn,早期 turn 彻底消失(Drop 不替换)。
- summarize Compactor 的常见行为:`covered` 同样单调扩张,但每个未被覆盖的 ContextCompacted 都在投影中发射一段 `<conversation_summary>` user 消息。模型看到多段不连续的 summary + 最近 turn + 当前 turn。

### 5.5 写入时不变量(由 StepRecorder 校验)

| Invariant | 写入校验位置 | 违反时行为 |
|---|---|---|
| `range.1 < self.seq`(禁止自指) | `record_context_compacted` | `Err(StoreError::InvariantViolated)` → Compactor 收到 `Err` → H11 降级,本轮不压缩 |
| `range.0` = 某个 `TurnStarted.seq`(turn 边界对齐起点) | 同上 | 同上 |
| `range.1` = 该 turn 的最后一个 conversation 事件 seq(turn 边界对齐终点) | 同上 | 同上 |
| 同一 `turn_id` 至多 1 个 `ContextCompacted`(v0.1 简化;v0.2+ 可放宽) | 同上 | 同上 |

`StepRecorder` 在写入前调用 helper(基于已有 in-memory history snapshot)检查这些条件;不通过则不写,返回 Err。

---

## 6 · Trait surface 完整定义

所有 trait + 类型在 `cogito-protocol::context` 模块(新增)。`cogito-core::harness` 仅 `use cogito_protocol::context::*`——符合 ADR-0004 第 6 条。

### 6.1 `Compactor`

```rust
#[async_trait]
pub trait Compactor: Send + Sync {
    /// Decide whether to compact for the upcoming turn. May invoke the model
    /// gateway for summarization-style compactors. Persists 0+ ContextCompacted
    /// events through the recorder. Returns one CompactionApplied per event
    /// written, for inclusion in the H11-emitted ContextDecisionRecorded summary.
    async fn maybe_compact(
        &self,
        input: CompactionInput<'_>,
    ) -> Result<Vec<CompactionApplied>, ContextError>;

    /// Implementation identity. Embedded in ContextCompacted.produced_by.
    fn id(&self) -> &'static str;
}

pub struct CompactionInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub history: &'a [ConversationEvent],
    pub strategy: &'a HarnessStrategy,
    pub last_usage: Option<TokenUsage>,
    pub model_gateway: &'a dyn ModelGateway,
    pub recorder: &'a mut StepRecorder,
}

pub struct CompactionApplied {
    pub event_id: EventId,
    pub replaced_seq_range: (u64, u64),
    pub kind: CompactionKind,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum CompactionKind {
    Truncate,
    Summarize,           // v0.2+
    ToolBodyElision,     // v0.2+
}
```

### 6.2 `HistoryProjector`

```rust
pub trait HistoryProjector: Send + Sync {
    /// Pure projection: events + strategy → messages for ModelInput.
    /// Implementations MUST honor ContextCompacted via the set-union covered
    /// semantics defined in §5.
    fn project(
        &self,
        events: &[ConversationEvent],
        strategy: &HarnessStrategy,
        current_turn: TurnId,
    ) -> Vec<Message>;

    fn id(&self) -> &'static str;
}
```

`HistoryProjector` 不 async、不写事件——纯函数。

### 6.3 `SystemPromptInjector`

```rust
#[async_trait]
pub trait SystemPromptInjector: Send + Sync {
    /// Compute this turn's system-prompt suffix and persist a SystemPromptInjected
    /// event. MUST write an event every turn (even when suffix is empty), per the
    /// audit semantics in §4.1.
    async fn inject(
        &self,
        input: InjectionInput<'_>,
    ) -> Result<EventId, ContextError>;

    fn id(&self) -> &'static str;
}

pub struct InjectionInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub strategy: &'a HarnessStrategy,
    pub history: &'a [ConversationEvent],
    pub exec_ctx: &'a ExecCtx,
    pub recorder: &'a mut StepRecorder,
}
```

### 6.4 `ToolFilterOverrider`

```rust
#[async_trait]
pub trait ToolFilterOverrider: Send + Sync {
    /// Decide per-turn tool filter override on top of strategy.allowed_tools.
    /// MUST write a ToolFilterOverridden event every turn (Inherit mode counts
    /// as "ran and decided not to change").
    async fn override_filter(
        &self,
        input: ToolFilterInput<'_>,
    ) -> Result<EventId, ContextError>;

    fn id(&self) -> &'static str;
}

pub struct ToolFilterInput<'a> {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub strategy: &'a HarnessStrategy,
    pub history: &'a [ConversationEvent],
    pub exec_ctx: &'a ExecCtx,
    pub recorder: &'a mut StepRecorder,
}
```

### 6.5 共享类型

```rust
#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum ContextError {
    #[error("summarization model call failed: {0}")]
    SummarizationModelFailed(#[from] ModelError),

    #[error("invariant violated: {0}")]
    InvariantViolated(String),

    #[error("operation aborted")]
    Aborted,

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}
```

### 6.6 ContextPipeline 装配体

```rust
// cogito-protocol::context
pub struct ContextPipeline {
    pub compactor: Arc<dyn Compactor>,
    pub projector: Arc<dyn HistoryProjector>,
    pub injector: Arc<dyn SystemPromptInjector>,
    pub overrider: Arc<dyn ToolFilterOverrider>,
}
```

H11 通过 `SessionShared.context_pipeline: Arc<ContextPipeline>` 拿到;装配在 `Runtime::open_session` 完成。

---

## 7 · 事件 payload 完整定义

全部在 `cogito-protocol::event::EventPayload` 中新增 variant;`EventPayload` 已是 `#[non_exhaustive]`,additive 不破 schema(ADR-0007)。

### 7.1 `ContextCompacted`

```rust
EventPayload::ContextCompacted {
    turn_id: TurnId,
    replaced_seq_range: (u64, u64),          // inclusive,turn 边界对齐
    produced_by: String,                     // Compactor::id()
    replacement: CompactionReplacement,
    token_estimate_before: Option<u64>,
    token_estimate_after: Option<u64>,
}

#[non_exhaustive]
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionReplacement {
    Drop,
    Summary {
        text: String,
        model: String,
    },
    // v0.2+: ToolBodyElided { kept_args_preview: bool, original_byte_count: u64 }
}
```

### 7.2 `SystemPromptInjected`

```rust
EventPayload::SystemPromptInjected {
    turn_id: TurnId,
    suffix: String,                          // 可空(no-op Injector 也写)
    contributors: Vec<String>,               // e.g. ["date", "skill:plan-review"]
    produced_by: String,                     // Injector::id()
}
```

### 7.3 `ToolFilterOverridden`

```rust
EventPayload::ToolFilterOverridden {
    turn_id: TurnId,
    mode: ToolFilterOverrideMode,
    contributors: Vec<String>,
    produced_by: String,                     // Overrider::id()
}

#[non_exhaustive]
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolFilterOverrideMode {
    /// Inherit strategy.allowed_tools verbatim (no-op).
    Inherit,
    /// Intersect strategy.allowed_tools with this list.
    Intersect { tools: Vec<String> },
    /// Replace strategy.allowed_tools entirely (used by Plugin / Subagent).
    Replace { tools: Vec<String> },
}
```

### 7.4 `ContextDecisionRecorded`

```rust
EventPayload::ContextDecisionRecorded {
    turn_id: TurnId,
    compactions: Vec<EventId>,               // 本轮 ContextCompacted 事件 id(0 或 1 v0.1)
    system_prompt_event: EventId,            // 本轮 SystemPromptInjected 的 id(必有)
    tool_filter_event: EventId,              // 本轮 ToolFilterOverridden 的 id(必有)
    errors: ContextDecisionErrors,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct ContextDecisionErrors {
    pub compactor: Option<String>,           // 序列化 ContextError(降级时)
    pub injector: Option<String>,
    pub overrider: Option<String>,
}
```

### 7.5 Schema 工件与 JSONL spec

- `docs/schemas/conversation-event-v1.json`:CI drift gate 自动重新生成,加 4 个新 variant
- `docs/data-model/jsonl-v1.md`:加 §"Context management events" 章节,描述 4 个 variant 的字段、example、与其他事件的时序关系
- 既有 fixture(`crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`)**不变**——它表示 pre-Sprint-6 session,reader 必须仍能解析
- 新增 fixture `sessions/sample-truncate-v1.jsonl`:含完整 ContextManaged 真实工作流的 100-turn session,用于反向兼容测试 + 文档示例

---

## 8 · cogito-context crate layout + build_pipeline 工厂

### 8.1 Crate 结构

```
crates/cogito-context/
  Cargo.toml                    # [dependencies] cogito-protocol = workspace
  src/
    lib.rs                      # pub use {pipeline::ContextPipeline, build_pipeline};
    pipeline.rs                 # ContextPipeline + build_pipeline 工厂
    compactor/
      mod.rs                    # pub mod none; pub mod truncate;
      none.rs                   # NoneCompactor(无 I/O,不写事件)
      truncate.rs               # TruncateCompactor + §10 算法
    projector/
      mod.rs                    # pub mod standard;
      standard.rs               # StandardProjector(§5 算法)
    injector/
      mod.rs                    # pub mod none;
      none.rs                   # NoneInjector(写空 suffix 事件)
    overrider/
      mod.rs                    # pub mod none;
      none.rs                   # NoneOverrider(写 Inherit 事件)
  tests/
    pipeline_assembly.rs        # build_pipeline 各 config 装配正确性
    truncate_compaction.rs      # §10 全部 7 个边界
    standard_projection.rs      # §5 多压缩场景
    none_impls.rs               # 4 个 no-op 实现的事件写入正确性
```

### 8.2 `build_pipeline` 工厂

```rust
// cogito-context::pipeline
pub fn build_pipeline(config: &ContextConfig) -> ContextPipeline {
    ContextPipeline {
        compactor: build_compactor(&config.compactor),
        projector: build_projector(&config.history_projector),
        injector: build_injector(&config.system_prompt_injector),
        overrider: build_overrider(&config.tool_filter_overrider),
    }
}

fn build_compactor(cfg: &CompactorConfig) -> Arc<dyn Compactor> {
    match cfg {
        CompactorConfig::None => Arc::new(compactor::none::NoneCompactor),
        CompactorConfig::Truncate(c) => Arc::new(compactor::truncate::TruncateCompactor::new(c.clone())),
    }
}

// build_projector / build_injector / build_overrider 同样的 match 模式
```

per CLAUDE.md §"Tagged-config factories":工厂住在拥有 impl 的 crate(`cogito-context`),Surface crate(`cogito-cli` / `cogito-tui` / consumer's Server)永远只调一次 `build_pipeline`,不需要知道有哪些 variant。

### 8.3 Runtime 装配点

`crates/cogito-core/src/runtime/mod.rs`:

```rust
// 在 Runtime::open_session 内,strategy 已知后:
let context_pipeline = Arc::new(cogito_context::build_pipeline(&strategy.context));
// 注入 SessionShared:
session_shared.context_pipeline = context_pipeline.clone();
```

`SessionShared` 已有 `model_gateway: Arc<dyn ModelGateway>` / `tool_provider: Arc<dyn ToolProvider>` 等;`context_pipeline` 是同模式的第三类注入。

---

## 9 · H11 orchestration + 失败降级

### 9.1 orchestration 顺序(v0.1 固定)

`crates/cogito-core/src/harness/turn_driver/transitions/context_managed.rs` 重写:

```rust
async fn run_context_managed(ctx: TurnCtx, shared: &SessionShared) -> Result<...> {
    // ── 已有逻辑:ContextManageEntered 已在 Init→ContextManaged 入口写
    let pipeline = &shared.context_pipeline;
    let mut errors = ContextDecisionErrors::default();

    // 1. Compactor:可能写 0 或 1 个 ContextCompacted
    let compactions = match pipeline.compactor.maybe_compact(CompactionInput {
        session_id: ctx.session_id,
        turn_id: ctx.turn_id,
        history: &ctx.history_snapshot,
        strategy: &ctx.strategy,
        last_usage: ctx.last_usage.clone(),
        model_gateway: shared.model_gateway.as_ref(),
        recorder: &mut shared.recorder.lock().await,
    }).await {
        Ok(v) => v.into_iter().map(|c| c.event_id).collect(),
        Err(e) => {
            errors.compactor = Some(e.to_string());
            tracing::warn!("compactor degraded: {e}");
            vec![]
        }
    };

    // 2. SystemPromptInjector:必写 1 个 SystemPromptInjected
    let system_prompt_event = match pipeline.injector.inject(InjectionInput { ... }).await {
        Ok(eid) => eid,
        Err(e) => {
            errors.injector = Some(e.to_string());
            tracing::warn!("injector degraded: {e}");
            // 降级:H11 自己写一个 empty SystemPromptInjected 替代
            shared.recorder.lock().await.record_system_prompt_injected(
                ctx.turn_id,
                String::new(),
                vec![],
                "fallback-empty",
            ).await?
        }
    };

    // 3. ToolFilterOverrider:必写 1 个 ToolFilterOverridden
    let tool_filter_event = match pipeline.overrider.override_filter(ToolFilterInput { ... }).await {
        Ok(eid) => eid,
        Err(e) => {
            errors.overrider = Some(e.to_string());
            tracing::warn!("overrider degraded: {e}");
            shared.recorder.lock().await.record_tool_filter_overridden(
                ctx.turn_id,
                ToolFilterOverrideMode::Inherit,
                vec![],
                "fallback-inherit",
            ).await?
        }
    };

    // 4. H11 自己写 ContextDecisionRecorded
    shared.recorder.lock().await.record_context_decision(
        ctx.turn_id,
        compactions,
        system_prompt_event,
        tool_filter_event,
        errors,
    ).await?;

    // 5. ContextManageCompleted + 转移
    shared.recorder.lock().await.record_context_manage_completed(ctx.turn_id).await?;
    Ok(TurnState::PromptBuilt { ctx })
}
```

### 9.2 失败降级矩阵

| Trait 失败 | H11 行为 | 影响 |
|---|---|---|
| Compactor `Err(...)` | 记 `errors.compactor = Some(msg)`,本轮不压缩 | 本轮 prompt 可能超 context window;Provider 报 length error,正常 ToolFailure 路径处理 |
| Injector `Err(...)` | 记 `errors.injector`;H11 fallback 写 `SystemPromptInjected{empty}` | 本轮 system suffix 为空,可能丢失 Skill 注入等 |
| Overrider `Err(...)` | 记 `errors.overrider`;H11 fallback 写 `ToolFilterOverridden{Inherit}` | 本轮使用 strategy 默认 tool surface,无收窄 |
| 任一 fallback 写入也失败 | propagate `Err` 上抛 → H01 进入 `TurnFailed` | 本轮失败;H03 resume 可恢复 |

**核心原则**:**Context 失败不应阻断 turn**——它是优化,不是必经路径。fallback 写入失败才视为不可恢复(因为已经触底)。

---

## 10 · TruncateCompactor 算法详尽规格

### 10.1 配置

```rust
// cogito-protocol::strategy::CompactorConfig::Truncate 携带:
pub struct TruncateConfig {
    pub max_tokens: u64,            // default: 100_000
    pub keep_first_user: bool,      // default: true
    pub keep_recent_turns: u32,     // default: 5
}
```

### 10.2 算法(完整伪码)

```rust
async fn maybe_compact(&self, input: CompactionInput<'_>) -> Result<Vec<CompactionApplied>, ContextError> {
    // ── 步骤 1:幂等检查(resume 关键)
    if let Some(existing) = find_compaction_for_turn(input.history, input.turn_id) {
        return Ok(vec![CompactionApplied {
            event_id: existing.event_id,
            replaced_seq_range: existing.range,
            kind: CompactionKind::Truncate,
        }]);
    }

    // ── 步骤 2:token 估算
    let estimated = input.last_usage
        .as_ref()
        .and_then(|u| u.prompt_tokens)
        .unwrap_or_else(|| estimate_visible_tokens(input.history));
    if estimated < self.config.max_tokens {
        return Ok(vec![]);
    }

    // ── 步骤 3:扫描 turn 边界 + 已覆盖范围
    let turn_boundaries = collect_turn_boundaries(input.history);
    let covered = collect_covered_ranges(input.history);

    // ── 步骤 4:确定保留索引边界
    let total = turn_boundaries.len();
    let first_keep_idx = if self.config.keep_first_user { 1 } else { 0 };
    let last_keep_idx = total.saturating_sub(self.config.keep_recent_turns as usize);
    if first_keep_idx >= last_keep_idx {
        return Ok(vec![]);
    }

    // ── 步骤 5:在可丢区间内定位首尾 uncovered turn
    let mut drop_start_seq = None;
    let mut drop_end_seq = None;
    for idx in first_keep_idx..last_keep_idx {
        let (_, start, end) = turn_boundaries[idx];
        let fully_covered = (start..=end).all(|s| covered.contains(s));
        if !fully_covered {
            if drop_start_seq.is_none() {
                drop_start_seq = Some(start);
            }
            drop_end_seq = Some(end);
        }
    }
    let (Some(start), Some(end)) = (drop_start_seq, drop_end_seq) else {
        return Ok(vec![]);
    };

    // ── 步骤 6:写 ContextCompacted(StepRecorder 校验 §5.5 invariants)
    let event_id = input.recorder.record_context_compacted(
        input.turn_id,
        (start, end),
        "truncate",
        CompactionReplacement::Drop,
        TokenEstimates {
            before: Some(estimated),
            after: Some(estimated.saturating_sub(estimated_dropped_chars(input.history, start, end) / 4)),
        },
    ).await?;

    Ok(vec![CompactionApplied {
        event_id,
        replaced_seq_range: (start, end),
        kind: CompactionKind::Truncate,
    }])
}
```

### 10.3 `estimate_visible_tokens` helper

```rust
fn estimate_visible_tokens(events: &[ConversationEvent]) -> u64 {
    let covered = collect_covered_ranges(events);
    let mut chars = 0u64;
    for ev in events {
        if covered.contains(ev.seq) { continue; }
        chars += match &ev.payload {
            EventPayload::TurnStarted { user_input, .. } => user_input.len() as u64,
            EventPayload::AssistantMessageAppended { text, .. } => text.len() as u64,
            EventPayload::ThinkingBlockRecorded { text, .. } => text.len() as u64,
            EventPayload::ToolUseRecorded { args, .. } => args.to_string().len() as u64,
            EventPayload::ToolResultRecorded { result, .. } => result.to_string().len() as u64,
            EventPayload::ContextCompacted { replacement: CompactionReplacement::Summary { text, .. }, .. } => text.len() as u64,
            _ => 0,
        };
    }
    chars / 4
}
```

**精度声明**:char/4 是粗估,与真实 prompt_tokens 偏差可达 ±20%。truncate 阈值默认设 100k(而非 120k)以吸收偏差。后续 Compactor 需要精确时**自己**用 provider tokenizer——cogito 不在 v0.1 提供统一 tokenizer 抽象(避免拖入 tiktoken / cl100k 等编码表大依赖)。

### 10.4 边界场景清单(测试必须覆盖)

| # | 场景 | 期望 |
|---|---|---|
| 1 | 全新 session,字符估算 < max_tokens | no-op,返回 `vec![]` |
| 2 | 估算 ≥ max_tokens,但 `total_turns ≤ keep_recent + keep_first_user` | no-op |
| 3 | 估算超阈,有可压区间,无先前压缩 | 写 1 个事件,range `[turn[first_keep].start, turn[last_keep-1].end]` |
| 4 | 估算超阈,先前压缩部分覆盖可压区间 | 写 1 个事件,range 起点跳过头部已覆盖 turn,终点扩到最后一个 uncovered turn 末(中段已覆盖部分由 set-union 自然吸收) |
| 5 | 估算超阈,可压区间全被先前压缩覆盖 | no-op |
| 6 | 本 `turn_id` 已有 ContextCompacted(resume) | 不调任何 I/O,返回已有事件的 `CompactionApplied` |
| 7 | range 计算后违反 §5.5(防御性,理论不会触发) | StepRecorder 返回 `InvariantViolated`,Compactor 透传 `Err`,H11 降级 |

---

## 11 · H04 / H05 改动点

### 11.1 H04 Prompt Composer

**改动**:
- 抽出 history projection 逻辑到 `HistoryProjector` trait 调用——`prompt::compose` 改为通过 `Arc<dyn HistoryProjector>`(从 SessionShared 拿)调用 `project()`
- 读取本轮 `SystemPromptInjected.suffix`,拼到 `strategy.system_prompt` 后(`\n\n` 分隔)
- 其余 ModelInput 装配逻辑不变

**保持**:H04 仍是纯函数(读 events 不算 I/O)。

**兼容性**:旧 session(pre-Sprint-6,无 SystemPromptInjected / ContextCompacted)走 §5 算法 fallback——`covered` 为空,`system_suffix` 为 None,行为与 pre-Sprint-6 完全一致。

### 11.2 H05 Tool Surface Builder

**改动**:
- 在 PromptBuilt 阶段,从 events 取本轮 `ToolFilterOverridden.mode`
- 按 mode 处理:
  - `Inherit`:不动 `strategy.allowed_tools`
  - `Intersect { tools }`:与 `strategy.allowed_tools` 求交集
  - `Replace { tools }`:完全替换
- 与已有 `strategy.tool_order` 处理无冲突(顺序仍来自 strategy)

**保持**:H05 仍是纯函数。

**兼容性**:旧 session 无 `ToolFilterOverridden` 事件 → fallback 同 `Inherit`,行为与 pre-Sprint-6 一致。

### 11.3 文档更新

- `docs/components/H04-prompt-composer.md`:补 §"HistoryProjector dispatch" 章节,§"System prompt injection" 章节
- `docs/components/H05-tool-surface.md`:补 §"ToolFilterOverridden integration" 章节
- `docs/components/H11-context-manage.md`:从 placeholder 升级——补 §"v0.1 implementation" 章节,引用本 spec / ADR-0008

---

## 12 · Resume / idempotency

### 12.1 不引入新 `ResumePoint`

H03 既有 9 行 decision table(`docs/components/H03-resume-coordinator.md` §"Decision table")**无需扩展**。理由:Context Manage 的所有 I/O(Compactor 的 ModelGateway 调用)都通过 Compactor 自身的幂等性保护——已有 `ContextCompacted` for `current_turn` 时 Compactor 直接短路。

H03 在遇到 `ContextManageEntered` 无 `ContextManageCompleted` 配对时,选择 `ResumeFromInit`(已有变体)——重跑 ContextManaged 转移,Compactor 因为幂等返回 0 个新事件,Injector / Overrider 因为 turn_id 重复检测(下面 §12.2)也短路或重写。

### 12.2 三个 trait 的幂等保证

| Trait | 幂等机制 |
|---|---|
| Compactor | 写入前检查 `history_find_compaction_for_turn(turn_id)`;命中则不调模型,直接返回 |
| Injector | 写入前检查 `history_find_system_prompt_injection_for_turn(turn_id)`;命中则返回已有 EventId,**不重写**(避免本轮多条 SystemPromptInjected 事件) |
| Overrider | 同 Injector,检查 `history_find_tool_filter_override_for_turn(turn_id)` |

**StepRecorder 实现**:`record_system_prompt_injected` / `record_tool_filter_overridden` 内部先查 in-memory snapshot;若本 turn_id 已有,返回已有 EventId 而非新写。Compactor 因为有 §5.5 校验路径,等价行为(已存在则会触发 "duplicate compaction for turn" InvariantViolated,Compactor 用 try-find 短路避开)。

### 12.3 Mid-compaction 崩溃决策表

| 崩溃点 | 已持久化状态 | Resume 行为 |
|---|---|---|
| 进入 Compactor 之前 / 中(model call 进行中) | 无 ContextCompacted | H11 重跑 → Compactor 重新调用模型(代价:重做一次 summarization,但语义正确) |
| Compactor 调用完模型,未写事件 | 无 ContextCompacted | 同上 |
| Compactor 写事件完成 / 未返回 | ContextCompacted 已 flush | H11 重跑 → Compactor 检测到已有,直接返回(无 I/O) |
| Injector 完成,Overrider 未跑 | ContextCompacted + SystemPromptInjected | H11 重跑 → Compactor 短路,Injector 短路,Overrider 重跑 |
| ContextDecisionRecorded 已写,Completed 未写 | 全套 + Decision | H11 重跑 → 全部短路;只重写 Completed |
| ContextManageCompleted 已写 | 全套 | H11 不再进入(已是 PromptBuilt 起点) |

### 12.4 既有 chaos scenarios 的反向验证

Sprint 3 既有 chaos scenarios(`single_tool_happy_path` / `no_tool_short_turn` 等)在 Sprint 6 后走的是**真实** ContextManaged 转移(no-op pipeline)。这给出反向自检:

- 若现有 chaos 仍 pass,说明 no-op pipeline 在 H03 resume 下不破坏既有不变量
- `resume_chaos.rs` 主驱动新增断言 helper `assert_context_managed_pairing(events)`:**任何 `ContextManageEntered` 必被 `ContextManageCompleted` 配对**,或被 turn 终态事件(`TurnFailed`)覆盖。~30 LoC 新增。

不新增 chaos scenarios(per ROADMAP Sprint 6 line:"Chaos test: skipped if v0.1 reference Compactor is truncate-only")。truncate 同步执行,无 mid-compaction 模型调用窗口。

---

## 13 · 测试策略

### 13.1 单测

`crates/cogito-context/`(集成测试在 tests/,单测在 src/ 中 `#[cfg(test)]` mod):

| 测试 | 文件 | 覆盖 |
|---|---|---|
| TruncateCompactor 全路径 | `tests/truncate_compaction.rs` | §10.4 全部 7 个边界 + idempotency |
| StandardProjector | `tests/standard_projection.rs` | §5 算法,含 §5.3 多压缩混合 Drop/Summary 场景的字符级断言 |
| 4 个 no-op impls | `src/{compactor,injector,overrider}/none.rs` 内 `#[cfg(test)]` | NoneCompactor 不写事件;Injector 写 empty;Overrider 写 Inherit |
| build_pipeline | `tests/pipeline_assembly.rs` | 各 ContextConfig variant 装配正确 Arc<dyn ...> |

### 13.2 集成测试

`crates/cogito-core/tests/`:

| 测试 | 关注点 |
|---|---|
| `context_managed_no_op.rs` | strategy = default ContextConfig → ContextManaged 写 4 事件(Entered + SystemInjected + ToolOverridden + Decision + Completed);无 Compacted |
| `context_managed_with_truncate.rs` | 100+ turn session + Truncate{max_tokens: 小阈值} → ContextManaged 写 5 事件含 ContextCompacted;next turn PromptComposed.surface_size 反映压缩 |
| `h04_multi_compaction_projection.rs` | §5.3 trace 直接 replay,断言 ModelInput.messages 序列字符级匹配 |
| `h05_tool_filter_intersect.rs` | strategy `allowed_tools: Allow([a,b,c])` + 注入 ToolFilterOverridden::Intersect([b]) → H05 输出仅含 b |
| `h05_tool_filter_replace.rs` | strategy `allowed_tools: All` + Replace([x]) → 仅 x |

### 13.3 Resume 测试

复用 Sprint 3 已有 chaos 框架,增量:
- `resume_chaos.rs` 主驱动加 `assert_context_managed_pairing` 断言(§12.4)
- `single_tool_happy_path` / `no_tool_short_turn` 现在走真实 ContextManaged → 反向自检(不新增 scenario)

不为 truncate 加专门 chaos scenario——v0.2 加 summarize Compactor 时一并加 mid-summarization crash 场景。

### 13.4 性能基线(可选)

`make bench` 已有 criterion 框架。Sprint 6 可加 micro-bench `context_managed_no_op_latency`,目标:default ContextConfig 下 ContextManaged 转移 P99 < 1ms。

非强制——若 Sprint 6 时间紧,跳过;v0.2 引入 summarize Compactor 时再加(届时性能基线更有意义)。

---

## 14 · 未来扩展点

### 14.1 Sprint 7(Skill loader)

- 引入 `SkillsInjector`(`cogito-skills` crate)实现 `SystemPromptInjector`:把激活的 Skill 内容拼接到 suffix,contributors 中列出 `skill:<name>`
- `SystemPromptInjectorConfig` 加 `Skills { ... }` variant(non_exhaustive,additive)
- 不需要修改 ADR-0008——trait surface 不变

### 14.2 v0.2 Sprint 12(Plugin local)

- Plugin 提供 `<plugin_id>:` 前缀的 SystemPromptInjector / Compactor / Overrider 实现
- 通过 `HookProvider` / `CompactorProvider` 等 provider-aggregation 模式接入(类似 Sprint 5 H09 已有 `HookProvider` 模式)
- ContextConfig 内可声明 `compactor: { kind: "plugin", id: "acme:adaptive-trunc" }`(后续 amendment 加 `Plugin` variant 到 `CompactorConfig`)

### 14.3 v0.2 Summary Compactor

- 新增 `compactor::summarize` 模块,实现 `Compactor`
- `CompactorConfig` 加 `Summary { summarization_model: String, target_tokens: u64, prompt_template: ... }` variant
- 需要解决:summarization model 与 turn model 不同时的并发调用、prompt template 设计、cascading max_depth(若决定支持级联摘要)
- 新增 chaos scenario:mid-summarization crash(§12 决策表已含)

### 14.4 v0.5 多模态 Summary

- `CompactionReplacement` 加 `MultimediaSummary { blocks: Vec<ContentBlock> }` variant
- 配合 `ContentBlock::Image` / `Storage` 抽象使用
- non_exhaustive 已就位,additive

### 14.5 v0.4 Postgres backend partitioning

- 利用 `EventPayload::category()` 把 `ContextDecision` / `HarnessMeta` 类事件路由到独立表
- `replay()` SQL UNION ALL ORDER BY seq;Brain 无感知

---

## 15 · Risks / open questions

### 15.1 R1:估算精度可能误触发

`estimate_visible_tokens` 用 char/4,可能与真实 prompt_tokens 偏差 ±20%。极端情况下:

- truncate `max_tokens = 100_000` 估算到 110_000(实际 90_000)→ 不必要的压缩
- 反之,估算到 95_000(实际 105_000)→ 该压缩没压,模型报 length error

**缓解**:
- 默认 max_tokens 设保守(100k,而非 contextWindow 80%)
- 优先使用 `last_usage.prompt_tokens`(真实值);只在它缺失时才退到 char 估算
- 文档明示估算精度,允许 strategy 调参

非阻塞——v0.2 加 summarize 时同样问题,届时考虑统一抽象(可能在 ModelGateway 加 `count_tokens` 方法)。

### 15.2 R2:Skill 注入 + 长 suffix 影响压缩判断

Sprint 7 SkillsInjector 注入的 SKILL.md 内容可能 1-3KB——这部分**不在** `last_usage.prompt_tokens`(prompt_tokens 是上一轮的,本轮 system suffix 还没发出去)。

**缓解**:Compactor 决策时不感知本轮 suffix——这是设计有意为之(per §9.1 顺序,Compactor 先跑、Injector 后跑)。后果:本轮估算偏低 1-3KB(Skill 启用时)。属可接受偏差,在 R1 范围内。

### 15.3 R3:并发 turn 与 context_pipeline 共享

`ContextPipeline.compactor: Arc<dyn Compactor>` 在并发 turn 时被多 task 共享。Compactor impl 必须 `Send + Sync` 且无 mutable state(`TruncateCompactor` 仅持有不可变 `TruncateConfig`,OK)。

**缓解**:trait bound 强制 `Send + Sync`(已在 trait 定义中);ADR-0008 明示 Compactor impl 不能持有 per-session mutable state(若需,通过 ExecCtx / SessionShared 访问)。

### 15.4 O1:open—`disable_model_invocation` 与 Compactor 失败级联

若 v0.2 Skill 的 frontmatter 含 `disable-model-invocation: true`,某个 Compactor 又需要调模型(summarize),冲突时谁优先?

**当前回应**:留到 Sprint 7 ADR-0020 处理——`disable-model-invocation` 是 Skill 内部 flag,不影响 Compactor。Compactor 决定调模型时,使用的是 `summarization_model`(可与 strategy.model 不同);若那个 model 被禁,Compactor 自己 fallback(目前 v0.1 无此问题,v0.2 加 summarize 时一并设计)。

### 15.5 O2:open—`PromptComposed` 是否记录 ContextDecision 引用

H11 已写 `ContextDecisionRecorded` 总结条,但 H04 写的 `PromptComposed` 是否需要再引用一次 `ContextDecisionRecorded.event_id`?

**当前回应**:暂不引用(避免事件之间环状引用)。审计工具按 `turn_id` 关联即可——同 turn 的所有事件天然属于一组。后续若需要,additive 加字段不破 schema。

### 15.6 O3:open—`SystemPromptInjected.contributors` 字段稳定性

`contributors: Vec<String>` 的元素格式("date" / "skill:plan-review" / "tenant:acme")在 Sprint 6 仅作为约定,**不形式化**。Sprint 7 / 12 可能引入 ContributorTag enum 替换 String。

**当前回应**:用 String 留弹性;若 Sprint 7 需要类型化,additive 在事件 payload 平行加 `contributor_tags: Vec<ContributorTag>` 字段(non_exhaustive),不破老数据。

---

## 16 · 实施依赖图

按依赖与并行可能性建议执行顺序(留 writing-plans skill 拆细):

```
[ADR-0008 起草]
        │
        ▼
[cogito-protocol 新增]──────┐
        │                   │
        ▼                   ▼
[cogito-context: build_   [StepRecorder 校验
 pipeline + no-op + truncate]   + 4 个 record_* 方法]
        │                   │
        └────────┬──────────┘
                 ▼
        [H11 transitions::context_managed.rs 重写]
                 │
        ┌────────┼────────┐
        ▼        ▼        ▼
   [H04 改]  [H05 改]  [Runtime 装配]
        │        │        │
        └────────┼────────┘
                 ▼
        [集成 + 单测 + 文档]
                 │
                 ▼
        [chaos pairing assert + 反向自检]
```

ADR-0008 与 cogito-protocol 新增高度并行(ADR 描述什么,protocol 落地什么);其余按图依赖。

---

## 17 · 后续动作清单

本 spec ratify 后立即:

- [ ] 用 writing-plans skill 把本 spec §16 实施依赖图拆为带 owner / 预估时长 / 接口验收的 task list
- [ ] 起草 ADR-0008 正文(本 spec 的 §3-§9 + §12 浓缩为规范级文档)
- [ ] 更新 `docs/components/H11-context-manage.md`:从 placeholder 升级为正式组件文档,引用 ADR-0008 + 本 spec
- [ ] 更新 `docs/components/H04-prompt-composer.md` / `H05-tool-surface.md` 注脚说明新读取/调用点
- [ ] 更新 `docs/data-model/jsonl-v1.md`:加 §"Context management events" 章节
- [ ] 待 §13 集成测试通过后,标记 Sprint 6 完成,更新 ROADMAP.md / CHANGELOG.md

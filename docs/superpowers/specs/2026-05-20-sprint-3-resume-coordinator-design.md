# Sprint 3 · Resume Coordinator — 设计 Spec

> **Status**: Accepted (2026-05-20)
> **Sprint**: v0.1 · Sprint 3 (per [ROADMAP.md](../../../ROADMAP.md))
> **Authors**: qiannengsheng + AI brainstorm partner

本文件是 Sprint 3 的**决策讨论轨迹**。可执行契约住在 durable 文档里
（`ARCHITECTURE.md` / `docs/components/H0X-*.md` / `docs/adr/`），每节末尾给出链接。
此 spec 解释 **why**；durable 文档定义 **what**。

---

## 1 · Sprint 目标

让 cogito 第一次真正**可复活**：单 session 在任意状态崩溃后，新进程读事件日志 →
H03 算出 resume point → actor 在正确状态继续推进 turn，直到 `Completed` /
`Paused` / `Failed` 三个终态之一。

这是 v0.1 quality gate 的核心证据 —— ADR-0005 §3 已把 chaos test 列为 v0.1
版本的可发布门槛。**Sprint 2 留下的"事件序列完整可被未来 H03 消费"承诺，
Sprint 3 必须兑现。**

### 1.1 In-scope

1. `EventPayload` 新增 `ModelCallCompleted { stop_reason, usage }` 变体，由 H06
   在流闭合时落库；JSON schema artifact 重生成、JSONL 人类可读 spec 更新、
   canonical fixture 增补一行。
2. `harness::resume::replay()` 实现完整决策表，输出
   `ResumeDecision { point: ResumePoint, last_event_seq: Option<u64> }`。
   `ResumePoint` 6 个变体：`FreshTurn` / `RestartCurrentTurn` /
   `ResumeFromModelCompleted` / `ResumeFromToolDispatching` /
   `ResumePausedJob` / `ResumeAfterJobCompletion`。
3. `runtime/actor.rs:226` 的 `replay(&[])` 接通：actor 启动序列变为
   "读 store → 算 decision → 初始化 seq → 按 ResumePoint 分流"。
4. `Runtime::open_session(SessionMode::Resume)` 走通：找不到 session 时返
   `RuntimeError::ResumeFailed`；找到时立即触发恢复。
5. Per-session `seq` 计数器从 store 末尾事件推得（`EventId` 是每事件现生
   UUID，无需起点）；Sprint 2 留下的 `recorded_event_id: "unknown"` stub
   修掉，由 `record_*` 方法的返回值串回。
6. `tests/resume_chaos.rs` 新建：Z 混合崩溃注入机制（Y 全量 + X 深度）+
   D 四条等价 oracle + ≥3 个代表性场景。
7. `cogito-test-fixtures` 加 `FaultInjectingStore` wrapper + `MockJobManager`
   （`ResumePausedJob` 场景用）。
8. `consecutive_tool_errors` 在 resume 后**显式置 0** —— spec 写明这是有意
   over-tolerant（safety net 而非 correctness boundary）。
9. `ARCHITECTURE.md` 加 §"Actor model — why and how"；ADR-0006 §1 Decision
   段扩；H03 doc 整段重写；本 spec 落地。

### 1.2 Out-of-scope（明确不做、防止 scope creep）

| 不做的事 | 何时做 |
|---|---|
| `ToolDispatchStarted` / `JobSubmitted` 事件变体 | Sprint 4（跟真 async tool 一起） |
| `partial_text: Option<String>` 字段 | 永不（ADR-0007 锁定 TextDelta 不落库） |
| H11 `ContextManage` 真实业务（compaction 等） | ADR-0008 ratified 后 |
| `cogito-jobs` 完整 `JobManager`（含 job 状态跨进程持久化） | Sprint 4 |
| Tool registry 兼容降级（tool 删除 / schema 变化时软处理） | v0.6 hardening（如有需求） |
| `TurnPaused` payload 加 `call_id` 字段 | Sprint 4（multi-async-dispatch 出现时） |
| Subagent resume | Sprint 5 / v0.3 |
| Storage layer URI resolvability across crash | v0.2 |
| "tail-only 增量读"优化 | v0.6（10k+ event session 出现时） |
| 多 session 跨进程协调复活 | v0.4 SaaS-ready |

每一行的"为什么不在 Sprint 3"都已经在前置 brainstorm 中讨论过，落在对应
durable doc 或下一个 sprint 的 ROADMAP 节点。

---

## 2 · 决策轨迹（Q1–Q4）

### Q1 · `EventPayload` 事件 vocabulary 怎么对齐

**讨论的备选**：

- **a · 决策表向当前事件收敛**：只用已有 13 个变体，决策表合并成更少起始
  状态。代价：崩在 model streaming 中途 vs 崩在 model 刚开始的两种情况，
  事件日志末尾都是 `ModelCallStarted` —— 唯一安全的决定是**重发 model
  请求**（重新计费 token）。所有非 `TurnCompleted` / `TurnFailed` /
  `TurnPaused` 收尾的日志都默认回退到"重发 model"，决策粒度粗。
- **b · 补齐两个事件**：在 `EventPayload` 加 `ModelCallCompleted` 和
  `ToolDispatchStarted`。代价：JSON schema artifact 重生成、fixture 同步、
  recorder 加两个方法、两个 transition 改动。`#[non_exhaustive]` 保证 0.x
  b-档兼容（ADR-0007），不需要 bump SCHEMA_VERSION。
- **c · 混合**：只加 `ModelCallCompleted`，不加 `ToolDispatchStarted`。

**决策 c**。理由：

1. **`ModelCallCompleted` 是信息密度最高的事件**：它把"是否要再花一次
   token 重跑 model"这个有钱可烧、用户可见的决策从 H03 内部推断变成事件
   直查。这是 resume 语义最贵的辨别点。
2. **`ToolDispatchStarted` 在 Sprint 3 没真对比项**：Sprint 3 只有 sync +
   幂等的 `read_file`，"已派发 vs 未派发"在重跑语义下无差别。要 Sprint 4
   引入真异步工具时才有用。
3. **延迟到 Sprint 4 加是有代价的**：那时 chaos test 要补新崩溃点、
   `step_recorder` 要补新方法、H03 决策表要重写一次。**但这笔代价是显式
   的技术债**（本 spec §9 + ROADMAP Sprint 4 记录），不是隐患。
4. **从 Codex / Claude Code 的工具目录看 sync 工具长期占比 > 70%**，
   `ToolDispatchStarted` 的真实收益等到 v0.3+ 才显现。

**新增变体定义**：

```rust
// crates/cogito-protocol/src/event.rs
#[non_exhaustive]
pub enum EventPayload {
    // ... 现有 13 个变体不变 ...

    /// Recorded by H06 Stream Demultiplexer when the model response stream
    /// emits `MessageCompleted` (Anthropic `message_delta` with stop_reason /
    /// OpenAI `finish_reason`). Sealing event for one model call.
    ModelCallCompleted {
        stop_reason: StopReason,
        usage: Usage,
    },
}
```

**落库时机**：`transitions/model_calling.rs::transit` 中 H06 流到
`ModelEvent::MessageCompleted` 时，先调
`step_recorder::record_model_call_completed(turn, stop_reason, usage)`，
再返回 `TurnState::ModelCompleted { output, surface }`。

**Schema 影响**：

- `SCHEMA_VERSION` **不动**（仍是 1）。`#[non_exhaustive]` 加变体属 b-档兼容。
- `docs/schemas/conversation-event-v1.json` 重生成（`cogito-gen-schema` bin
  执行），CI drift gate 触发一次，作为预期 commit。
- `docs/data-model/jsonl-v1.md` 加一段说明：what / when / payload shape。
- `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
  增加一行，放在 `model_call_started` 之后、首个 `assistant_message_appended`
  之前的合理位置。
- `event.rs` 的 `all_nine_variants_roundtrip` 测试改名 + 加一例。

→ 具体契约：[ARCHITECTURE.md §"Content blocks"](../../../ARCHITECTURE.md) ·
   [docs/components/H02-step-recorder.md](../../components/H02-step-recorder.md) ·
   [docs/components/H06-stream-demux.md](../../components/H06-stream-demux.md)

### Q2 · `ResumeDecision` 的 shape

**讨论的备选**：

- **A · Variant enum**（当前 Sprint 2 stub 的形态扩展版）：
  `ResumeDecision::FreshTurn | RestartCurrentTurn | ... | PausedOnJob`。
  类型安全，payload-per-variant。
- **B · 扁平 record**（H03 doc 原写法）：
  `ResumeDecision { state: TurnState, last_event_seq, partial_text }`。
  问题：`TurnState::ModelCalling` 内嵌 `BoxStream`（不可序列化、不可从事件
  重建），`state` 必须是 `TurnState` 的子集 —— 类型系统帮不上忙。
  `partial_text` 在 v0.1 永远是 `None`（ADR-0007 锁定 TextDelta 不落库），
  是永远填不上的死字段。**不推荐**。
- **C · 混合**：variant enum 表 resume 点的语义 + 公共元数据外层。

**决策 C**。理由：

1. **`last_event_seq` 是真有用的公共元数据**：actor 启动后 `seq` 生成器必
   须 strictly > 日志末尾值，提到外层抽一次比埋在每个变体里干净。
2. **`partial_text` 不进去**：违反 YAGNI + b-档兼容窗口内移除是破坏性变更。
3. **`TurnState` 不适合作为 resume 接口**：`BoxStream` 证明"运行时状态"和
   "可恢复状态"是两套类型，硬合并污染 FSM 定义。
4. **跟现存 `enter_turn` 兼容**：当前已经在 enum match，从 A 风格升 C 风
   格只是套一层 `.point` 解包。

**Q2 后续修正（讨论中发现）**：

Q2 第一稿锁了 5 个 `ResumePoint` 变体，**实际有 6 个**。漏掉的是
`ResumeFromModelCompleted`：model 已经完整返回 `end_turn`、没有任何 tool
call，但 actor 崩在写 `TurnCompleted` 之前。这一档如果归到
`RestartCurrentTurn`，意味着 resume 时白白再调一次 model —— 恰恰是 Q1 加
`ModelCallCompleted` 想避免的核心 case。

**最终类型定义**：

```rust
// crates/cogito-core/src/harness/resume.rs

#[derive(Debug, Clone)]
pub struct ResumeDecision {
    pub point: ResumePoint,
    /// `seq` of the last event in the log when this decision was computed.
    /// `None` iff `point == FreshTurn` AND the log is empty.
    /// Actor uses this to initialize the per-session event seq generator.
    pub last_event_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum ResumePoint {
    /// 空日志或上一 turn 已终结（TurnCompleted/Failed）。
    /// Actor idle；下一条 Input 触发 spawn TurnDriver。
    FreshTurn,

    /// In-flight turn 但 model call 未完成。FSM 进 Init，H04 从日志重建
    /// prompt；一次 model call 会被重新计费。
    RestartCurrentTurn { turn_id: TurnId },

    /// 最近 model call 已 `ModelCallCompleted` 且 stop_reason = end_turn
    /// 无 tool calls，actor 崩在写 TurnCompleted 之前。FSM 进 ModelCompleted
    /// 状态用 rebuilt_output，直接 fast-path 终结 —— 不重发 model。
    ResumeFromModelCompleted {
        turn_id: TurnId,
        rebuilt_output: ModelOutput,
    },

    /// Tool dispatch 轮次部分进行中。可能 0 或多个工具已完成。
    /// FSM 进 ToolDispatching；surface 由 enter_turn 调 H10+H05 重建。
    ResumeFromToolDispatching {
        turn_id: TurnId,
        /// `latest_mcc` 之后未配对的 `ToolUseRecorded` 条目；保持日志顺序。
        /// H07 在 dispatch 前重验 schema。
        pending: Vec<ResumePendingCall>,
        /// 已配对 `(call_id, ToolResult)`。
        completed: Vec<(String, ToolResult)>,
    },

    /// Turn 在 async job 上暂停。`TurnPaused` 是最近事件，无后续
    /// `JobCompletedRecorded`。Actor 进 PausedOnJob 重注册 on_complete。
    ResumePausedJob { turn_id: TurnId, job_id: JobId },

    /// Async job 已完成但 Brain 在消费 `JobCompletedRecorded` 之前崩溃。
    /// FSM 进 ToolDispatching 注入结果。`call_id` 由 walk-back 解出
    /// （Sprint 3 invariant：≤1 async dispatch per turn；Sprint 4 可能改）。
    ResumeAfterJobCompletion {
        turn_id: TurnId,
        job_id: JobId,
        outcome: JobOutcome,
        call_id: String,
        completed_before_pause: Vec<(String, ToolResult)>,
        pending_after_pause: Vec<ResumePendingCall>,
    },
}

/// 从 `ToolUseRecorded` 事件恢复出的 raw 三元组。enter_turn 通过 H07
/// 重验后才进 dispatch。
#[derive(Debug, Clone)]
pub struct ResumePendingCall {
    pub call_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    #[error("malformed event log: {0}")]
    Malformed(String),
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    #[error("tool `{tool_name}` (call_id `{call_id}`) no longer registered")]
    ToolUnavailable { call_id: String, tool_name: String },
    #[error("tool `{tool_name}` schema rejects persisted args: {reason}")]
    ToolSchemaDrift { tool_name: String, reason: String },
}
```

**关键设计决定**（durable doc 要写明）：

1. **`ResumeFromModelCompleted` 内嵌 `rebuilt_output: ModelOutput`**：由 H03
   从事件流派生，避免 enter_turn 二次扫描。H03 唯一会做"事件 → 高层值"重
   构的地方。
2. **不带 `surface_snapshot`**。所有需要 surface 的 ResumePoint，由
   `enter_turn` 在恢复进 FSM 前调 H10+H05 重建。理由：`ToolDescriptor` 内含
   `Arc<dyn ToolHandler>` 类型句柄，跨进程无意义。
3. **`pending` 用 `ResumePendingCall`（裸三元组）而非 `ToolInvocation`
   （resolved）**：enter_turn 必须过 H07 重验 schema，保证恢复路径和 fresh
   dispatch 走完全相同代码。schema 漂移 → `ResumeError::ToolSchemaDrift`，
   fail-loud。
4. **`consecutive_tool_errors` 不从日志重建**：actor 启动时显式置 0。
   有意 over-tolerant（恢复后允许多 4 次错误尝试），但这是 safety net 而非
   correctness boundary。

→ 具体契约：[docs/components/H03-resume-coordinator.md](../../components/H03-resume-coordinator.md)

### Q3 · Chaos test 的等价判定 oracle

**讨论的备选**：

- **A · 严格事件序列等价**：屏蔽 `event_id` / `ts` 后两份 log 字节相同。
  最强证据但完全依赖 mock 确定性，未来扩展（hooks 加随机抽样等）易碎。
- **B · 工具调用集合等价 + 终态等价**：比对 `{call_id → (args, result)}`
  + `TurnOutcome`。对 model nondeterminism 友好，但 assistant text 差异看
  不见。
- **C · 前缀严格 + 终态等价**：崩溃前已落库事件不变（验 ADR-0002 不可变
  性）+ 终态 outcome 一致。中段事件序列差异不查。
- **D · 混合**：C 的两条 + B 的工具映射 + mock 确定性提供的一条 strict
  text 比对。**4 条独立断言**。

**决策 D**。理由：

1. **每条断言对应一个清晰失败模式**：挂哪条直接定位（前缀 → 写入路径 bug；
   终态 → FSM 分流错；映射 → H07/H08 复活错；文本 → H06/H04 重建错）。
2. **前缀不可变这条直接验证 ADR-0002 承诺** —— 这是 cogito 比一般 agent
   runtime 强的关键点，值得独立断言。
3. **工具映射等价独立于 mock 确定性** —— 未来 v0.6 hardening 想拿真
   provider 跑 chaos 时，这条仍然成立。
4. **四条彼此独立、互不替代**：A 失败 = 不知道是 mock / EventId / H03 哪
   里坏；D 失败立即定位语义层。

**四条 oracle 形态**：

```rust
// crates/cogito-core/tests/resume_chaos.rs

fn canonical(e: &ConversationEvent) -> Canonical {
    // 屏蔽 event_id / ts，保留其他字段
}

// ① 前缀不可变
fn assert_prefix_immutable(golden: &[Event], resumed: &[Event], crash_n: u64);

// ② 终态等价（TurnCompleted / TurnFailed / TurnPaused 同变体；
//   Failed 比 TurnFailureReason 变体）
fn assert_terminal_equivalent(g_term: &EventPayload, r_term: &EventPayload);

// ③ 工具调用映射等价（{call_id → (tool_name, args, ToolResult)} 集合相等）
fn assert_tool_mapping_equivalent(golden: &[Event], resumed: &[Event]);

// ④ 最终 assistant 内容字节相等（mock 确定性保证）
fn assert_final_text_equivalent(golden: &[Event], resumed: &[Event]);
```

→ 具体契约：[docs/components/H03-resume-coordinator.md §"Testing strategy"](../../components/H03-resume-coordinator.md)

### Q3.5 · Chaos test 的 model 后端

**背景**：用户提议提供一个真实 LLM endpoint 作为 chaos test 的 ModelGateway。

**问题**：D 方案的四条 oracle 里，③（工具映射）和 ④（assistant 文本）**强
依赖 model 确定性**。真 LLM 哪怕 `temperature=0` 也只是近似贪婪 —— 跨负载
/ 跨 provider 小版本 / 浮点抖动都可能让 ③④ 挂。chaos test 跑 N 个崩溃点，
挂了之后无法分辨"是 H03 bug"还是"model 这次回答不同" —— chaos 失去
quality gate 资格。

**决策**：分工。

| 测试类型 | 后端 | 用途 |
|---|---|---|
| `tests/resume_chaos.rs`（CI gate） | `ScriptedMockModel`（确定性） | D 四条 oracle，每事件边界注入崩溃 |
| `tests/resume_smoke_real_llm.rs`（stretch / nightly） | 真 LLM endpoint | 1-2 个崩溃点，断 oracle ① + ②；不断 ③④ |
| 手动 e2e（dev loop） | 真 LLM endpoint | `just chat` + 中途强杀 / cancel_turn，验证 paused-job 复活 |

行业里 Codex / Claude Code / Temporal 都用脚本化 mock 跑 chaos —— 不是没有
真 LLM，而是 chaos 的本质是"控制所有变量、只允许崩溃点变化"。

### Q4 · 崩溃注入机制

**讨论的备选**：

- **X · 真 `panic!()`** 注入到 transition 内部，触发 ADR-0006 panic catch；
  新 `Runtime` 实例模拟新进程。真实度最高，**捎带验证 panic catch quality
  gate**；代价：panic 噪声、每点慢、需 set_hook 静音。
- **Y · 干净停机**：写到第 N 个事件后由外部调 `SessionHandle::shutdown`，
  actor 走正常 drain → 新 Runtime 复活。确定、安静、快；**但不验 panic
  catch**，跟 production 真崩溃路径不完全一致。
- **Z · 混合**：Y 当主力（一个场景的每个事件边界都跑），X 补深度（5-10 个
  panic 语义关键点）。

**决策 Z**。理由：

1. **覆盖面 vs 深度兼得**：Y 提供"每边界都测"广度，X 补 panic 路径深度。
2. **顺手验证 ADR-0006 panic catch**：ADR-0005 §3 已把 panic catch 列为
   quality gate，Sprint 3 跟 resume 一起证了，省下单独写 panic catch 测试
   的工作。
3. **实现成本可控**：fault injection 机制做出来后，X 和 Y 共用同一个
   `FaultInjectingStore` wrapper，差别只在触发动作。

**Fault knob 形态（spec 写死）**：

测试侧 wrapper，**零修改 production 代码**：

```rust
// crates/testing/cogito-test-fixtures/src/fault_store.rs
pub struct FaultInjectingStore<S> {
    inner: S,
    written_count: AtomicU64,
    trigger: Mutex<FaultTrigger>,
}

pub enum FaultTrigger {
    None,
    /// 写到第 N 个事件后 panic（X 路径）。事件已落库再 panic。
    PanicAt { event_no: u64, message: &'static str },
    /// 写到第 N 个事件后通过 oneshot 通知测试（Y 路径）。
    NotifyAt { event_no: u64, signal: oneshot::Sender<()> },
}
```

`cogito-protocol::ConversationStore` 已经是 trait → 直接套 wrapper。
production 代码完全不感知 fault injection 的存在，无需 cfg flag / feature
gate。AGENTS.md "不为假设的未来加字段"约束不违反 —— wrapper 只活在
`cogito-test-fixtures` 里。

→ 具体契约：本 spec §8（Chaos test 设计）

---

## 3 · Resume 完整时序

新进程 / 重启后单 session 端到端恢复时序：

```
┌─ Caller (CLI / consumer) ──────────────────────────────────────────────┐
│   runtime.open_session(id, SessionMode::Resume).await                  │
└──────────────────┬─────────────────────────────────────────────────────┘
                   │
                   ▼
┌─ Runtime::open_session ───────────────────────────────────────────────┐
│  ① 检查 in-memory 注册表（防止并发开同一 session）                      │
│  ② store.range(session_id, ..).await   ← 拉全部事件                    │
│  ③ events.is_empty()? → Err(ResumeFailed: no such session)            │
│  ④ 派 SessionActor::spawn(initial_events = events)                    │
└──────────────────┬─────────────────────────────────────────────────────┘
                   │
                   ▼
┌─ SessionActor::actor_main ────────────────────────────────────────────┐
│  ⑤ schema 检查（fail-fast）                                            │
│  ⑥ let decision = harness::resume::replay(&initial_events)?          │
│  ⑦ state.event_seq.store(decision.last_event_seq + 1)                │
│  ⑧ 若 New session 写 SessionStarted                                   │
│  ⑨ apply_resume_point(decision.point):                                │
│     - FreshTurn               → in_flight=Idle                       │
│     - RestartCurrentTurn      → spawn TurnDriver (Init-like)          │
│     - ResumeFromModelCompleted→ spawn TurnDriver (ModelCompleted)     │
│     - ResumeFromToolDispatching→ spawn TurnDriver (ToolDispatching)   │
│     - ResumePausedJob         → in_flight=PausedOnJob;                │
│                                 job_manager.on_complete(sink)         │
│     - ResumeAfterJobCompletion→ inject result → spawn TurnDriver      │
│                                 (ToolDispatching)                     │
│  ⑩ 进 mailbox 主循环                                                  │
└────────────────────────────────────────────────────────────────────────┘
```

**关键不变量**（durable doc 必写）：

- **② 全部在 Runtime 上层完成**，actor 启动后不再回 store 拉历史。保证
  actor_main "events → state"是确定性纯函数（可单测）。
- **⑦ 必须早于 ⑨**：seq 生成器未初始化前任何写入都可能产生
  seq < last_event_seq 的事件，破坏 ADR-0002 不可变性。
- **`ResumePausedJob` 分支不 spawn TurnDriver**。Turn 已主动暂停等的是
  外部 job 而非模型；错误 spawn 会让 TurnDriver 立刻终止，actor 多绕一圈
  又要注册 on_complete —— bug 温床。
- **`ResumeAfterJobCompletion` 是独立分支**（非 ResumeFromToolDispatching
  的子情况）：前者从 `JobCompletedRecorded` 派生 completed 的最后一项，
  后者从 `ToolResultRecorded` 派生 —— 两者数据来源不同。

→ 具体契约：[ARCHITECTURE.md §"Turn state machine"](../../../ARCHITECTURE.md) 末尾增
   "Resume entry path" 子节；[docs/components/H03-resume-coordinator.md](../../components/H03-resume-coordinator.md)
   §"Interface"；[docs/adr/0006-runtime-h01-execution-model.md](../../adr/0006-runtime-h01-execution-model.md)
   §1 amendment。

---

## 4 · H03 决策表 v1.1

输入：完整事件日志切片。输出：`ResumeDecision`。算法 O(N)，单次线性扫描。

### 4.1 阶段一 · 找最近的 turn 边界

从日志末尾倒着找第一个属于
`{TurnStarted, TurnCompleted, TurnFailed, TurnPaused}` 之一的事件。

| 边界事件 | 后续 | ResumePoint |
|---|---|---|
| 不存在（只有 SessionStarted 或空日志） | — | `FreshTurn` |
| `TurnCompleted` / `TurnFailed` | — | `FreshTurn` |
| `TurnPaused { job_id }` | 找不到匹配 `JobCompletedRecorded { job_id }` | `ResumePausedJob` |
| `TurnPaused { job_id }` | 后面有匹配 `JobCompletedRecorded { job_id, outcome }` | `ResumeAfterJobCompletion`（call_id 见 §4.3） |
| `TurnStarted { turn_id }` | — | 进阶段二分类 |

### 4.2 阶段二 · 当前 turn 内部分类

从 `TurnStarted` 之后的事件中找：

- `latest_mcs` = 最近的 `ModelCallStarted` 索引
- `latest_mcc` = 最近的 `ModelCallCompleted` 索引

| `latest_mcs` | `latest_mcc` | 关系 | ResumePoint |
|---|---|---|---|
| `None` | — | — | `RestartCurrentTurn` |
| `Some(s)` | `None` | model 在飞 | `RestartCurrentTurn` |
| `Some(s)` | `Some(c)` | `s > c` | `RestartCurrentTurn`（新一轮 model call 在飞） |
| `Some(s)` | `Some(c)` | `c ≥ s` | 看 `c` 之后的 tool 事件 → 下表 |

在 `latest_mcc` 之后的事件中，把 `ToolUseRecorded` 配 `ToolResultRecorded`：

| `ToolUseRecorded` 数 | 已配对 | 未配对 | ResumePoint |
|---|---|---|---|
| 0 | — | — | `ResumeFromModelCompleted`（rebuild output；stop_reason 必为 end_turn） |
| ≥1 | k | u | `ResumeFromToolDispatching { pending: u, completed: k }` |

### 4.3 阶段三 · 构造 `ResumeDecision`

- `last_event_seq = events.last().map(|e| e.seq)`。
- **`ResumeFromModelCompleted.rebuilt_output`**：扫描 `latest_mcs` 与
  `latest_mcc` 之间的事件，把 `AssistantMessageAppended → ContentBlock::Text`、
  `ToolUseRecorded → ContentBlock::ToolUse` 按 seq 顺序拼成
  `Vec<ContentBlock>`，叠上 `latest_mcc` 的 `stop_reason` + `usage`。
- **`ResumeAfterJobCompletion.call_id`**：在 `TurnPaused` 之前找最近一个
  unmatched `ToolUseRecorded` 的 call_id。Sprint 3 invariant：每 turn ≤1
  async dispatch；Sprint 4 重新评估（可能改为在 `TurnPaused` payload 直接
  存 call_id）。

### 4.4 边界与错误

- `JobCompletedRecorded` 但找不到匹配 `TurnPaused` → `ResumeError::Malformed`。
- 多个 turn 嵌套（`TurnStarted` 后又 `TurnStarted` 没 `TurnCompleted/Failed`）
  → `ResumeError::Malformed`（v0.1 单 session 单 turn-in-flight 不变量）。
- `schema_version > SCHEMA_VERSION` → `ResumeError::UnsupportedSchema`。
- `schema_version < SCHEMA_VERSION` → 当前 SCHEMA_VERSION 仍是 1，无问题；
  Sprint 7 后再考虑。

### 4.5 单元测试矩阵

`crates/cogito-core/src/harness/resume.rs` 的 `#[cfg(test)] mod tests`：

- 决策表每一行一个测试（约 9 行）。
- 每个 `ResumePoint` 变体的边界 case（empty completed / empty pending /
  multi-round model-tool loop）。
- `ResumeError` 三种触发路径各一个测试。
- proptest：随机事件序列 + 不变量"决策对应的 FSM 状态接受的下一个 transition
  不会与日志已记录的事件冲突"。

→ 具体契约：[docs/components/H03-resume-coordinator.md §"Resume decision table"](../../components/H03-resume-coordinator.md)
   整段重写。

---

## 5 · Actor recovery 改动详解

### 5.1 `Runtime::open_session` 入口分流

```rust
let initial_events: Vec<ConversationEvent> = match mode {
    SessionMode::New => {
        let existing = self.store.range(&id, ..).await?;
        if !existing.is_empty() {
            return Err(RuntimeError::SessionAlreadyExists { id });
        }
        vec![]
    }
    SessionMode::Resume => {
        let events = self.store.range(&id, ..).await?;
        if events.is_empty() {
            return Err(RuntimeError::ResumeFailed {
                id, reason: "no such session in store".into(),
            });
        }
        events
    }
    SessionMode::Attach => {
        self.store.range(&id, ..).await?  // 找到就用，找不到当 New
    }
};
let actor = SessionActor::spawn(self.shared.clone(), id, initial_events).await?;
```

新增 `RuntimeError` 变体：`SessionAlreadyExists`、`ResumeFailed`。

### 5.2 `actor_main` 启动序列

```rust
pub(super) async fn actor_main(
    state: ActorState,
    initial_events: Vec<ConversationEvent>,
) -> ShutdownOutcome {
    // ① schema 检查
    if let Some(evt) = initial_events.iter()
        .find(|e| e.schema_version > SCHEMA_VERSION)
    {
        return ShutdownOutcome::ResumeFailed(
            format!("unsupported schema_version={}", evt.schema_version));
    }

    // ② H03 算决策
    let decision = match harness::resume::replay(&initial_events) {
        Ok(d) => d,
        Err(e) => return ShutdownOutcome::ResumeFailed(e.to_string()),
    };

    // ③ seq 生成器初始化（必须先于任何写入）
    state.event_seq.store(
        decision.last_event_seq.map_or(0, |s| s + 1),
        Ordering::SeqCst,
    );

    // ④ New session 写 SessionStarted
    if initial_events.is_empty() {
        state.recorder.lock().await.record_session_started(/* meta */).await?;
    }

    // ⑤ 按 ResumePoint 分流
    apply_resume_point(&mut state, decision.point).await?;

    // ⑥ mailbox 主循环（Sprint 2 已有部分）
    actor_loop(state).await
}
```

**步骤顺序不能换**：① 早于 ② 因为 schema 不兼容则后续算法无意义；② 早于
③ 因为 last_event_seq 来自 decision；③ 早于 ⑤ 因为分流可能立即 spawn
TurnDriver 触发写入。

### 5.3 ResumePoint 分流（`apply_resume_point`）

```rust
async fn apply_resume_point(
    state: &mut ActorState,
    point: ResumePoint,
) -> Result<(), ShutdownReason> {
    match point {
        ResumePoint::FreshTurn => { /* idle */ }
        ResumePoint::RestartCurrentTurn { turn_id } => {
            spawn_turn_driver(state, turn_id, TurnEntry::FreshLikeInit).await?;
        }
        ResumePoint::ResumeFromModelCompleted { turn_id, rebuilt_output } => {
            spawn_turn_driver(
                state, turn_id,
                TurnEntry::FromModelCompleted { output: rebuilt_output },
            ).await?;
        }
        ResumePoint::ResumeFromToolDispatching { turn_id, pending, completed } => {
            spawn_turn_driver(
                state, turn_id,
                TurnEntry::FromToolDispatching { pending, completed },
            ).await?;
        }
        ResumePoint::ResumePausedJob { turn_id, job_id } => {
            state.in_flight = InFlight::PausedOnJob { job_id: job_id.clone(), turn_id };
            state.job_manager
                .on_complete(job_id, state.job_completion_tx.clone())
                .await
                .map_err(|e| ShutdownReason::JobManagerUnavailable(e.to_string()))?;
        }
        ResumePoint::ResumeAfterJobCompletion {
            turn_id, job_id: _, outcome, call_id,
            completed_before_pause, pending_after_pause,
        } => {
            let mut completed = completed_before_pause;
            completed.push((call_id, ToolResult::from_job_outcome(outcome)));
            spawn_turn_driver(
                state, turn_id,
                TurnEntry::FromToolDispatching { pending: pending_after_pause, completed },
            ).await?;
        }
    }
    Ok(())
}
```

`TurnEntry` 是 `cogito-core::harness::turn_driver` 内部的中间 enum，把
`ResumePoint` 转译成 `enter_turn` 真正吃的形态。让 `ResumePoint` 不必依赖
`TurnEntry`，enter_turn API 也不被 `ResumePoint` 形态绑死。

### 5.4 EventId 在 `TurnFailed` 中的回填

Sprint 2 留的 stub（`state.rs:135`）：
```rust
recorded_event_id: "unknown".into(),
```

Sprint 3 解决：

1. `step_recorder` 的所有 `record_*` 方法签名统一为 `Result<EventId, _>`。
2. `transitions/*.rs` 中拿到 EventId 后塞进相关 `TurnOutcome` 字段。
3. 特别是 `record_turn_failed` 的 EventId 回填进
   `TurnOutcome::Failed { recorded_event_id }`，让 caller 能精准定位失败事件。

### 5.5 错误对外形态

| 错误来源 | 形态 |
|---|---|
| Session 不存在（Resume mode） | `open_session` 同步 `Err(RuntimeError::ResumeFailed)` |
| Schema 不兼容、Malformed log | actor 启动失败，`ShutdownOutcome::ResumeFailed(reason)` |
| Tool unavailable / schema drift | enter_turn 转为 `TurnOutcome::Failed { reason: ResumeFailed }`，**写一条 TurnFailed 事件**到日志（下次 resume 会看到这个 TurnFailed 而走 FreshTurn 路径自然脱困） |
| JobManager `on_complete` 报 unknown job | `ShutdownOutcome::JobManagerUnavailable`；**不写日志事件**（runtime 配置失败而非 turn 失败） |

**分界线**：ResumeError 自身（事件流问题）不写日志；turn-level resume
错误（tool 找不到、schema drift）写日志。区分依据是错误是 cogito 自身
bug 还是 turn 内部的合法失败。

### 5.6 PausedOnJob 跨进程契约

Sprint 3 用 `MockJobManager`，测试代码控制 job 生命周期。**Sprint 4 上真
`cogito-jobs` 时**：新进程的 `JobManager` 实例必须能识别上一进程提交的
`job_id` 并履行 `on_complete` 契约 —— 这意味着 `JobManager` 需要持久化
job 状态（很可能是 JSONL job log mirror 事件日志结构）。

本 spec 注记一条："`cogito-jobs`（Sprint 4 ADR）必须实现跨进程 job 状态持
久化；Sprint 3 的 MockJobManager 通过测试 fixture 模拟这一行为"。

→ 具体契约：[docs/adr/0006-runtime-h01-execution-model.md](../../adr/0006-runtime-h01-execution-model.md)
   §1 amendment；[docs/components/H03-resume-coordinator.md](../../components/H03-resume-coordinator.md)
   §"Called by"；ROADMAP Sprint 4 deliverables 加 "job state persistence" 一行。

---

## 6 · Resume 落盘语义

**`ResumeDecision` 不落盘**。它是 H03 这个纯函数的输出 —— 给同样日志永远算
出同一值。落盘 = 派生状态和事实状态并列 = event sourcing 反模式。

| 概念 | 形态 | 生命周期 | 责任方 |
|---|---|---|---|
| `ConversationEvent` | 落盘（JSONL） | 永久 / 跨进程 | `ConversationStore` |
| `JobState` / `JobOutcome` | 落盘（job log，Sprint 4） | 永久 / 跨进程 | `JobManager` |
| **`ResumeDecision`** | **纯内存值** | actor_main 启动到 enter_turn 调用之间 | actor 本地 stack |
| `TurnState` (FSM 中) | 纯内存值 | 一个 turn 的生命周期 | `TurnDriver` 任务 stack |
| `InFlight` enum | 纯内存值 | 一个 actor 的生命周期 | `SessionActor` 字段 |
| Per-session `seq` 计数器起点 | 纯内存值（从日志末尾 seq 推得） | 一个 actor 的生命周期 | `SessionActor` 启动时计算一次 |

简而言之：**盘上只有"发生过什么"，内存里才有"现在在哪儿"**。后者每次进
程重启从前者重新派生。

**违反落盘的 4 条理由**（任一条都足以否决落盘）：

1. **违反 AGENTS.md inviolable rule #3**：state 在 Conversation Service，不
   在 Harness memory；ResumeDecision 在 ADR-0002 体系下属可重建派生值。
2. **违反 ADR-0002 单一真理源**：bug 路径多出"两份不一致信谁"歧义。
3. **跨版本会自爆**：Sprint 4 引 async tool 时决策算法会变；旧落盘的
   ResumeDecision 用新算法读要么忽略（白做）要么照搬（语义错）。
4. **它本来就不可序列化**：`ToolDescriptor.handler` 是 `Arc<dyn ...>`
   （trait object），跨进程无意义。

**性能注记**：Sprint 3 一次性全量读 store。H03 实际只需要从最近
`TurnStarted` 起的事件就够算决策 —— 但 Sprint 3 不优化（v0.6 hardening
单独 ADR）。v0.1 单 session 事件量级几十到几千，顺序流 I/O 可忽略。

→ 具体契约：[docs/components/H03-resume-coordinator.md](../../components/H03-resume-coordinator.md)
   §"Critical invariants" 加第 6 条 "ResumeDecision is a derived projection;
   never persisted"。

---

## 7 · Actor 模型主张（ARCHITECTURE.md 新节内容）

Sprint 3 的 actor recovery 路径要求 actor 模型本身是 first-class 概念 ——
不是 ADR-0006 的一个旁注，而是 cogito 整体架构的承重柱。本节内容将沉淀
到 `ARCHITECTURE.md` 新 §"Actor model — why and how"（紧随
§"Brain / Hands / Session boundaries"）。

### 7.1 Why an actor model

cogito 是嵌入式库（ADR-0005 §1），单进程要服务 ≥1000 个并发 session
（ADR-0005 §3 SLO）。这条约束直接排除 Codex 风格的
`Arc<Session> + Mutex<ActiveTurn>` 共享状态方案 —— mutex 一旦在某 session
内 poisoned，所有访问同一 mutex 的代码路径全瘫，违背"单 session 故障隔离"。

具体要满足：

- **故障隔离**：单 session panic 不波及其它 session
- **消费方拥有 tokio**：cogito 不 `Runtime::new()`，接受外部 `Handle`
- **协作式取消**：ctrl-C 终止当前 turn 而非 kill session
- **双路事件流**：持久化（durable, backpressure）和广播（low-latency,
  lossy）契约矛盾
- **异步 job 唤醒**：actor 在 PausedOnJob 时仍能响应 mailbox

这五条共同指向 actor 模型。

### 7.2 Actor 模型的四条核心不变量

cogito 的 actor 模型由四条不变量定义（不是工程偏好，是正确性前提）：

1. **私有状态**：每 session 的运行时状态由**一个**任务独占。无跨 actor 的
   `Arc<Mutex<_>>`。
2. **消息驱动**：所有跟 actor 的交互走 channel —— mailbox（命令）、
   broadcast（事件）、persist（落库）、job sink（异步唤醒）。函数调用直
   达 actor 内部 = 设计 bug。
3. **单一可变所有者**：actor 任务是其私有状态的唯一 mutator。subtask
   （TurnDriver、store_writer）通过 channel 拿值的副本或显式 handle。
4. **协作式终止**：取消走 `CancellationToken` + `select!`，永不
   `task.abort()`。每个 await 点都有机会 drop RAII guard、flush pending event。

### 7.3 cogito 的具体拓扑

```
                      Caller (CLI / consumer service)
                                  │
                                  ▼  Arc<Runtime>
                      ┌───────────────────────────┐
                      │         Runtime            │
                      │  · session_registry        │
                      │  · DI: store / model / ... │
                      │  · panic catch boundary    │
                      └────────────┬───────────────┘
                                   │ open_session
                                   ▼
   ┌───────────────── SessionActor (one task per session) ──────────────┐
   │                                                                     │
   │      mailbox (mpsc<SessionCommand>, cap 64)                         │
   │       Input / Shutdown / Cancel / JobCompleted                      │
   │              │                                                      │
   │              ▼ FIFO drain                                           │
   │       ┌───────────────┐                                             │
   │       │  actor_loop    │── private state (in_flight, seq, ...)      │
   │       └──┬──────┬──────┘                                            │
   │          │      │  events_out (broadcast<StreamEvent>, cap 256)     │
   │          │      └────────────────────────────────────►              │
   │          │             0..N live subscribers (lossy)                │
   │   spawn  │                                                          │
   │   per-   │      persist_tx (mpsc<PersistCommand>, cap 256)          │
   │   turn   │             │                                            │
   │          ▼             ▼                                            │
   │   TurnDriver task    store_writer subtask (serial fsync)            │
   │   (FSM run loop)              │                                     │
   │                               ▼                                     │
   │                       ConversationStore (JSONL / Postgres / ...)    │
   │                                                                     │
   │      job_completion_rx (mpsc<JobCompletionEvent>, cap 32)           │
   │       ◄── JobManager.on_complete(job_id, sink) callbacks            │
   └─────────────────────────────────────────────────────────────────────┘
```

### 7.4 优势（在 cogito 语境）

- **故障隔离落到 scheduler 层**：tokio 把 panicking task 单独 unwind；其它
  session 完全无感。这是 ADR-0005 §3 ≥1000 concurrent session SLO 的前提。
- **Backpressure 一等公民**：channel 容量（64/256/256）是显式 SLO 旋钮。
  慢消费者通过 `Lagged(n)` 自我感知，无静默 unbounded 增长。
- **取消可验证**：每个 await 在 `select!` 守护下，RAII guard 正常 drop ——
  跟 `task.abort()` 留下半状态形成对比。
- **Scaling 单元清晰**：单进程 = N 个 actor；多进程 = sticky session_id
  路由。cogito 不在进程内跨 actor 协调（消费方 deployment 问题），actor
  数量纵向加几乎无协调开销。
- **Resume 是局部的**：单 session 崩溃只需重建单 actor（Sprint 3 H03 +
  actor_main 流程）。共享状态方案要重建跨 session 锁状态 —— 复杂度等级
  不同。

### 7.5 代价（诚实记录）

- **每 session 基线内存**：tokio task stack + 3-4 channel + 私有状态 ≈
  10-30 KiB（未跑 turn 时）。Codex 测量近似。
- **Mailbox FIFO 矛盾**：`cancel_turn` 不能排队在 backlog 后面 → ADR-0006
  §3 用直接 token 信号绕过。
- **样板代码**：每 actor 管 4 类 channel + drain 协议；比 `Arc<Mutex>` 多
  约 30% LoC。
- **跨 actor 交互调试需结构化 tracing**：每 actor 一 span，否则日志被
  mailbox 顺序乱序困扰。

→ 具体契约：[ARCHITECTURE.md](../../../ARCHITECTURE.md) §"Actor model —
   why and how"（本节内容直接复制过去）；
   [docs/adr/0006-runtime-h01-execution-model.md](../../adr/0006-runtime-h01-execution-model.md)
   §1 Decision 体扩 + 交叉引用 ARCHITECTURE 新节。

---

## 8 · Chaos test 设计

### 8.1 文件布局

```
crates/cogito-core/tests/resume_chaos.rs           ← 主测试入口
crates/testing/cogito-test-fixtures/src/
  ├── fault_store.rs                                ← FaultInjectingStore
  ├── mock_job_manager.rs                           ← PausedOnJob 场景用
  └── chaos_scenarios.rs                            ← 脚本化场景目录
crates/testing/cogito-mock-model/src/lib.rs        ← 核实 / 补脚本化模式
```

**零修改 cogito-core 生产代码、零 cfg flag、零 feature gate**。所有 fault
注入靠 ConversationStore wrapper。

### 8.2 主测试结构

```rust
#[tokio::test]
async fn chaos_y_path_every_event_boundary() {
    for scenario in chaos_scenarios::all() {
        let golden = run_to_completion_without_faults(&scenario).await;
        for crash_after_n in 1..golden.events.len() as u64 {
            let resumed = run_with_y_fault(&scenario, crash_after_n).await;
            assert_prefix_immutable(&golden.events, &resumed.events, crash_after_n);
            assert_terminal_equivalent(&golden.terminal, &resumed.terminal);
            assert_tool_mapping_equivalent(&golden.events, &resumed.events);
            assert_final_text_equivalent(&golden.events, &resumed.events);
        }
    }
}

#[tokio::test]
async fn chaos_x_path_curated_panic_points() {
    let panic_points = [
        "after_turn_started", "after_prompt_composed",
        "after_model_call_started", "after_model_call_completed",
        "after_assistant_message_appended", "after_tool_use_recorded",
        "after_tool_result_recorded", "after_turn_paused",
        "after_job_completed_recorded",
    ];
    // 同 4 oracle + 额外验 ADR-0006 panic catch 工作
}
```

### 8.3 场景目录

| 场景 | 流程 | 事件数 | 覆盖 ResumePoint |
|---|---|---|---|
| `single_tool_happy_path` | user → model+tool_use → tool → model end_turn | ~12 | FreshTurn / RestartCurrentTurn / ResumeFromModelCompleted / ResumeFromToolDispatching |
| `no_tool_short_turn` | user → model end_turn | ~7 | FreshTurn / RestartCurrentTurn / ResumeFromModelCompleted |
| `tool_returns_error` | user → model+tool_use → ToolResult::Error → model handles | ~14 | + 工具错误路径 |
| `paused_async_job` | user → model+async_tool → TurnPaused → MockJob.complete → model end_turn | ~10 | ResumePausedJob / ResumeAfterJobCompletion |

每场景的 model script 写死在 `chaos_scenarios.rs`。

### 8.4 `MockJobManager` 契约

```rust
impl MockJobManager {
    /// Test API. 把 job 标记完成并触发已注册的 on_complete sink。
    pub async fn complete(&self, job_id: JobId, outcome: JobOutcome);
}

#[async_trait]
impl JobManager for MockJobManager {
    async fn on_complete(&self, job_id: JobId, sink: mpsc::Sender<JobCompletionEvent>) -> Result<()> {
        // 契约 1：job 已完成 → 立即触发 sink（resume 路径必需）
        // 契约 2：job 未完成 → 存 sink，complete() 调用时投递
    }
}
```

**契约 1 是 Sprint 3 ResumePausedJob 路径能跑通的前提** —— 否则 actor 重
启后注册一个永不触发的 callback，turn 永远 hang。Sprint 4 的 `cogito-jobs`
也必须满足此契约。

### 8.5 CI 时间预算

| 路径 | 单 run | 场景 × 崩溃点 | 总时长 |
|---|---|---|---|
| Y | ~50ms (tmpfs + mock model) | 4 × ~12 = ~48 | ~2.5s |
| X | ~150ms (panic + 新 Runtime) | 4 × 8 = 32 | ~5s |
| **合计** | | | **< 10s** |

满足放进 `just ci` gate 的要求。**chaos test 是 v0.1 quality gate 的默认
CI 一部分**，与 fmt / clippy / unit test 同级。`just chaos` recipe 留给
v0.6 hardening 的 fuzz / property tests。

### 8.6 `cogito-mock-model` 阻断性前置

Sprint 3 第一件落代码的事：

1. 读 `crates/testing/cogito-mock-model/src/lib.rs` 当前实现。
2. 验证：给定 `input.messages` 序列，model 调用必须返回**逐字节相同**的
   ModelEvent stream（含 block_index、call_id、token 切片）。
3. 不满足则补 `ScriptedMockModel { matchers: Vec<(InputMatcher, OutputScript)> }`。
   `InputMatcher` 用 messages 序列的简单结构匹配。
4. **关键**：同一 input 调多次必须返回完全相同的 stream。chaos test 每 run
   至少调 model 1-N 次（restart 路径会重调），mock 必须幂等。

不幂等则 oracle ③④ 全随机挂，chaos test 失去意义 —— 所以这是**阻断性前
置**。

→ 具体契约：本 spec §"Q3+Q4 chaos test 完整设计"；
   [docs/components/H03-resume-coordinator.md](../../components/H03-resume-coordinator.md)
   §"Testing strategy" 替换；
   [docs/adr/0005-production-scope-and-quality-gates.md](../../adr/0005-production-scope-and-quality-gates.md)
   §3 chaos test 条目交叉引用本节。

---

## 9 · 风险与未决问题

**Sprint 3 启动不阻塞，但 visible 在 spec 里**：

1. **`cogito-mock-model` 脚本化能力未核实** — 阻断性，落代码 step #1 必查
   （§8.6）。
2. **X 路径下 tokio 测试的 panic 噪声处理** — `std::panic::set_hook` 临时
   静音 + `JoinHandle::await` 观察。具体实现时验证。
3. **`MockJobManager` 的 on_complete 契约（"job 已完成时立即触发"）** 在
   Sprint 4 `cogito-jobs` 文档里复刻 —— TODO 写进 ROADMAP Sprint 4。
4. **Resume 性能在 10k 事件以上未量化** — v0.1 不优化；v0.6 hardening 考
   虑增量读。
5. **真 LLM smoke vs mock chaos 的对比 benchmark dataset** — 没想清楚；
   Sprint 3 收尾时由测试结果驱动决定是否需要。
6. **`TurnPaused` 何时加 `call_id` payload** — Sprint 4 multi-async-dispatch
   场景出现时强制需要。本 sprint walk-back 算法在 v0.1 invariant 下正确。
7. **Tool registry 漂移的 fail-loud vs 软处理** — Sprint 3 选 fail-loud；
   消费方反馈出现"resume 后 tool 找不到是常见 case"时再加 ADR。

---

## 10 · 落地工件清单

> **方法论原则**：spec 是**决策轨迹**（why），durable doc 是**真理来源**
> （what + how）。锁定的决策内容**必须**沉到对应 durable doc，否则一年
> 后 spec 沉积会让真理碎片化（参见用户记忆 `feedback_doc_strategy`）。
> 本节列出 Sprint 3 全部需要更新的文档及其改动颗粒度。

### 10.1 文档沉淀矩阵（design-only PR）

实施顺序：本 spec → ARCHITECTURE / ADR → 各 component doc → ROADMAP。
每行一个 commit，共 **10 个 commit** 串成一个 design-only PR。

| # | 工件 | 改动性质 | 沉淀什么内容 |
|---|---|---|---|
| 1 | `docs/superpowers/specs/2026-05-20-sprint-3-resume-coordinator-design.md` | **新建**（本文件） | 完整决策轨迹 Q1–Q4 + 最终架构 |
| 2 | `ARCHITECTURE.md` | 加新 §"Actor model — why and how" + §"Turn state machine" 末尾增 "Resume entry path" 子节 | spec §7 全文复制 + §3 Resume 完整时序图 |
| 3 | `docs/adr/0006-runtime-h01-execution-model.md` §1 | Decision 体扩 + §"Amendments" 加 2026-05-20 条目 | actor 模型四不变量（私有状态 / 消息驱动 / 单一可变所有者 / 协作式终止）；交叉引用 ARCHITECTURE 新节 |
| 4 | `docs/components/H03-resume-coordinator.md` | **整段重写**：Interface / Resume decision table / Critical invariants / Testing strategy | spec §4 决策表 + spec §5 actor 路径调用点 + spec §6 落盘语义（作为 invariant #6）+ spec §8 chaos test |
| 5 | `docs/components/H02-step-recorder.md` | 事件清单加 `ModelCallCompleted`；`record_*` 方法签名章节统一为 `Result<EventId, _>`；触发时机表加一行 | spec §4 Q1 + spec §5.4 EventId 串回 |
| 6 | `docs/components/H06-stream-demux.md` | 末尾增"调 step_recorder 时机"小节 | spec §4 Q1 落库时机段（MessageCompleted → record_model_call_completed → 返回 ModelCompleted state） |
| 7 | `docs/components/H01-turn-driver.md` | 加 "Resume entry path" 小节，说明 `enter_turn` 接受 `TurnEntry` 的 3 个 ResumePoint 来源（actor 内部转译） | spec §5.3 dispatch + `TurnEntry` 内部 enum 三变体 |
| 8 | `docs/adr/0007-event-log-as-cross-language-contract.md` | 加 §"Additive variant precedent" 注记 | `ModelCallCompleted` 作为 `#[non_exhaustive]` 加变体的 b-档兼容先例；不 bump SCHEMA_VERSION 的依据 |
| 9 | `docs/data-model/jsonl-v1.md` | 加 `ModelCallCompleted` 段（what / when emitted / payload shape / 示例） | spec §4 EventPayload 改动；与新 fixture 一对一 |
| 10 | `ROADMAP.md` | Sprint 3 checklist 备注 `ModelCallCompleted` 新增；Sprint 4 deliverables 加 "job state persistence (cross-process)" 一行 | spec §1.1 #1 + spec §5.6 跨进程契约 |

**说明**：

- **方向性原则**：每个 durable doc 章节最后增加 "→ 参考 spec
  `2026-05-20-sprint-3-resume-coordinator-design.md` §N" 链接，让未来读
  者能回溯决策过程；但**doc 自身要 self-contained**，不允许"看 spec 才
  能读懂"的省略。
- **重写 vs 增补**：H03 doc 现状跟新决策表差异太大（事件 vocabulary 都
  对不齐），整段重写；H02 / H06 / H01 doc 是增补章节；ADR-0006 是扩
  Decision 体；ADR-0007 加注记；ARCHITECTURE 加新节。
- **顺序约束**：commit 2（ARCHITECTURE actor model 节）必须在 commit 3
  （ADR-0006 §1 扩）之前 —— ADR-0006 要交叉引用 ARCHITECTURE 新节锚点。
  commit 4-7 之间无顺序约束。
- **CI 验证**：每个 commit 后跑 `just fmt` + markdown link 检查（如有）。
  整 PR 不应触发 schema gate（protocol 改动在 §10.2 实现 PR 里）。

### 10.2 实现工件（后续 PR）

**Protocol 层**：

- `crates/cogito-protocol/src/event.rs` — 加 `ModelCallCompleted` 变体 + 测试
- `docs/schemas/conversation-event-v1.json` — 重生成（`cogito-gen-schema` 执行）
- `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl` — 补一行

> 注：`docs/data-model/jsonl-v1.md` 已在 §10.1 commit 9 写入说明段；实现
> PR 只需校对内容跟 protocol 改动一致。

**Brain（cogito-core）层**：

- `crates/cogito-core/src/harness/resume.rs` — 重写 `replay()` + 类型定义 + 单元测试
- `crates/cogito-core/src/harness/step_recorder.rs` — 加 `record_model_call_completed`；所有 `record_*` 统一返 `Result<EventId, _>`
- `crates/cogito-core/src/harness/turn_driver/transitions/model_calling.rs` — H06 闭合时调 recorder
- `crates/cogito-core/src/harness/turn_driver/state.rs` — `recorded_event_id` stub 清理
- `crates/cogito-core/src/harness/turn_driver/mod.rs` — `enter_turn` 接受 `TurnEntry`

**Runtime 层**：

- `crates/cogito-core/src/runtime/builder.rs` — `open_session` 按 SessionMode 分流读 store
- `crates/cogito-core/src/runtime/actor.rs` — `actor_main` 启动序列改造（§5.2）+ `apply_resume_point`
- `crates/cogito-core/src/runtime/types.rs` — `RuntimeError` 加 `SessionAlreadyExists` / `ResumeFailed`；`ShutdownOutcome` 加 `ResumeFailed` / `JobManagerUnavailable`

**测试层**：

- `crates/cogito-core/tests/resume_chaos.rs` — 新建（§8.2 主结构）
- `crates/testing/cogito-test-fixtures/src/fault_store.rs` — 新建
- `crates/testing/cogito-test-fixtures/src/mock_job_manager.rs` — 新建
- `crates/testing/cogito-test-fixtures/src/chaos_scenarios.rs` — 新建（§8.3 四场景）
- `crates/testing/cogito-mock-model/src/lib.rs` — 视核实结果增 `ScriptedMockModel`

**Stretch / nightly**：

- `crates/cogito-core/tests/resume_smoke_real_llm.rs` — 真 LLM endpoint smoke
  （oracle ① + ②；当真 LLM 配置到位后）

---

## 11 · 与既有 ADR / spec 的关系

本 spec 跟既有 durable doc 的承继关系（不重复 §10.1 的 doc 修改清单，只
列承继 / 约束关系）：

- **ADR-0002（事件源化）**：本 spec §6 "Resume 落盘语义" 直接落实 ADR-0002
  §"State lives in events" 的实操形态。
- **ADR-0003（FSM Turn Driver）**：本 spec §3 Resume 时序复用 ADR-0003 的
  FSM 状态枚举，未新增 FSM 状态。
- **ADR-0004（Brain / Hands / Session 边界）**：本 spec 所有跨层调用走
  trait，零违反。`MockJobManager` / `FaultInjectingStore` 只活在 testing
  crate（Hands 层），Brain 通过 protocol trait 拿到，不知道是 mock。
- **ADR-0005（生产范围与 quality gate）**：本 spec §8.5 把 chaos test 钉死
  为默认 CI gate，落实 ADR-0005 §3 chaos 测试要求 + §"Panic isolation"
  quality gate（X 路径捎带验证）。
- **ADR-0006（Runtime + H01 execution model）**：本 spec §5 Actor recovery
  是 ADR-0006 §1 actor 模型在 resume 路径的具体落地。§10.1 commit 3 把
  actor 模型四不变量扩进 ADR-0006 Decision 体。
- **ADR-0007（事件日志作为跨语言契约）**：本 spec §4 EventPayload 改动遵守
  `#[non_exhaustive]` b-档兼容；SCHEMA_VERSION 不动。§10.1 commit 8 把这
  次改动作为"加变体不 bump version"的先例留进 ADR-0007。
- **Sprint 2 spec（`2026-05-19-sprint-2-minimal-loop-design.md`）**：
  本 spec 兑现 Sprint 2 spec 留下的"事件序列完整、可被未来 H03 消费"
  承诺；并清理 Sprint 2 留下的 `recorded_event_id: "unknown"` stub。
- **未来 ADR-0008（H11 Context Manage）**：本 spec 未触及 `ContextManaged`
  状态（v0.1 pass-through 不变）；H11 真业务上线后 H03 决策表可能加
  `ContextCompacted` 事件相关行，由 ADR-0008 决定。

---

**spec 到此为止**。

接下来：
- 用户审阅本 spec 并反馈调整意见（如有）。
- 通过后进 writing-plans skill，按 §10.1 拆出 design-only PR 的 10 个
  commit 计划，按 §10.2 拆出实现 PR 的工件落地顺序。

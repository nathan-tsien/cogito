# Runtime + H01 Turn Driver Execution Model — v0.1 Design Spec

> **Status**: Draft, pending review
> **Date**: 2026-05-18
> **Scope**: cogito v0.1 Foundation
> **Authors**: design dialogue with Codex Rust + Claude Code reference + SaaS platform survey

---

## §1 · Overview & scope

### Purpose

把 v0.1 阶段 Brain (H01–H10) 赖以运行的执行平台 —— Runtime layer + H01 Turn Driver 的实现路径 —— 一次性钉死。线程模型、内存所有权、并发拓扑、生命周期，全部落到具体类型和不变量上。

### In scope

- **Runtime layer**：Session actor、tokio handle 注入、panic isolation、per-session budget
- **H01 Turn Driver**：FSM 驱动循环、状态转移与事件持久化的关系、cancellation 透传
- **三个跨界协议层修正**：
  - `StreamEvent`（新增 protocol 类型）
  - `SessionCommand` mailbox（runtime 内部类型）
  - `JobManager` 回调形状（protocol trait 微调，增 `on_complete` method）
- **新增协议类型**：`ExecutionClass` (ToolDescriptor)、`InvokeOutcome` 落地、`TurnOutcome` / `TurnFailureReason` 完整定义

### Out of scope（其它文档负责）

- H02 step recorder 内部实现细节（除写入路径外）—— 已在 `docs/components/H02-*.md`
- H03–H10 各组件的内部算法 —— 各自的 H0X 文档
- `ConversationStore` JSONL 文件格式细节
- 各 Sprint 的具体任务排期 —— `ROADMAP.md`
- v0.2+ 的 `StorageSystem`、subagent、multi-tenant、observability adapter

### Prerequisites

ADR-0001（workspace layout）、ADR-0002（event sourcing）、ADR-0003（FSM Turn Driver）、ADR-0004（Brain/Hands/Session boundaries）、ADR-0005（production scope + quality gates）。本 spec 不修改任何已 ratify 的 ADR，只补它们之间没说清的"如何在 tokio 上跑"那一层。

### Deliverable

本 spec ratify 后：
1. 走 `superpowers:writing-plans` skill 产出实施计划，对应 Sprint 0 收尾 + Sprint 1–6 各 sprint 的入场条件
2. 提炼 load-bearing 决策成正式 ADR-0006（`docs/adr/0006-runtime-h01-execution-model.md`），引用本 spec 为详细参考

---

## §2 · Architecture context

在 ADR-0004 layer 图里，本 spec 涉及的范围（**框内 = 本 spec 定义；框外 = 已有 ADR 定义**）：

```
                ┌─────────────────────────────────────────────┐
                │   Surface (cogito-cli, consumer service)    │
                └────────────────────┬────────────────────────┘
                                     │ holds
                  ┌──────────────────▼──────────────────────┐
                  │  RUNTIME LAYER ─ this spec §3, §4       │
                  │   Runtime                                │
                  │     ├─ RuntimeBuilder (handle injection) │
                  │     ├─ open_session() / shutdown_all()   │
                  │     └─ SessionHandle map                 │
                  │                                          │
                  │   SessionActor (per session, tokio task) │
                  │     ├─ mailbox: mpsc<SessionCommand>     │
                  │     ├─ events_out: broadcast<StreamEvent>│
                  │     ├─ store writer subtask              │
                  │     └─ in_flight turn handle             │
                  └────┬───────────────────┬─────────────────┘
                       │ hosts (DI)        │ drives (calls)
            ┌──────────▼─────────┐    ┌────▼──────────────────┐
            │ Brain (cogito-core │    │ BRAIN H01 FSM         │
            │     ::harness)     │◄───┤ ─ this spec §5        │
            │  H01–H10           │    │ (each transition      │
            │  see docs/components│    │  writes event before  │
            │  /H0X-*.md         │    │  moving)              │
            └──────────┬─────────┘    └───────────────────────┘
                       │ uses traits (only protocol)
   ┌───────────────────┼───────────────────────────────────┐
   │ Protocol (cogito-protocol)                            │
   │   ConversationStore  ModelGateway  ToolProvider       │
   │   JobManager(*)      HookHandler                      │
   │   ConversationEvent  ContentBlock                     │
   │   StreamEvent ◄ NEW (this spec §5, §7)                │
   │   ExecutionClass ◄ NEW (this spec §6)                 │
   └───────────────────┬───────────────────────────────────┘
                       │ implemented by
        ┌──────────────┼──────────────────────────────────┐
        │ Session (cogito-store)                    │
        │ Boundary (cogito-model)                         │
        │ Hands (cogito-tools, cogito-jobs, cogito-sandbox)│
        └─────────────────────────────────────────────────┘

(*) JobManager trait 微调（见 §6）：增加 callback 注册形状以支持反向唤醒
```

### vs Codex Rust

- Codex 的 `Session` 是 Brain + Runtime 混在一起，`Arc<Session>` 共享 + `Mutex<ActiveTurn>` 同步当前 turn (`core/src/codex.rs:440`)
- cogito 坚持 ADR-0004 分层：`Runtime::SessionActor` 是平台/任务层，`harness::TurnDriver` 是 Brain 状态机；前者持有后者，前者跑在 tokio 任务里，后者只是逻辑
- 有意分叉：cogito 要支持 ≥1000 并发 session + per-session budget + multi-tenant 准备，actor 隔离比共享锁可扩展

### vs Claude Code

- Claude Code 把"agentic harness"作为整个进程的单一概念（一份 conversation = 一份 agent loop）
- cogito 是多 session 并发，所以每个 session 一个 actor 等价于 "many Claude Code instances multiplexed in one process"

---

## §3 · Threading & task topology

### 任务拓扑（per process，空间视图）

```
                    consumer service (axum / tonic / CLI / ...)
                              │  holds Arc<Runtime>
                              ▼
   ┌─────────────────────────────────────────────────────────────┐
   │ Runtime                                                      │
   │   tokio::runtime::Handle  (injected at build time)           │
   │   sessions: DashMap<SessionId, SessionHandle>                │
   │   job_manager: Arc<dyn JobManager>                           │
   │   shutdown_token: CancellationToken  (reserved for v0.4)     │
   └────────────────────────┬────────────────────────────────────┘
                            │ open_session() spawns ▼
   ┌─────────────────────────────────────────────────────────────┐
   │  SessionActor task (1 per session)                           │
   │  ─ tokio task, owns SessionHandle's receivers                │
   │  ─ catch_unwind boundary HERE (panic stops only this task)   │
   │  ─ runs the per-session loop (§4) until shutdown             │
   └────────┬─────────────────┬───────────────────┬──────────────┘
            │ owns            │ spawns            │ uses
            ▼                 ▼                   ▼
   ┌─────────────┐  ┌──────────────────┐  ┌────────────────────┐
   │ store       │  │ TurnDriver task  │  │ JobManager (shared)│
   │ writer      │  │  (per turn)      │  │ ─ separate tokio   │
   │ subtask     │  │ ─ runs H01 FSM   │  │   tasks per job    │
   │ ─ owns      │  │ ─ ends when turn │  │ ─ on complete:     │
   │   ConvStore │  │   reaches term   │  │   send JobCompleted│
   │   handle    │  │   state          │  │   to actor mailbox │
   │ ─ tokio task│  │ ─ JoinHandle     │  └────────────────────┘
   │ ─ recv from │  │   held by actor  │
   │   mpsc      │  └──────────────────┘
   └─────────────┘
```

**任务计数（稳态 1 active session）= 3 个 tokio task + N 个 job task：**

1. SessionActor 主任务（mailbox loop）
2. Store writer 子任务（每事件 → 写 store）
3. TurnDriver 子任务（每 turn 创建一次，结束销毁）
4. JobManager 内部任务（每 in-flight job 一个，共享给所有 session）

### 时间线视图（同一 session 多任务的生灭关系）

```
时间轴 →

caller thread
    │  open_session()           handle.send(Input)           handle.shutdown()
    │   ↓                          ↓                            ↓
    │   ●━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━●   持有 SessionHandle
    │
SessionActor task （长生命周期：与 session 同寿）
    │   ●━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━●
    │   ▲ replay → ready              ▲ in_flight=Active     ▲ cleanup
    │                                  │                       │
Store writer subtask （长生命周期：与 actor 同寿）
    │     ●━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━●
    │
TurnDriver task #1 （**短生命周期：1 个 turn 1 个 task，结束即销毁**）
    │                              ●━━━━━━━●
    │                              ▲ spawn ▲ join → TurnOutcome::Paused
    │
JobManager job task #J （**独立生命周期：跟 TurnDriver 完全解耦**）
    │                                ●━━━━━━━━━━━━━━━━━━━━━━●
    │                                ▲ submit               ▲ complete →
    │                                                         send JobCompleted
    │                                                         to actor mailbox
TurnDriver task #2 （**新 task：因 JobCompleted 重新 spawn，承接 #1 中断的位置**）
    │                                                          ●━━━━●
    │                                                          ▲ spawn  ▲ join → Completed
```

**关键不变量：**

1. **TurnDriver 不是常驻 task**。每个 `Input` 触发 spawn 一个 TurnDriver task；它跑完 turn（无论结果是 Completed / Paused / Failed / Cancelled）就结束销毁
2. **一个 turn 在生命周期内可能被多个 TurnDriver task 接力跑**。被 async job 截断后，Paused 的 turn 由后面**新 spawn 的另一个 TurnDriver task** 续上，承接的是中断处的 FSM state（从 ToolDispatching 重新开始）
3. **Job 任务和 TurnDriver 任务从不同居一个 task**。TurnDriver 把 tool 派给 ToolProvider，ToolProvider 返回 `InvokeOutcome::Async(JobId)` 就回 actor → TurnDriver 当前生命终结。Job 的实际执行在 JobManager 的内部 task 里跑，谁都不 await 它
4. **store writer subtask 是 session 范围内的常驻**。每次 turn 产生的事件不直接调 `store.append()`，而是塞 `persist_tx`，由 writer 子任务串行落盘

### 为什么 TurnDriver 单独 spawn 而不在 actor task 里直接 await

| 选项 | 结果 |
|---|---|
| 直接 await | actor 在 turn 进行时 mailbox 不读 → `CancelTurn`/`Shutdown` 进不来 → 违反 actor 不变量 |
| **spawn 子任务，actor select 在 (mailbox.recv, turn_join, cancel_token)** ✅ | mailbox 永远 polled；cancel 通过 token 透传到 TurnDriver；shutdown 看 in_flight 决定要不要等 |

这是 Claude Code "between turns" 原则在 Rust tokio 上的精确翻译。

### tokio Handle 注入（Q3 决策）

```rust
let runtime = Runtime::builder()
    .handle(tokio::runtime::Handle::current())     // 默认 = caller 当前 runtime
    // .handle(my_dedicated_handle)                // 想隔离就显式注入
    .conversation_store(Arc::new(jsonl_store))
    .model_gateway(Arc::new(anthropic_gateway))
    .tool_provider(Arc::new(builtin_tools))
    .job_manager(Arc::new(local_jobs))
    .build()?;
```

不传 handle → fallback `Handle::current()`，等价 tokio 库约定的行为，但保留了显式注入位 —— 一行配置切换隔离。

**multi-thread 检测**：build 时不强制要求 multi-thread；spawn 失败时 surface 报错（current_thread runtime 也能跑，单 session 顺序场景，测试友好）。production 用 multi-thread 是约定，文档写明，不在 builder 层强制。

#### 备选方案对比

| 方案 | 形态 | 放弃理由 |
|---|---|---|
| A · 库约定（无所谓现什么 runtime） | 不创建 runtime，所有 `tokio::spawn` 用 caller 当前的；要求 multi-thread | 把 multi-thread 检测做硬约束会挡掉测试和 current_thread；无显式注入位 = 后期想加 dedicated runtime 是 API break |
| **B · caller 显式注入 Handle**（选定） | `Runtime::builder().handle(...).build()` | 正交于消费方架构；测试友好；不强制 multi-thread |
| C · cogito 自带 runtime | 内部 `Runtime::new()` 起 multi-thread runtime | 跟"嵌入式库"定位有张力；多起一个 runtime 换不到额外安全 |

### 阻塞 I/O 路径（JSONL fsync）

走 `spawn_blocking`，不用 `tokio::fs`。理由：tokio 的 fs 实际上也是 `spawn_blocking` 套壳，但缺少批量优化空间；自己控制能在 H02 step recorder 里做"同一 turn 的 fsync 合批"（详见 §8）。

### Panic 隔离边界（顶层）

```rust
// SessionActor 主任务入口：
async fn session_loop(session_id: SessionId, ...) {
    let result = std::panic::AssertUnwindSafe(actor_main(...))
        .catch_unwind()  // ← 边界
        .await;
    match result {
        Ok(Ok(())) => { /* 正常 shutdown */ }
        Ok(Err(e)) => { /* actor 内部结构化错误，记日志 */ }
        Err(panic) => {
            // panic 捕获，写一条 SessionPanicked 事件（best-effort，可能 store 也挂了）
            tracing::error!(?panic, %session_id, "session actor panicked");
        }
    }
    // 任一种结束都从 Runtime::sessions 摘除
}
```

**保证**：单 session panic = 单 task 死亡 = process 其他 session 完全不受影响。这是 ADR-0005 §4 "Failure isolation" 的具体落地。完整三层 panic 边界详见 §9。

### vs Codex panic 处理

Codex 用 `Arc<Session>` + `Mutex`，panic 在 await 点中断时如果 mutex 在持有状态，会 poisoned —— Codex 接受这个代价（单用户 CLI 整体重启即可）。cogito 不能接受（一个 session panic 不能影响别人），所以选 actor 隔离 + catch_unwind。

---

## §4 · Session lifecycle

### 三个 open 模式

```rust
pub enum OpenMode {
    /// 新建：session_id 必须在 store 中不存在；写一条 SessionStarted 事件
    New,
    /// 复用已有 session：读 store 重建位置；store 中无记录则 panic（contract violation）
    Resume,
    /// 等同 Resume，但 store 无记录时返回 Err 而非 panic（更安全的 caller-facing API）
    Attach,
}

let handle: SessionHandle = runtime.open_session(session_id, OpenMode::Attach).await?;
```

### open 与 resume 的时序

```
caller                Runtime              SessionActor task
  │                      │                       │
  │─ open_session(…) ───▶│                       │
  │                      │── spawn ────────────▶ │ (catch_unwind boundary)
  │                      │                       │
  │                      │                       │── replay phase ──┐
  │                      │                       │   读 store.range(0..),│
  │                      │                       │   H03 决定 resume │
  │                      │                       │   point           │
  │                      │                       │◀──────────────────┘
  │                      │                       │
  │                      │                       │── ready signal ──┐
  │                      │◀──── oneshot ─────────│ ─ Ok(())          │
  │                      │      ready            │ ─ 或 Err(ResumeError)
  │◀── SessionHandle ───│                       │
  │                      │                       │
  │                      │                       │── mailbox loop ──▶
```

**关键决定**：`open_session()` 是 **async fn 且 await 直到 replay 完成**。replay 失败（log corrupt、schema 不兼容）在这里 surface 出来，而不是延后到第一条 input。

| 备选方案 | 放弃理由 |
|---|---|
| open 立即返回，replay 后台跑 | caller 第一条 send 突然报 ResumeError，error path 横穿正常代码；测试也难 |
| open 不 replay，第一条 input 触发 | 同上 + 第一条 input 异常慢，violates "subscribe 必须在 open 后立刻订阅" 约定 |

**vs Codex**：Codex `ConversationManager::resume_conversation_from_rollout(...)` 也是同步 `.await` 完成 replay 后再返回 handle，**形态一致**。

### Cancellation 与 lifecycle 语义（Q4 决策）

支持两级：**turn cancel + session shutdown**。

#### 场景动机

| 场景 | 用户期望 | 对应 API |
|---|---|---|
| ctrl-C 停当前 turn 但保留 session | 停 model stream + tool dispatch；turn 留 Cancelled 事件 | `handle.cancel_turn()` |
| 关掉 CLI 窗口 | flush buffer，关 store handle | drop `SessionHandle`（默认 shutdown timeout） |
| SIGTERM | 全 session 发 shutdown，超时强杀 | v0.4 实装 `runtime.shutdown_all()` |

**备选方案对比：**

| 方案 | 含义 | 选择 |
|---|---|---|
| A · 只做 session shutdown | drop handle = shutdown，无 turn cancel API | 放弃：长 stream/tool 没"刹车" caller 体验差 |
| **B · turn cancel + session shutdown** | `cancel_turn` + `shutdown(timeout)`，cooperative cancellation | 选定 |
| C · B + 进程级 shutdown_token | 全 session 收统一 broadcast | API 留位但 v0.1 不实装；v0.4 配合 ADR-0010 |

#### `cancel_turn` 实现

```rust
// SessionHandle::cancel_turn 实现
pub async fn cancel_turn(&self) -> Result<()> {
    // 不发 mailbox，直接 fire token
    self.shared.current_cancel_token.cancel();

    // 但 PausedOnJob 状态下 token 没人 select（TurnDriver 已经退出）
    // 走 mailbox 给 actor 一条 InternalCancel 命令，让 actor 调 jobs.cancel
    let (ack_tx, ack_rx) = oneshot::channel();
    self.shared.mailbox_tx
        .send(SessionCommand::InternalCancel { ack: ack_tx })
        .await?;
    ack_rx.await?
}
```

**两种路径合并的关键**：cancel_turn 同时做两件事 —— fire token（覆盖 Active 状态）+ 发 InternalCancel（覆盖 PausedOnJob 状态）。一次调用同时覆盖所有 in_flight 形态。

#### Cooperative cancel（非 abort）

- token 是 `Arc<CancellationToken>`，actor 每次 spawn TurnDriver 时**重建一个**并存进 `self.shared`，turn 结束自动废弃
- 所有可能长时间运行的 await 点都 `select!` 在 token 上，token 触发后 future 自己优雅退出
- **已经 in-flight 的 sync tool 不强杀**：让它跑完，结果记成 `ToolResult::Cancelled`（或正常 result，由 tool 决定）
- **abort 路径**（`task.abort()`）不用，因为 drop 时如果某个 future 在 RAII 中（比如握着 file lock），状态可能不一致

**vs Codex**：Codex 通过往 mailbox 发 `Op::Interrupt`，actor select 在 mailbox + 100ms grace 上；cogito 不走 mailbox 而走独立 token 字段，是因为 cogito mailbox 是严格 FIFO（背压模型简单），cancel 不该排在普通 input 后面。

### shutdown 与 drop

```rust
// 显式 shutdown，可控超时：
handle.shutdown(Duration::from_secs(5)).await?;

// 隐式 drop：等价于 shutdown(default_drop_timeout)，默认 5s
{ let handle = runtime.open_session(...); … }  // drop here
```

**actor 收到 `SessionCommand::Shutdown { ack, deadline }` 的行为：**

1. 关闭 mailbox 入口（后续 `handle.send` 直接 `Err(SessionClosed)`）
2. 检查 `in_flight`：
   - `Idle` → 直接进 cleanup
   - `Active(turn_join)` → 等 turn_join 完成或 deadline 到（到了就 `cancel_token.cancel()`，cooperative 等 100ms grace，仍不完成则 abort turn task）
   - `PausedOnJob { job_id, .. }` → 调 `job_manager.cancel(job_id)` + 写 `TurnCancelled`
3. **cleanup**：drain `persist_tx`，flush text-delta buffer，等 store writer 子任务 join
4. 通过 oneshot `ack` 回报 `ShutdownOutcome { clean: bool, in_flight_cancelled: Option<TurnState> }`

**注意**：shutdown 不写 `SessionEnded` 事件 —— session "结束" 是 caller 视角的概念，cogito 不臆断 caller 是否会重新 attach。

### budget enforcement

ADR-0005 §3 列了 per-session 三项：memory cap、turn 时间 cap、idle 内存目标 <1 MiB。

| 项 | 强制点 | v0.1 实现 |
|---|---|---|
| **turn 时间 cap** | actor 在 spawn TurnDriver 时记录 `started_at`；主 select 多一个 `sleep(remaining_budget)` arm，触发即调 `cancel_token.cancel()` 并写 `TurnFailed { reason: TurnTimedOut }`（详见 §5 actor_main，本 spec v1 简化未画出此 arm，实施时补） | hard limit；cooperative cancel 先尝试 100ms，仍不退则 abort turn task |
| **memory cap** | 不强制，只测量 | v0.1 只 expose `MetricsRecorder::record_session_mem(...)` hook；真正强制留 v0.4 跟 sandbox 一起做 |
| **token / cost cap** | hook 在 H09 的 `pre_prompt` 拦截 | 由 strategy 配置，不在 Runtime 层 |

---

## §5 · Turn lifecycle inside actor

### actor 主循环（伪代码，最准确的规格）

```rust
async fn actor_main(state: ActorState) -> Result<()> {
    // 阶段 1：resume on open（见 §4）
    state.replay_and_position().await?;
    state.ready_tx.send(Ok(())).ok();

    // 阶段 2：mailbox loop
    loop {
        tokio::select! {
            // 当前有 turn 在跑：等它完成
            outcome = async { state.in_flight.as_mut().unwrap().join().await },
                if matches!(state.in_flight, Some(InFlight::Active(_))) => {
                    state.handle_turn_outcome(outcome).await?;
            }

            // mailbox 永远 polled
            Some(cmd) = state.mailbox_rx.recv() => {
                match cmd {
                    SessionCommand::Input(msg) => state.try_start_turn(msg).await?,
                    SessionCommand::JobCompleted { job_id, result }
                        => state.try_resume_from_job(job_id, result).await?,
                    SessionCommand::InternalCancel { ack }
                        => state.handle_internal_cancel(ack).await?,
                    SessionCommand::Shutdown { ack, deadline }
                        => { state.shutdown(deadline, ack).await?; break; }
                }
            }

            else => break,  // mailbox closed + 无 in_flight
        }
    }

    state.cleanup().await
}
```

**两个 select arm 是关键**：

- `outcome = … if matches!(in_flight, Active(_))` ：guard 确保只在有 turn 时才 await join handle（否则 None.unwrap 会爆）
- `mailbox.recv()`：永远 polled，turn 在跑时新 input 仍然能进 mailbox 排队

### TurnDriver 子任务：H01 FSM 的具体跑法

```rust
async fn turn_driver(req: TurnRequest, ctx: TurnContext) -> TurnOutcome {
    let mut state = TurnState::Init;
    let cancel = ctx.cancel.clone();

    loop {
        // 1. 写一条 state-transition 事件（先写后转，ADR-0003 不变量）
        ctx.persist(turn_state_to_event(&state)).await?;

        // 2. 检查 cancel
        if cancel.is_cancelled() {
            return TurnOutcome::Cancelled;
        }

        // 3. 状态转移
        state = match state {
            TurnState::Init => {
                let strategy = ctx.harness.strategy_for(&req).await?;
                let (prompt, surface) = ctx.harness.compose(&req, &strategy).await?;
                TurnState::PromptBuilt { prompt, surface, strategy }
            }
            TurnState::PromptBuilt { prompt, surface, strategy } => {
                let stream = ctx.harness.model.stream(prompt, &ctx.exec_ctx).await?;
                TurnState::ModelCalling { stream, surface, strategy }
            }
            TurnState::ModelCalling { stream, surface, strategy } => {
                let result = ctx.harness.demux(stream, &cancel, &ctx.persist).await?;
                TurnState::ModelCompleted { result, surface, strategy }
            }
            TurnState::ModelCompleted { result, .. } => {
                if let Some(calls) = ctx.harness.resolve_tool_calls(&result)? {
                    TurnState::ToolDispatching { calls, … }
                } else {
                    return TurnOutcome::Completed;
                }
            }
            TurnState::ToolDispatching { calls, .. } => {
                match ctx.harness.dispatch(calls, &cancel, &ctx.exec_ctx).await? {
                    DispatchOutcome::AllSync(results) => {
                        // 把 ToolResult 写进下一轮 prompt 的 history，回到 Init
                        TurnState::Init  // re-enter loop for next sub-turn
                    }
                    DispatchOutcome::Async(job_id) => {
                        return TurnOutcome::Paused { job_id };
                    }
                }
            }
        };
    }
}
```

**几条关键不变量：**

1. **每个 state 持有自己流转所需的全部上下文**（`prompt`、`stream`、`surface`、`strategy`）—— 不放进 actor，因为崩了重建从 store 重新走
2. **FSM 不持久化 state 本身**，持久化的是 **状态转移事件**（`TurnEntered { state: PromptBuilt }`）。H03 read 事件流时根据"末尾事件 + 末尾状态"反推该从哪个状态恢复
3. **多轮工具调用走 inner loop**（`TurnState::Init` 再入），不递归调用 `turn_driver`。一次 `turn_driver` 调用 = 一次完整的 "input → final answer 或 paused" 周期
4. **ToolDispatching 路径分叉**：全 sync 完成 → re-enter Init；任一 async → 立即返回 Paused，actor 切到 PausedOnJob 状态

### 与 ADR-0003 的关系

ADR-0003 给了状态图，没给"状态在 Rust 里是 struct 还是 enum"的答案。本 spec 选 **`enum TurnState`**（algebraic，每个 variant 携带流转上下文），不选 struct + 字段。理由：

- enum 让 "ModelCalling 必有 stream" 这种状态-数据不变量被类型系统强制
- 跳过状态（比如从 Init 直接跳到 ToolDispatching）是 type error，不是运行时 bug

---

## §6 · Async job lifecycle

### 同步任务 vs 异步任务的判定机制

H08 Tool Dispatcher 最关键的分叉点。三个候选机制：

| 方案 | 判定方 | 判定时机 | 典型工具 |
|---|---|---|---|
| **(1) Static per-tool** | tool 作者 | 注册时（`ToolDescriptor` 里声明） | `read_file` = Sync；`run_tests` = Async；二选一固定 |
| **(2) Runtime per-call** | tool 实现内部 | 每次 invoke 时根据参数判断 | `transcribe_audio(uri)`：clip < 30s 走 Sync，否则走 Async |
| **(3) LLM 显式决策** | 模型 | tool call 参数里带 `run_in_background: bool` | Claude Code 的 `Bash` 工具就是这套 |

**cogito v0.1 选 (1) + (2) 组合，明确禁止 (3)**。

#### 为什么不选 (3)

把"是否异步"暴露给 LLM 的代价：

1. LLM 要懂"什么任务该后台跑"这种**执行模型层**的知识 —— 这是 runtime 责任，不该污染 prompt
2. LLM 可能"误判"：把短任务标 background → 浪费一次 mailbox 往返；把长任务标 sync → 模型 stream 卡几分钟 timeout
3. 跨 model 不兼容：Anthropic 跟 OpenAI 的 tool spec 不一定都允许这种元参数；如果某个 provider 不允许，整套形态崩了
4. **Claude Code 用 (3) 是因为它是 single-user CLI**，LLM 跟运行环境是同一个交互场景；cogito 是嵌入式 runtime，要支持任意 caller / 任意模型

#### 为什么 (1) 不够

固定 Static 决定不了 `transcribe_audio` 这种 input-dependent 的情况；写两个 tool（`transcribe_audio_short` / `transcribe_audio_long`）然后让 LLM 选，本质上变成 (3)，更糟。

#### (1) + (2) 的具体形状

```rust
// cogito-protocol
pub struct ToolDescriptor {
    pub name: String,
    pub schema: JsonSchema,
    pub description: String,
    pub execution_class: ExecutionClass,  // ← 新增
    pub outputs_model_visible_multimodal: bool,
}

pub enum ExecutionClass {
    /// 总是同步返回。tool 实现必须返回 InvokeOutcome::Sync；返回 Async 是 contract violation。
    /// 典型：read_file / now / parse_json
    AlwaysSync,

    /// 总是异步返回。tool 实现必须返回 InvokeOutcome::Async；返回 Sync 是 contract violation。
    /// 典型：run_tests / build_release / provision_vm
    AlwaysAsync,

    /// 由 tool 实现自己按参数决定，每次 invoke 都可能不同。
    /// 典型：transcribe_audio（按 clip 长度），summarize_video（按视频长度）
    Adaptive,
}

#[async_trait]
pub trait ToolProvider: Send + Sync {
    fn descriptors(&self) -> &[ToolDescriptor];
    async fn invoke(&self, name: &str, args: Value, ctx: &ExecCtx) -> Result<InvokeOutcome>;
}

pub enum InvokeOutcome {
    Sync(ToolResult),
    Async(JobId),
}
```

#### 判定时机的精确顺序

```
H08 Tool Dispatcher 收到一个 tool call
  │
  ├─ 1. 查 ToolDescriptor.execution_class
  │     ├─ AlwaysSync → 期望 invoke() 返回 Sync(...)；其它情况 = Bug
  │     ├─ AlwaysAsync → 期望 invoke() 返回 Async(JobId)；其它情况 = Bug
  │     └─ Adaptive → 两种都可接受
  │
  ├─ 2. 调用 provider.invoke(name, args, ctx).await
  │     ├─ 返回 Sync(result) → H08 直接生成 ToolCallCompleted 事件
  │     └─ 返回 Async(job_id) → H08 让 TurnDriver 立刻返回 Paused{job_id}
  │
  └─ 3. contract violation 检查（debug build 用 debug_assert!；release 写 warn log + 按返回值实际类型走）
```

#### Adaptive 情况下，tool 内部的判定例子

```rust
// cogito-tools-multimedia 里 transcribe_audio 的实现（v0.2+）
async fn invoke(&self, name: &str, args: Value, ctx: &ExecCtx) -> Result<InvokeOutcome> {
    let uri = args["uri"].as_str().ok_or(...)?;
    let meta = ctx.storage.resolve(uri).await?;

    // 判定：clip < 30s 用 Sync 路径（直接调 API 等返回）
    //        clip >= 30s 用 Async 路径（submit 给 LocalJobManager，返回 JobId）
    if meta.duration < Duration::from_secs(30) {
        let text = whisper_api_sync(uri).await?;
        Ok(InvokeOutcome::Sync(ToolResult::text(text)))
    } else {
        let job_id = self.local_jobs.submit(async move {
            whisper_api_long(uri).await
        });
        Ok(InvokeOutcome::Async(job_id))
    }
}
```

#### JobManager 的 submit 路径

ARCHITECTURE.md 之前留的空（"JobManager 不暴露 submit"）的具体落地：

| 视角 | 看到的 API |
|---|---|
| Brain（H08 dispatcher） | `Arc<dyn JobManager>`，只有 `status` / `result` / `cancel` / `on_complete` |
| Hands（async tool 实现） | `Arc<LocalJobManager>`（**concrete type**），既有 trait 方法又有 `submit(future) -> JobId` |

具体 Hands crate（比如 `cogito-tools-multimedia`）在 Cargo.toml 直接依赖 `cogito-jobs`（concrete），通过 DI 拿 `Arc<LocalJobManager>` 注入到 tool struct 里。Brain 只通过 `Arc<dyn JobManager>` 看 JobManager —— 仍然满足 ADR-0004 的"Brain only sees Protocol"。

这是 ADR-0004 §1 表里 "Hands (Brain-facing)" 和 "Hands (internal primitive)" 区分的延伸：**JobManager 同一个 crate 同时提供 trait（Brain 侧）+ 具体类型（Hands 侧）**，这是 v0.1 简化形态；v0.2+ 如果出现第二个 JobManager 实现（distributed），再抽 `JobSubmitter` 单独 trait。

#### H05 Tool Surface Builder 的过滤介入

`HarnessStrategy` 可以配置 `allow_async_tools: bool`（默认 true）。H05 据此过滤暴露给 LLM 的工具：

- 关掉 → 所有 `AlwaysAsync` + `Adaptive` 工具不上 prompt
- 开启 → 全部上

这给 strategy 一个干净的开关："这个 role 不能跑长任务"（比如 critic role 只该读不该跑），不需要改 tool registry。

### Async job 完整时序

```
TurnDriver task            SessionActor               JobManager task
     │                          │                          │
     │ dispatch tool returns    │                          │
     │ InvokeOutcome::Async ────▶                          │
     │ (job_id)                 │                          │
     │                          │ register on_complete    │
     │                          │  sender for job_id ─────▶
     │ return                   │                          │
     │ TurnOutcome::Paused ─────▶                          │
     │ (job_id)                 │                          │
   ──┘                          │ persist TurnPaused      │
                                │ in_flight = PausedOnJob │
                                │ continue mailbox loop ──▶ (idle except for new input)
                                │                          │
                                │                          │ job task completes
                                │◀── JobCompleted ─────────│
                                │   { job_id, result }     │
                                │                          │
                                │ matches in_flight?       │
                                │   yes → spawn new        │
                                │   TurnDriver with state  │
                                │   = ToolDispatching      │
                                │     resumed              │
                                │   no  → ignore (stale    │
                                │                  job)     │
```

### Job ↔ TurnDriver 详细 sequence

```
SessionActor              TurnDriver #1          ToolProvider          JobManager            TurnDriver #2
   task                    task (短命)            (Hand impl)          tasks (per job)         task (短命)
    │                         │                      │                      │                       │
    │ ← Input(msg) ───────────┤                      │                      │                       │
    │   (mailbox)             │                      │                      │                       │
    │                         │                      │                      │                       │
    ├─ spawn ─────────────────▶ ◍                    │                      │                       │
    │                         │ FSM: Init→Prompt     │                      │                       │
    │                         │ →ModelCalling→...    │                      │                       │
    │                         │ →ToolDispatching     │                      │                       │
    │                         │                      │                      │                       │
    │                         ├─ tools.invoke(…) ────▶                      │                       │
    │                         │                      │ 内部决定走 async    │                       │
    │                         │                      │ 自己 submit job ────▶ ◍ spawn job task     │
    │                         │                      │                      │ (实际执行 60s)        │
    │                         │ ◀─ Async(job_id) ────┤                      │                       │
    │                         │                      │                      │                       │
    │                         │ persist TurnPaused   │                      │                       │
    │                         │ (走 persist_tx)      │                      │                       │
    │                         │                      │                      │                       │
    │ ◀─ join: Paused{job_id} ┤ ✗ (TurnDriver #1 dies)                      │                       │
    │                                                                       │                       │
    ├─ register on_complete ─────────────────────────────────────────────────▶                      │
    │   sink = job_completion_tx.clone()                                    │                       │
    │                                                                       │                       │
    │ in_flight = PausedOnJob{job_id}                                       │                       │
    │ ↻ mailbox loop (idle; 仍可处理新 Input / Cancel / Shutdown)          │                       │
    │                                                                       │                       │
    │                                                                       │ job completes        │
    │ ◀─ JobCompletionEvent{job_id, outcome} ───────────────────────────────┤                       │
    │   (job_completion_rx)                                                 │                       │
    │                                                                       │                       │
    │ in_flight 匹配？是 → 合成 SessionCommand::JobCompleted 入 mailbox     │                       │
    │ (经过 mailbox 是为了跟其它 cmd 排队，FIFO 不变量)                    │                       │
    │                                                                       │                       │
    │ 处理 JobCompleted → spawn TurnDriver #2 ──────────────────────────────────────────────────────▶ ◍
    │   resume_state = ToolDispatching{ tool_result: outcome.into(), ... }                          │
    │                                                                       │                       │
    │                                                                       │                       │ FSM 从 ToolDispatching 续
    │                                                                       │                       │ → 把 tool_result 喂回 model
    │                                                                       │                       │ → ModelCalling → ...
    │                                                                       │                       │ → Completed
    │ ◀─ join: Completed ───────────────────────────────────────────────────────────────────────────┤ ✗
    │                                                                       │                       │
```

#### 关键不变量

| 不变量 | 含义 |
|---|---|
| **TurnDriver 不 await Job** | TurnDriver 见到 `InvokeOutcome::Async` 立刻终结自己；Job 的等待完全由 actor + JobManager 配合，TurnDriver 不参与 |
| **Job 不直接通知 TurnDriver** | JobManager 完成时往 actor 的 `job_completion_tx` 发，actor 决定下一步；Job 永远不知道有几个 TurnDriver 接力过 |
| **接力 turn 用新 TurnDriver task** | 不复用任何对象，新 task 拿 store + 新 cancel_token + resume_state 起步 |
| **resume_state 是从 store 里反查出来的** | 不是 actor 在内存里偷偷保存的；这是 ADR-0002/0003 "state in store, not in memory" 的具体落地 |
| **JobCompletionEvent → SessionCommand::JobCompleted 经过 mailbox** | 不是直接处理。这样 FIFO 不变量保留（新 Input 在前先处理，Job 在后再处理） |

### `JobManager` trait（v0.1 形状）

```rust
// cogito-protocol
#[async_trait]
pub trait JobManager: Send + Sync {
    async fn status(&self, job_id: JobId) -> Result<JobStatus>;
    async fn result(&self, job_id: JobId) -> Result<JobOutcome>;
    async fn cancel(&self, job_id: JobId) -> Result<()>;

    /// 新增：注册完成回调。
    /// `sink` 在 job 终态时收到 1 条消息，之后 sender 被 drop。
    /// 实现允许 best-effort：sink 关闭则丢弃（actor 已经死了的话无意义）。
    async fn on_complete(
        &self,
        job_id: JobId,
        sink: mpsc::Sender<JobCompletionEvent>,
    ) -> Result<()>;
}
```

**为什么 callback 用 mpsc Sender 而不是 oneshot？**
actor 复用一个 mpsc sender 接收**所有** job 的完成事件（avoid 持 N 个 oneshot 句柄），sender clone 给 JobManager，每个 job 完成时各发一条 `JobCompletionEvent { job_id, outcome }`。`JobCompletionEvent` 在 actor 内部转 `SessionCommand::JobCompleted` 入 mailbox。

### 候选机制对比

| 选项 | 含义 | 选择 |
|---|---|---|
| A · actor 在 Paused 时阻塞等 JobManager | `let result = job_manager.await_result(job_id).await?;` | 放弃：actor 卡住 = mailbox 不读 = cancel/shutdown 进不来 |
| **B · actor Paused 后释放控制权，JobManager 通过 mailbox 唤醒** | 上述形态 | 选定 |
| C · actor 自己 select 在 mailbox + JobManager 信号上 | `tokio::select!(mailbox.recv(), job_handle.wait())` | 放弃：双 loop 模式 cancel-unsafe 风险高 |

### 崩溃恢复路径

actor 在 PausedOnJob 状态时进程崩溃，重启后 Runtime rebuild session：

```
1. open_session(id, Resume) 进入 replay 阶段
2. H03 扫 store log，末尾事件是 TurnPaused { job_id }
3. H03 → ctx.jobs.status(job_id) 查询：
   ├─ Completed → 合成 JobCompletionEvent → 立刻进 mailbox 处理 → resume turn
   ├─ Running   → 重新注册 on_complete sink → in_flight = PausedOnJob → 等
   └─ NotFound  → JobManager 状态丢失 → 写 TurnFailed { reason: JobStateLost } → in_flight = None
```

**这一步是 ADR-0003 没明写的**，本 spec 把它加进 H03 的决策表。

### SaaS scalability of this design

Q6.B 的 callback 形态（`JobManager::on_complete(job_id, sink)` + `SessionCommand::JobCompleted` 入 mailbox）属于 "durable execution" 派的轻量化形态（参考 Inngest `waitForEvent`、Temporal Signal、LangGraph Redis Pub/Sub）。v0.4 SaaS-ready 阶段的具体形态：

- **`JobManager` trait 形状不变**（这是为什么 v0.1 必须用 trait method 而非具体回调函数）
- **`cogito-jobs-distributed` 实现**：Redis Stream 当作 job queue + completion pub/sub，consumer 决定使用本地或外接到 Inngest/Temporal
- **Actor 重建路径**：worker 通过 store + `JobManager.status` 查询从任意节点重建 session（"any worker can resume"，跟 LangGraph/Inngest 一致）
- **拒绝的路径**：
  - Manus-style sticky session：违反 ADR-0001 state-in-store
  - LangGraph-style 缺 distributed locking：cogito 利用 store fsync + 事件单调 ID 检测 fork，写 `SessionForked` 事件让其中一个 worker 退出（v0.4 ADR 细化）

ADR-0005 §1 "future-SaaS-readiness preserved via trait-based pluggability" 通过本设计具体兑现。

---

## §7 · Channel & memory model

### channel 拓扑（完整清单）

| channel | 类型 | 容量 | 方向 | backpressure 策略 |
|---|---|---|---|---|
| **`mailbox`** | `mpsc::channel<SessionCommand>` | 64 | caller → actor | sender `send().await` 阻塞 |
| **`events_out`** | `broadcast::channel<StreamEvent>` | 256 | actor → subscribers | 慢 subscriber 收 `Lagged(n)`，自行降级 |
| **`persist_tx`** | `mpsc::channel<PersistCommand>` | 256 | actor → store writer subtask | sender `send().await` 阻塞 → turn 推进减速 |
| **`job_completion_tx`** | `mpsc::channel<JobCompletionEvent>` | 32 | JobManager → actor | sender `send().await` 阻塞；32 足够（同一 session 同时 in-flight job 不会多） |
| **`ready_tx`** (open_session) | `oneshot<Result<()>>` | 1 | actor → caller | 一次性，replay 完成信号 |
| **`shutdown_ack`** | `oneshot<ShutdownOutcome>` | 1 | actor → caller | 一次性，shutdown 完成信号 |
| **`turn_outcome`** (TurnDriver → actor) | `oneshot<TurnOutcome>` | 1 | turn task → actor | 一次性（通过 JoinHandle 间接） |

**容量选择理由：**

- **mailbox = 64**：caller 想 batch 发 input 不至于立刻阻塞，但也不至于积压太多。`SessionCommand` 平均 ~256 B，64 个 = 16 KB，可接受
- **events_out = 256**：UI 一秒能消费几百 chunk，256 给 1s 缓冲；慢 UI 漏事件由 caller 处理（broadcast 标准模式）
- **persist_tx = 256**：store fsync per event 假设 ~1ms（Sprint 1 SLO P99<5ms），256 给 256ms 缓冲；满了说明 store 跟不上，需要 SLO 报警。**容量数字照搬 Codex** (`rollout/recorder.rs:244`)
- **job_completion_tx = 32**：同一 session 同时 pending 的 async job 数极小（v0.1 sequential dispatch，最多 1）；32 是松散上界

所有数字进 §11 Open TBDs（TBD-C1），Sprint 1 benchmark 后微调。

### channel-by-channel Codex 对照

| cogito channel | 类型 / 容量 | Codex 对应 | Codex 类型 / 容量 | 关键差异 |
|---|---|---|---|---|
| **M** mailbox | 64 mpsc | `SubmissionLoop` 接收 `Submission` (`Op` enum) | mpsc | Codex 用 `Op::Interrupt` 当 cancel 走 mailbox；cogito cancel 走独立 token（避免 FIFO 排队） |
| **E** events_out | 256 broadcast | app-server JSON-RPC `notification` 输出（stdout） | 不是 channel，是序列化输出流 | **Codex 是 single subscriber**（一个 client 连接）；cogito 用 broadcast 支持 N subscribers |
| **P** persist_tx | 256 mpsc | `Sender<RolloutCmd>` in `SessionServices` | mpsc 256（`rollout/recorder.rs:244`） | **容量数字一致**。Codex 用 `tokio::fs` 异步写 + 按需 flush，**无 per-event fsync**；cogito v0.1 选 `spawn_blocking` + per-event fsync |
| **J** job_completion | 32 mpsc | **不存在** | N/A | Codex 没有 async job 概念，无对应；cogito 独有 |
| **Rd** ready oneshot | 1 | `ConversationManager::resume_conversation_from_rollout(...).await` 直接 await 完成 | 隐式（async fn 返回） | 形态相同 |
| **Sa** shutdown_ack | 1 | Codex 没有显式 graceful shutdown API | N/A | Codex 是 CLI；cogito 是 lib，必须有显式 shutdown |
| **To** turn_outcome（JoinHandle 隐式） | 1 | `ActiveTurn` 里的 `Mutex<TaskState>` | `Arc<Mutex<_>>` | **本质差异**：Codex 共享 mutex 同步 active turn；cogito spawn 子 task + JoinHandle，actor 在 select 上等 |

### Channel 数据流：三个典型场景

#### Channel 编号

```
M  : mailbox            mpsc<SessionCommand>          caller → actor
E  : events_out         broadcast<StreamEvent>        actor → subscribers
P  : persist_tx         mpsc<PersistCommand>          actor / TurnDriver → store writer
J  : job_completion     mpsc<JobCompletionEvent>      JobManager → actor
Rd : ready (oneshot)    oneshot<Result<()>>           actor → caller (open 完成)
Sa : shutdown_ack       oneshot<ShutdownOutcome>      actor → caller
To : turn_outcome       oneshot<TurnOutcome>          TurnDriver → actor (JoinHandle 间接)
```

#### 场景 A · 最简 turn（无 tool 调用）

```
caller          actor          TurnDriver       store writer       subscribers
  │   M:Input     │               │                  │                 │
  │ ─────────────▶                │                  │                 │
  │              spawn ───────────▶                  │                 │
  │               │             FSM 跑               │                 │
  │               │       ────P:TurnEntered─────────▶ fsync            │
  │               │                                  │                 │
  │               │       ────E:TurnStarted──────────────────────────▶
  │               │                                  │                 │
  │               │ (ModelCalling: stream demux)     │                 │
  │               │       ────P:AssistMsg(batched)──▶ fsync            │
  │               │       ────E:TextDelta────────────────────────────▶ (per chunk)
  │               │       ────E:TextDelta────────────────────────────▶
  │               │       ────E:TextDelta────────────────────────────▶
  │               │       ────P:TurnEntered{Completed}──▶ fsync        │
  │               │       ────E:TurnCompleted──────────────────────────▶
  │             join ◀─── To:Completed                                  │
  │               │ ✗                                                   │
  │ in_flight = None                                                    │
  │ 继续 mailbox loop                                                   │
```

要点：

- `P`（持久化）和 `E`（实时）**两条独立 channel**，TurnDriver 同一个事件可能两边都发（但格式不同：P 是合批后的 `ConversationEvent`，E 是逐 chunk 的 `StreamEvent`）
- text_delta 在 P 这边批合（200ms / 500 chars），在 E 这边逐条
- caller 不见 `To`（它是 TurnDriver→actor 的内部 JoinHandle），caller 通过 `subscribe()` 拿 `E`

#### 场景 B · 有 sync tool 的 turn

```
TurnDriver       ToolProvider     subscribers          store writer
   │   invoke      │                  │                     │
   │ ─────────────▶                  │                     │
   │       ─────E:ToolDispatchStarted────────────────────▶  │
   │       ─────P:ToolCallRequested─────────────────────────▶ fsync
   │              read_file 同步执行   │                     │
   │ ◀── Sync(ToolResult) ─                                  │
   │       ─────P:ToolCallCompleted─────────────────────────▶ fsync
   │       ─────E:ToolDispatchEnded────────────────────────▶
   │ FSM re-enter Init (next sub-turn 把 tool result 喂回 model)
   │ ... 继续场景 A 的 ModelCalling
```

#### 场景 C · async job + cancel 的完整生命

```
caller         actor          TD#1          tools(async)    JobManager       TD#2     subscribers
  │  M:Input    │              │               │              │               │            │
  │ ───────────▶               │               │              │               │            │
  │           spawn ───────────▶               │              │               │            │
  │             │     invoke ──▶               │              │               │            │
  │             │               │  submit ─────▶               ▶ spawn job task            │
  │             │     ◀── Async(job_id) ───────                              │            │
  │             │     P:TurnPaused ────────────────────────────────▶ writer  │            │
  │             │     E:TurnPaused ───────────────────────────────────────────────────────▶
  │           join ◀── To:Paused                                              │            │
  │             │ ✗                                                            │            │
  │  in_flight = PausedOnJob{job_id}                                          │            │
  │  on_complete sink registered ─────────────────────────────▶              │            │
  │             │                                                              │            │
  │  ====== 用户决定取消 ======                                                │            │
  │  cancel_turn() (直接调 cancel_token.cancel()，不走 M)                     │            │
  │             │                                                              │            │
  │  (current turn 没有 TD task 在跑，cancel_token 没人 select 它)            │            │
  │  →  actor 主动调 jobs.cancel(job_id) （在 cancel_turn 实现里检测到        │            │
  │      in_flight = PausedOnJob 就这么走）                                   │            │
  │  ──────────────────────────────────────────▶ job task 收到 cancel        │            │
  │  P:TurnCancelled ────────────────────────────────────────▶ writer        │            │
  │  E:TurnCancelled ──────────────────────────────────────────────────────────────────────▶
  │  in_flight = None ↻ mailbox loop                                          │            │
  │                                                                            │            │
  │  ====== 假设用户没取消 ======                                              │            │
  │  (job 跑完)                                                                │            │
  │             │ ◀── J:JobCompletionEvent{job_id, outcome} ─────────────────┤            │
  │  in_flight 匹配 → 合成 SessionCommand::JobCompleted 入 M（自己发自己）   │            │
  │  M:JobCompleted ────────────────────────────────────────────────▶ (自己接) │            │
  │  spawn TD#2 with resume_state = ToolDispatching{tool_result=outcome}     ─▶ ◍          │
  │  E:TurnResumed ─────────────────────────────────────────────────────────────────────────▶
  │             │                                                              │           │
  │             │  TD#2 把 outcome 转 ToolResult → 喂回 model → ModelCalling → Completed   │
  │             │                                                              │           │
  │           join ◀── To:Completed                                            ─┤ ✗        │
```

这张图揭示的几个隐藏不变量：

1. **cancel 在 PausedOnJob 状态下也能用**：actor 内部走 `jobs.cancel(job_id)`，不是依赖 cancel_token
2. **JobCompletionEvent 到 actor 后再合成 SessionCommand 入 mailbox**，不直接处理。FIFO + 跟 caller 发的新 Input 公平排队
3. **events_out 上有 `TurnResumed` 这种 stream-only 事件**，store 不写 —— 因为 store 的 `TurnPaused` 已经隐含"下次出现新事件 = resumed"，无需双写

### 双流（持久 + 实时）的设计选择 vs 候选

cogito 里有两类 "event"：

| 类型 | 目的 | 谁消费 | text_delta 处理 | 一定要持久 |
|---|---|---|---|---|
| **ConversationEvent** | resume 用，全量重放 | H03、`ConversationStore`、replay 工具 | **批合** 200ms 或 500 字符再写 | ✅ |
| **StreamEvent** | UI/TUI 实时刷屏、调用方 progress、metrics adapter | TUI、CLI、observability、consumer | **不批合**，每个 chunk 都推 | ❌ 可丢 |

#### 候选对比

| 方案 | 含义 | 选择 |
|---|---|---|
| A · 单流，subscriber 只能拿 `ConversationEvent` | actor 只往 store 写 + 往 broadcast 推同一份 | 放弃：text_delta 被批合，UI 卡顿 |
| **B · 双流，分别投递** | 两个 channel：`tx_persist` + `tx_stream` | 选定 |
| C · 单流 + 不同 fidelity | 只有一个 broadcast `StreamEvent`，store writer 自己订阅 + 批合后写入 | 放弃：broadcast 拥塞时 store subscriber 可能掉事件 → 持久化丢失 |

#### vs Codex / Claude Code

| 维度 | Codex Rust | Claude Code | cogito B |
|---|---|---|---|
| 类型 | 单 `EventMsg` enum | conversation inject | `ConversationEvent` + `StreamEvent` 双类型 |
| stream 数 | 双（JSON-RPC stdout + rollout JSONL） | 双（live + persist） | 双（broadcast + mpsc to store） |
| subscriber 数 | 1 (app-server connection) | 1 (LLM) | N (broadcast) |
| catchup | rollout file replay | conversation 本身 | v0.1 不保证；v0.4 加 `subscribe_from(event_id)` |

cogito 同形态于 Codex/Claude Code 的双 stream 拓扑，但**主动选 broadcast 而非 mpsc**，是因为嵌入式 lib 多 subscriber 是常态：TUI + observability + consumer hook 三者同时挂监听不应互相阻塞。

### SessionActor 持有的状态（完整清单）

```rust
struct ActorState {
    // 身份与配置（创建后不变）
    session_id: SessionId,
    handle_runtime: tokio::runtime::Handle,

    // 协议层注入的 Hands/Boundary/Session（Arc 共享）
    store: Arc<dyn ConversationStore>,
    model: Arc<dyn ModelGateway>,
    tools: Arc<dyn ToolProvider>,
    hooks: Arc<dyn HookHandler>,
    jobs: Arc<dyn JobManager>,
    strategy_selector: Arc<dyn StrategySelector>,
    metrics: Arc<dyn MetricsRecorder>,

    // channel 端点（actor 这端）
    mailbox_rx: mpsc::Receiver<SessionCommand>,
    events_tx: broadcast::Sender<StreamEvent>,
    persist_tx: mpsc::Sender<PersistCommand>,
    job_completion_rx: mpsc::Receiver<JobCompletionEvent>,
    job_completion_tx: mpsc::Sender<JobCompletionEvent>, // clone 给 JobManager

    // 跨 turn 的运行时状态（**唯二**两个字段，且都可从 store 重建）
    in_flight: Option<InFlight>,
    current_cancel_token: CancellationToken, // 每 turn 重建

    // 资源 budget
    budget: SessionBudget,
}

enum InFlight {
    Active {
        turn_join: JoinHandle<TurnOutcome>,
        started_at: Instant,
    },
    PausedOnJob {
        job_id: JobId,
        paused_at_event_id: EventId,  // 用于 resume 时定位
    },
}
```

### "跨 turn 状态" 的不变量检查

AGENTS.md inviolable principle #3："State lives in Conversation Service, not in Harness memory." 本 spec 落地：

| ActorState 字段 | 跨 turn 持有理由 | 可从 store 重建吗 |
|---|---|---|
| `session_id` / `handle_runtime` / 各 Arc | 配置类，进程内不变 | N/A（重建时 Runtime 重新注入） |
| `mailbox_rx` / `events_tx` / `persist_tx` / `job_completion_*` | tokio runtime 资源 | N/A（重建时新建） |
| `in_flight: Active` | turn 在跑的 join handle | **不能** —— 但崩溃后 turn 必然挂了，重建时一定是 `None` |
| `in_flight: PausedOnJob` | 用于路由 JobCompleted | **能** —— store 最末事件是 `TurnPaused { job_id }` |
| `current_cancel_token` | 让 `handle.cancel_turn()` 能影响当前 turn | N/A（重建时新建） |
| `budget` | 资源使用累计 | v0.1 不持久化（每次 open 从 0 计；v0.4 移入 store） |

**结论**：违反 #3 的只有 `budget`，且 v0.1 文档明记 "budget is per-actor-lifetime, not per-session-lifetime；v0.4 移入 store"。其余全部满足。

### 内存占用估算（对照 ADR-0005 idle <1 MiB 目标）

| 项 | 估算 |
|---|---|
| ActorState 本身（不算 channel buffer） | ~800 B |
| mailbox buffer (64 × ~256 B) | 16 KB |
| events_out buffer (256 × ~512 B) | 128 KB |
| persist_tx buffer (256 × ~1 KB ConvEvent) | 256 KB |
| Arc fat pointers (~10 个 trait object) | ~160 B |
| store writer subtask stack + 缓冲 | ~64 KB |
| **小计（idle 时）** | **~465 KB** |
| **预算余量** | ~559 KB |

**结论**：1 MiB 目标可达，但 events_out + persist_tx 的容量是大头，Sprint 1 benchmark 时如果实测高于此，优先压缩这两个 buffer。

---

## §8 · Storage write path

### H02 在 actor 模型里的具体形态

ARCHITECTURE/H02 文档说 "H02 is called by every component" —— 这是逻辑视角。物理视角下，**H02 = persist_tx producer 侧 + store writer subtask consumer 侧的复合体**。没有一个叫 `Step Recorder` 的对象被"调用"，而是组件通过 `persist_tx.send(...)` 间接持久化。

```
┌──── H02 producer 侧（每个写事件方都做这一步） ────┐
│                                                    │
│ // 在 TurnDriver / actor 主循环 / hook 调用点：    │
│ let (ack_tx, ack_rx) = oneshot::channel();         │
│ persist_tx.send(PersistCommand::Append {           │
│     event: conversation_event,                     │
│     ack: Some(ack_tx),  // 阻塞等 ack              │
│ }).await?;                                         │
│ ack_rx.await??;                                    │
│ // 此时事件已 fsync 落盘，FSM 才能转移             │
│                                                    │
└────────────────────────────────────────────────────┘
                       │
                       │ mpsc 256
                       ▼
┌──── H02 consumer 侧（store writer subtask） ────┐
│                                                  │
│ async fn store_writer(                           │
│     mut rx: mpsc::Receiver<PersistCommand>,      │
│     store: Arc<dyn ConversationStore>,           │
│ ) {                                              │
│     let mut delta_buffer = TextDeltaBuffer::new();│
│                                                  │
│     loop {                                       │
│         tokio::select! {                         │
│             biased;                              │
│             cmd = rx.recv() => match cmd {       │
│                 Some(PersistCommand::Append { event, ack }) => {│
│                     // 1. 状态转移类事件 → 先 flush buffer，再写它 │
│                     if !event.is_text_delta() {  │
│                         flush_buffer(&mut delta_buffer, &store).await?;│
│                         append_with_fsync(&store, event).await?;│
│                     } else {                     │
│                         // 2. text_delta → 累积到 buffer，不立刻写│
│                         delta_buffer.push(event); │
│                         if delta_buffer.should_flush() {│
│                             flush_buffer(&mut delta_buffer, &store).await?;│
│                         }                        │
│                     }                            │
│                     if let Some(tx) = ack { let _ = tx.send(Ok(())); }│
│                 }                                │
│                 Some(PersistCommand::Flush { ack }) => {│
│                     flush_buffer(&mut delta_buffer, &store).await?;│
│                     let _ = ack.send(Ok(()));    │
│                 }                                │
│                 None => break, // actor 关了      │
│             },                                   │
│             _ = sleep(Duration::from_millis(200)) => {│
│                 // 3. 时间窗到了也 flush         │
│                 flush_buffer(&mut delta_buffer, &store).await?;│
│             }                                    │
│         }                                        │
│     }                                            │
│     flush_buffer(&mut delta_buffer, &store).await?; // 最后 flush│
│ }                                                │
└──────────────────────────────────────────────────┘
```

### text-delta 批合的精确规则

| 触发条件 | 行为 | 出处 |
|---|---|---|
| 累积时间 ≥ 200ms | flush 整个 buffer 为一条 `AssistantMessageAppended { content: 合并后的 Vec<ContentBlock> }` | AGENTS.md §"Inviolable design principles" #2 + H02 文档 |
| 累积字符 ≥ 500 chars | 同上 | 同上 |
| **下一个非 delta 事件到来**（任何状态转移事件） | 强制 flush，再写新事件 | 本 spec 新增（之前文档没说清） |
| **Flush 命令显式触发** | flush，然后 ack | shutdown / `cancel_turn` / 测试场景 |
| store writer subtask 退出 | 最后一次 flush（best-effort，写失败就丢） | 同上 |

**"非 delta 事件强制 flush" 必须性的反例：**

```
没有这条规则的反例：
  T+0:    delta "Hello "  → buffer
  T+10ms: delta "world"   → buffer
  T+50ms: TurnEntered { ToolDispatching } 事件
          → 直接 append，但 buffer 里 "Hello world" 还没写
  T+50ms: store 里事件顺序变成 [...历史, TurnEntered]
          → 用户看到的 stream 是 "Hello world" 之后才 dispatch
          → 但 store 重放时 dispatch 先出现，之后 "Hello world"
          → store 顺序与时间顺序矛盾，FSM 重建出错
```

flush-before-non-delta 保证 **store 里事件的逻辑顺序与发出顺序严格一致**。

### fsync 策略

| 事件类型 | fsync 时机 |
|---|---|
| 状态转移（TurnEntered, ToolCallRequested, ToolCallCompleted, TurnPaused, TurnCompleted, ...） | **每个事件一次 fsync** |
| AssistantMessageAppended（text-delta 合批后） | **每个 batch 一次 fsync**（不是 per-delta） |
| StreamEvent | **不进 store，不 fsync** |

**fsync 成本估算**（用于 Sprint 1 SLO 验证）：

- 典型 NVMe SSD：fsync ~1–3ms
- 典型 EBS gp3：fsync ~5–10ms
- 典型本地 HDD：fsync ~20–50ms

ADR-0005 SLO 目标 P99 < 5ms → 假设 NVMe 才能稳过；EBS 场景需要 group commit 优化或者放宽 SLO 到 P99 < 15ms。

### 错误处理：store 写失败怎么办

**inviolable**：store 写失败 = ADR-0002 单一真相源失效，不能"继续 turn"。所有 store 写失败统一走：

```
append fails (IO error / disk full / corrupt)
  │
  ├─ ack 回 Err(StoreWriteError) → caller 检测到 →
  │
  ├─ TurnDriver: TurnDriver 提前终结返回 TurnOutcome::Failed { reason: StoreUnavailable }
  ├─ actor 主循环写状态事件失败: actor catch 错误 → 写 SessionDegraded（best-effort）→ panic
  │   → catch_unwind 兜底 → actor task 结束 → Runtime 摘掉这个 session
  │
  └─ 总体：单 session degrade 到 Failed，其它 session 不受影响
```

caller 通过 SessionHandle 看到的：next send → `Err(SessionClosed)` ；events_out broadcast 看到 `SessionFailed { reason }` 终态事件。

### Sprint 1 SLO benchmark 计划

```
crates/cogito-store/benches/append_throughput.rs:
  - bench_1: per-event fsync, 单 session, 10K events 顺序写
  - bench_2: 同上但 group commit（同一 turn 内事件攒到 turn 结束一次 fsync）
  - bench_3: 多 session 并发（10/100/1000 个 session 同时写各自的 jsonl）

输出：P50/P95/P99/P999 写入延迟矩阵；锁定后写入 ADR-0005 §3 表
```

### tokio::fs vs spawn_blocking 的选择

`tokio::fs` 内部其实**就是** `spawn_blocking` 的包装（tokio runtime 没有真正异步的 fs syscall），区别只在 API 形态。我们选显式 `spawn_blocking` 是因为：

1. **可控合批**：自己写的 writer 子任务可以一次 `spawn_blocking` 里做 N 个 event 的 fsync，省 task 调度开销；`tokio::fs` 做不到这个优化
2. **依赖更少**：只用 `std::fs`，不强绑 `tokio` 的 fs feature
3. **跟未来 Postgres 后端（v0.4）形态一致**：那里也是 sqlx 内部走专用 driver task，跟我们的 writer 子任务概念对得上

---

## §9 · Cancellation & error propagation

### CancellationToken 完整传播链

```
caller
  │ handle.cancel_turn()
  ▼
SessionHandle
  │ self.shared.current_cancel_token.cancel()
  ▼
CancellationToken（actor 持有，每 turn 重建）
  │
  ├──▶ TurnDriver task：每个 .await 点 select! 在 token 上
  │       │
  │       ├──▶ H06 stream demux：select!(stream.next(), token.cancelled())
  │       │       → 看到 cancelled → drop stream → 模型 SSE 连接断
  │       │
  │       ├──▶ H08 tool dispatch：select!(provider.invoke(...), token.cancelled())
  │       │       → cancelled → 调用方拿到 Cancelled；
  │       │         **已经 in-flight 的 tool future 不 abort**（cooperative）
  │       │       → tool 实现自己也 select 在 token 上的话，自己优雅退出
  │       │         否则 cogito 只是不收它的结果（drop future on next yield）
  │       │
  │       └──▶ H09 hook pipeline：每个 hook 实现可以 select on token
  │
  └──▶ ExecCtx.cancel: CancellationToken（exposed to tool implementations）
          │
          ▼
       tool 实现自己决定 cooperative 程度（文档约定）
```

### actor 处理 InternalCancel 命令

```rust
SessionCommand::InternalCancel { ack } => {
    match &self.in_flight {
        Some(InFlight::PausedOnJob { job_id, .. }) => {
            self.jobs.cancel(*job_id).await.ok();
            self.persist_with_fsync(Event::TurnCancelled { ... }).await?;
            self.in_flight = None;
            self.broadcast(StreamEvent::TurnCancelled);
        }
        Some(InFlight::Active(_)) => {
            // TurnDriver 会从 token 看到 cancel，自己结束
            // 我们只 ack，不做事
        }
        None => { /* idle, no-op */ }
    }
    let _ = ack.send(Ok(()));
}
```

### Panic 捕获边界（三层嵌套）

```
┌──────────────────────────────────────────────────────────────┐
│ Layer 1 · SessionActor task entry  (catch_unwind)            │
│                                                              │
│   actor panic（极少；通常是 actor 内部 bug）                  │
│   → 这个 session 死，其它 session 不受影响                    │
│   → Runtime 把这个 session 从 sessions DashMap 摘掉           │
│   → 不重启（caller 显式 re-open 才会有新 actor）              │
│                                                              │
│   ┌────────────────────────────────────────────────────────┐ │
│   │ Layer 2 · TurnDriver task entry  (catch_unwind)        │ │
│   │                                                        │ │
│   │   TurnDriver panic（model gateway / hook 实现 bug）    │ │
│   │   → 当前 turn Failed，actor 继续活，可接受新 input     │ │
│   │   → 写 TurnFailed { reason: TurnPanicked, location }   │ │
│   │                                                        │ │
│   │   ┌──────────────────────────────────────────────────┐ │ │
│   │   │ Layer 3 · tool invoke (per-call, catch_unwind)   │ │ │
│   │   │                                                  │ │ │
│   │   │   tool 实现 panic                                │ │ │
│   │   │   → 单 tool call Failed，turn 继续，其它 tool    │ │ │
│   │   │     call 不受影响                                │ │ │
│   │   │   → ToolResult::Error { kind: ToolPanicked }     │ │ │
│   │   └──────────────────────────────────────────────────┘ │ │
│   └────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

**为什么三层而不是只在最外层：**

- 只 Layer 1 → tool panic 把整个 session 干掉。但 tool 是第三方代码（MCP、consumer-supplied），不可信
- 加 Layer 2 → TurnDriver 自身 panic 不掀掉 session。允许 caller 直接重试新 input
- 加 Layer 3 → 多 tool call 时一个 panic 不影响其它

**catch_unwind 在 async fn 里的具体形态**（标准模式）：

```rust
use futures::FutureExt;
use std::panic::AssertUnwindSafe;

let result = AssertUnwindSafe(my_async_fn()).catch_unwind().await;
match result {
    Ok(Ok(value)) => /* 正常成功 */,
    Ok(Err(e)) => /* 业务错误 */,
    Err(panic) => /* panic 捕获 */,
}
```

### 结构化错误策略（落到 type 上）

```rust
// cogito-protocol
pub enum ToolResult {
    Output(Vec<ContentBlock>),
    Error { kind: ToolErrorKind, message: String, retryable: bool },
}

pub enum ToolErrorKind {
    InvalidArgs,       // H07 schema 失败
    InvocationFailed,  // tool 内部业务错误
    ToolPanicked,      // Layer 3 catch 到的
    Cancelled,         // cancellation token fired
    Timeout,           // tool 自己超时（不是 turn 超时）
    JobStateLost,      // resume 时 JobManager 查不到 job
    AsyncFailed,       // async job 完成但内部失败
}

pub enum TurnOutcome {
    Completed,
    Paused { job_id: JobId },
    Cancelled,
    Failed {
        reason: TurnFailureReason,
        recorded_event_id: EventId, // 末尾事件 ID，便于诊断
    },
}

pub enum TurnFailureReason {
    StoreUnavailable,        // store 写失败
    ModelGatewayFailed(String), // gateway 返回 Err
    TurnPanicked { location: &'static str },
    TurnTimedOut,            // turn 超出 budget
    HookRejected { hook_name: String, message: String },
}
```

**铁律：**

1. **`ToolResult::Error` 永远不向上 propagate**：H08 把它当正常结果记录，喂回模型让模型自己决定怎么办
2. **`HookDecision::Reject` 不是 error**：是正常控制流，写 `TurnRejected` 事件
3. **`TurnOutcome::Failed` 是 turn 终态**：actor 收到后写终态事件，回到 idle 接受新 input
4. **只有 store write 失败导致 session 整体不可用**：actor 自己挂掉，Runtime 摘 session（caller 看到 SessionClosed）

---

## §10 · Reference comparison

集中对比表 —— 把分散在 §3/§5/§6/§7/§8/§9 的 Codex / Claude Code 对比 + SaaS 平台调研汇总成单一引用表。

### 主对比表

| 维度 | Codex Rust | Claude Code | OpenAI Assistants | LangGraph Platform | Inngest AgentKit | Lindy (Temporal) | Manus | cogito v0.1 | cogito v0.4 计划 |
|---|---|---|---|---|---|---|---|---|---|
| **进程模型** | single CLI | single CLI | server-side opaque | distributed workers | distributed workers | Temporal workers | dedicated sandbox/session | per-session actor (1 process) | per-session actor (N workers) |
| **session-worker 绑定** | N/A | N/A | opaque | **any worker** (Redis queue) | **any worker** | **any worker** | **sticky** (sandbox = session) | N/A | **any worker** (rehydrate) |
| **状态权威源** | rollout JSONL | conversation | OpenAI 内部 | Postgres checkpoints | Inngest backend (memoized steps) | Temporal event history | sandbox FS + memory layer | JSONL event log | Postgres event log |
| **async task → agent 通知** | 同进程 await | inject conversation | **客户端轮询** `/runs/{id}` | **Redis Pub/Sub** by session | **`waitForEvent`** + event bus | **Temporal Signal** | recursive callback (loop monitors) | mpsc + `SessionCommand::JobCompleted` | Redis Stream → 同 trait method |
| **崩溃恢复** | 重启 CLI | 重开 conversation | 不可见（managed） | 从 Postgres checkpoint resume | step memoization + retry | event history replay | sandbox replace + 上次 checkpoint resume | actor rebuild + H03 replay + JobManager.status | 同 v0.1，跨 worker |
| **distributed locking** | N/A | N/A | 内部有 | **没有**（Diagrid 批评点） | event-key 级有 | workflow-id 级有 | sandbox 物理隔离 | N/A | session-id sticky on `BLPOP`（v0.4 设计），store fsync + 单调事件 ID 检测 fork |

### 关键概念全维度对比（前 6 维上的扩展）

| 维度 | Codex Rust | Claude Code | cogito v0.1 | cogito 选择理由 |
|---|---|---|---|---|
| **Session 模型** | `Arc<Session>` + `Mutex<ActiveTurn>` (`core/src/codex.rs:440`) | "agentic loop"（单进程） | per-session actor + mailbox | ≥1000 并发 session + budget 隔离；ADR-0005 §3 |
| **tokio runtime** | 隐式 `#[tokio::main]` multi-thread | N/A（非 Rust） | caller 注入 `Handle`，默认 `Handle::current()` | 嵌入式 lib 约定 |
| **主循环 / FSM** | function chain `run_turn` loop (`core/src/codex.rs:2960`) | function chain | 显式 enum FSM（ADR-0003） | resumability 是 v0.1 北极星 |
| **事件流** | 单 `EventMsg` enum + 两 stream（JSON-RPC + rollout） | inject 进 conversation | 双类型双 stream：`ConversationEvent`（持久）+ `StreamEvent`（实时 broadcast） | broadcast 支持 multi-subscriber |
| **events out channel** | JSON-RPC stdout，single subscriber | conversation injection | `broadcast::channel(256)` | multi-subscriber + 慢 subscriber lagged 语义 |
| **persist channel** | `Sender<RolloutCmd>` mpsc 256 (`rollout/recorder.rs:244`) | N/A | `mpsc::channel(256)` | 容量直接照搬 |
| **store 落盘** | `tokio::fs` 异步 + flush 按需，**无 fsync** | N/A | `spawn_blocking` + per-event fsync | event sourcing 单一真相源 |
| **text-delta 批合** | unit-by-unit per `ResponseItem` 立即 record (`stream_events_utils.rs:56`) | inject 整段（不暴露） | persist 侧 200ms / 500 chars 合批；stream 侧逐 chunk | AGENTS.md §inviolable #2 + UI 流畅 |
| **同步/异步判定** | 不存在（所有同步） | LLM 通过 `run_in_background` 参数 (3) | `ToolDescriptor.execution_class` (1)+(2) | runtime 责任，不污染 prompt；跨 model 兼容 |
| **长任务模型** | tokio task 在 turn 内 await | `<task-notification>` inject | `InvokeOutcome::Async(JobId)` + actor PausedOnJob | Sprint 4 长任务需要跨 turn / 跨进程 |
| **唤醒机制** | N/A | inject 到 conversation | `JobManager` callback → mpsc → `SessionCommand::JobCompleted` 入 mailbox | 不轮询 + 不打断当前 turn |
| **mid-turn 不打断** | tools 在 turn 内同步 await | "fires between turns" | mailbox FIFO 排队，turn 间处理 | 直接参照 Claude Code |
| **cancellation 入口** | `Op::Interrupt` 走 mailbox | `TaskStop` tool | `handle.cancel_turn()` → 独立 `CancellationToken` | 避免与 input 排队 |
| **cancellation grace** | 100ms 后强 abort (`tasks/mod.rs:235`) | 不公开 | cooperative（tool 自己 select on token） | tool 实现可控 + 避免 unsafe abort |
| **panic 隔离** | mutex poison（接受） | 不可见 | 三层 `catch_unwind`（actor / turn / tool） | ≥1000 session 不能因一个 tool 一起死 |
| **multi-tool 并发** | per-tool parallel/serial 闸（read/write lock，`tools/parallel.rs:80`） | LLM 一次发多个 | v0.1 sequential；strategy flag 可解锁 v0.2 | v0.1 收敛 |
| **shutdown** | 进程退出（CLI） | 不可见 | `handle.shutdown(timeout)` graceful + drain | 嵌入式 lib 必须 |
| **per-session budget** | 不需要 | 不可见 | turn 时间 cap（v0.1）+ memory/cost cap（v0.4） | ADR-0005 §3 |

### 三个"有意分叉" callout

**1. Session 模型：actor vs shared mutex**
- Codex 选 `Arc<Session> + Mutex` 是因为单用户 CLI，并发度低
- cogito 选 actor 是因为 multi-session + budget 隔离 + panic isolation 三个硬需求

**2. store 落盘：fsync vs no-fsync**
- Codex 接受丢最后几条事件（重启 CLI 体验 OK）
- cogito 不能丢任何事件（resume 不变量）

**3. 同步/异步判定：runtime-decided vs LLM-decided**
- Claude Code 把决策给 LLM 是其特定 CLI 体验决定
- cogito 在 runtime 决策，不让"执行模型"知识污染 prompt

### 三个"直接复用" callout

**1. EventMsg / RolloutItem 双类型分离** → cogito `StreamEvent` / `ConversationEvent` 同形态
**2. Codex persist channel 容量 256** → cogito 同
**3. Claude Code "fires between turns" 设计原则** → cogito mailbox FIFO + JobCompleted 不打断 turn 的直接落地

### SaaS 平台调研要点

| 平台 | 通知机制 | 状态存储 | 崩溃恢复 | session 绑定 | 主要参考 |
|---|---|---|---|---|---|
| **Manus** | 内部 task queue + recursive callback | sandbox FS + memory layer checkpoints | sandbox replace + 上次 checkpoint | **sticky**（sandbox = session 单位） | Manus Context Engineering blog；E2B 集成 |
| **OpenAI Assistants** | 轮询 `/runs/{id}` 或 SSE；Responses API 后台模式有 webhook | server-side OpenAI 管理 | 不可见 | opaque（按 ID 路由） | platform.openai.com docs |
| **LangGraph Platform** | Redis BLPOP queue + Redis Pub/Sub for streaming/cancel | Postgres checkpoints + Redis ephemeral | Postgres checkpoint resume；BLPOP 原子性 | **any worker** (`N_JOBS_PER_WORKER=10`) | LangChain Data Plane docs；neuralware blog |
| **Inngest AgentKit** | `step.waitForEvent(name, match)` 暂停 + 事件匹配 resume | Inngest backend（memoized steps） | re-execute from last step；memoized skip | **any worker** | inngest.com/docs |
| **Lindy AI** | Temporal Signals 用于外部回调 | Temporal event history | event history replay | **any worker** (Temporal workflow) | Lindy/Temporal case study |
| **AutoGen** | in-memory event bus（distributed 仍 experimental） | application-level（用 `save_state`） | 无内置；application 自管 | distributed 中"host" 路由 | microsoft.github.io/autogen |
| **CrewAI** | `@persist` Flow checkpoint | "a database"（默认 SQLite） | `kickoff(state_id=...)` | 公开文档未明 | docs.crewai.com |

### 业界两大派系

| 派系 | 代表 | 通知机制 | 状态存储 | cogito 接近度 |
|---|---|---|---|---|
| **stateful queue** | LangGraph、OpenAI Assistants | Redis BLPOP / 轮询 | Postgres + Redis 分工 | ★★★（v0.4 形态接近） |
| **durable execution** | Inngest、Lindy(Temporal)、Manus | event signal / waitForEvent | 框架自己的 event history | ★★★★（事件溯源理念一致） |
| **in-process special case** | Codex / Claude Code CLI | in-process channel | 文件 | ★★★★★（v0.1 同形态） |

**两个观察：**

- cogito 的 "Conversation Service = single source of truth" + actor 模型 = **durable execution 派的轻量化形态**。Q6.B 的 `JobManager::on_complete` callback 正是 Inngest `step.waitForEvent` 和 Temporal Signal 的 in-process 退化，trait 形状不变可以直接 SaaS 化
- "Sticky session" 是反主流。研究里**只有 Manus 是 sticky**；LangGraph / OpenAI / Inngest / Temporal 全是 "any worker can resume any session"。这跟 cogito 的"state in store, not in memory"原则**完全一致**

---

## §11 · Open TBDs

按"何时拍板"分组。

### Sprint 1 实施时根据测量拍

| ID | 主题 | 当前默认 | 拍板依据 |
|---|---|---|---|
| **TBD-C1** | channel 容量数字（mailbox=64, events_out=256, persist_tx=256, job_completion=32, oneshot 都 = 1） | 见 §7 表 | Sprint 1 benchmark：单 session 10K events 顺序写、N 并发 session 测 lagged 比率，根据 P99 调整 |
| **TBD-T1** | store 是否需要 group commit（同一 turn 内事件攒到 turn 结束一次 fsync） | 不做（per-event fsync） | Sprint 1 fsync benchmark 结果。如果 P99 > 5ms（ADR-0005 §3 SLO 目标）→ 启用 group commit；如果 P99 ≤ 5ms 达标 → 不启用，维持 per-event fsync |
| **TBD-T2** | turn-level timeout 默认值 | 暂定 5 min | 等 Sprint 2 跑通端到端后看真实长 turn 分布 |

### Sprint 1–6 实施过程中讨论

| ID | 主题 | 候选 | 影响范围 |
|---|---|---|---|
| **TBD-D1** | `ExecutionClass::Adaptive` 工具被 strategy 的 `allow_async_tools: false` 时如何过滤？整个 tool 不上 prompt，还是上 prompt 但 invoke 失败？ | 候选 a: H05 整个 tool 隐藏；候选 b: tool 可见但 H08 拒绝 Async 返回 | Sprint 5 H10 strategy 时定 |
| **TBD-R1** | resume 时如果 store 末尾事件 corrupt（半截 JSONL）H03 怎么处理 | 候选 a: 截断到上一条完整事件，写 `ResumeRecovered`；候选 b: 直接 `ResumeError` 让 caller 决定 | Sprint 3 H03 实施 |
| **TBD-H1** | H09 hook 是否支持 async（Claude Code `asyncRewake` 形态） | v0.1 纯 sync；async 形态 v0.2+ 加 | Sprint 6 H09 实施 |
| **TBD-E1** | `ConversationEvent::schema_version` 第一次 bump 的具体触发场景 | 等到首次 incompatible 变更时（v0.2 加 Image 是 additive，不 bump） | 自然事件 |
| **TBD-P1** | `RuntimeBuilder::shutdown_token` 字段 v0.1 留位但不消费 | 已定（不消费） | v0.4 配合 ADR-0010 一起做 |

### v0.4 SaaS-ready 才设计

| ID | 主题 | 备注 |
|---|---|---|
| **TBD-S1** | session rehydration 触发条件 | **收紧**：worker 从 Redis Stream 收到该 session 的 JobCompletion 时，本地 actor 表查不到就 rehydrate；具体 ADR-0009 写 |
| **TBD-S2** | JobManager 分布式后端 | **收紧**：Redis Stream + Postgres（前者放队列与 completion pub/sub，后者放 job 终态）；NATS 留作 consumer 自定义 |
| **TBD-S3** | broker → actor routing | **收紧**：Redis Stream consumer group by node + session-id 分片 + sticky-on-first-poll（LangGraph 形态） |
| **TBD-S4** | memory cap 强制点 | sandbox 内 cgroup vs 进程级 RLIMIT；v0.4 配合 ADR-0010 |
| **TBD-S5** | `TenantContext` 注入位置 | `ExecCtx` 还是 `Runtime` 级；v0.4 ADR-0012 |
| **TBD-S6** | per-session quota / 计费 metric 维度 | v0.4 + `MetricsRecorder` |
| **TBD-S7** | distributed locking 策略 | 利用 store fsync + 事件单调 ID 检测 fork，写 `SessionForked` 事件让其中一个 worker 退出 |

### v1.0 GA 前必须解决

| ID | 主题 | 备注 |
|---|---|---|
| **TBD-G1** | `SessionCommand` enum `#[non_exhaustive]` + sealed extension trait？ | 公 API 稳定性审计 |
| **TBD-G2** | `ToolErrorKind` / `TurnFailureReason` 是否封闭枚举 | 同上 |
| **TBD-G3** | `JobManager` trait 在 1.0 是否 sealed | 同上 |

---

## §12 · Testing strategy

### 测试矩阵（按本 spec 引入的新概念）

| 概念 | 单元 | 集成 | 契约 | 混沌 | 属性 | 基准 |
|---|---|---|---|---|---|---|
| Actor task lifecycle（start/run/shutdown/panic/drop） | ✅ | ✅ | — | ✅ | — | — |
| mailbox FIFO + select 主循环 | ✅ | — | — | — | ✅ | — |
| TurnDriver spawn/join 契约 | ✅ | ✅ | — | ✅ | — | — |
| H01 FSM 状态转移 | ✅ | ✅ | — | ✅ | ✅ | — |
| H02 store writer + text-delta 批合 | ✅ | ✅ | ✅ (`ConversationStore`) | — | ✅ | ✅ |
| `JobManager::on_complete` 回调 + resume | ✅ | ✅ | ✅ (`JobManager`) | ✅ | — | — |
| `CancellationToken` 三层传播 | ✅ | ✅ | — | ✅ | — | — |
| `catch_unwind` 三层隔离 | ✅ | ✅ | — | — | — | — |
| broadcast lagged 慢 subscriber | ✅ | — | — | — | — | — |
| Resume on open（replay） | ✅ | ✅ | — | ✅ | ✅ | — |
| 并发 N 个 session 互不污染 | — | ✅ | — | ✅ | — | ✅ |

### 文件布局

```
crates/cogito-core/
├── src/
│   ├── runtime/
│   │   ├── actor.rs            # SessionActor 主循环
│   │   ├── handle.rs           # SessionHandle API
│   │   ├── builder.rs          # RuntimeBuilder
│   │   └── store_writer.rs     # H02 consumer 侧
│   └── harness/
│       ├── turn_driver.rs      # H01 FSM
│       └── (H03-H10 各自子模块)
└── tests/
    ├── actor_lifecycle.rs      # open/shutdown/drop/panic
    ├── mailbox_fifo.rs         # FIFO 不变量 property test
    ├── turn_driver_fsm.rs      # 状态转移单元 + 集成
    ├── job_resume.rs           # JobManager callback + crash recovery
    ├── cancellation.rs         # 三层 token 传播 + cancel-while-paused
    ├── concurrent_sessions.rs  # N 并发 session 隔离
    ├── resume_chaos.rs         # 在每个 state 转移点注入崩溃
    └── store_writer_batching.rs # text-delta 200ms/500-char 边界

crates/cogito-store/
├── src/lib.rs
├── benches/
│   └── append_throughput.rs    # SLO benchmark（Sprint 1 拍 TBD-T1）
└── tests/
    └── contract.rs             # 共享契约测试（含 in-memory + jsonl）

crates/cogito-protocol/
└── tests/
    ├── stream_event_codec.rs   # StreamEvent serde 稳定性
    └── execution_class.rs      # ExecutionClass 行为契约

testing/cogito-test-fixtures/
├── mock_job_manager.rs         # 可脚本化完成时机的 JobManager 假实现
├── mock_tool_provider.rs       # 可脚本化返回 Sync/Async 的 ToolProvider
└── chaos.rs                    # 崩溃注入工具
```

### 关键测试细则

**1. mailbox FIFO property test**：用 proptest 生成任意 `Vec<SessionCommand>` 序列发给 actor，断言**store 写入顺序与发送顺序一致**（除 `InternalCancel` 外，它走独立路径）。

**2. cancel-while-paused 集成测试**：

```
1. spawn mock async tool 返回 Async(job_id)
2. mock JobManager 不主动完成（等被 cancel）
3. handle.cancel_turn()
4. 断言：
   - store 末尾事件 = TurnCancelled
   - mock JobManager.cancel(job_id) 被调用过 1 次
   - actor in_flight = None
   - 后续 handle.send(Input) 正常工作
```

**3. 三层 panic 隔离测试**：

- Layer 3：mock tool 在 invoke 里 panic → 单 tool call 变 ToolResult::Error；其它 tool call 继续；turn 完成
- Layer 2：mock model gateway 在 stream 里 panic → 当前 turn 变 Failed { TurnPanicked }；actor 仍然能接受新 Input
- Layer 1：actor 内部 unwrap 触发 panic（人造）→ actor task 死；Runtime.sessions.get(sid) 返回 None；其它 session 不受影响

**4. resume_chaos 扩展**（已有文件，本 spec 新增覆盖点）：

| 注入点 | 期望 |
|---|---|
| 每个 FSM 状态转移事件写完前后 | resume 后到达语义等价状态 |
| `JobManager::on_complete` 注册前后 | resume 后能重新拿到 job 完成 |
| store writer 子任务正在 flush 时 actor 死掉 | 已 ack 的事件保证在 store；未 ack 的允许丢 |
| PausedOnJob 状态下 JobManager 状态丢失 | 写 `TurnFailed { JobStateLost }`，不挂 actor |

**5. 并发隔离测试**：

```
spawn 100 个 session，每个独立跑 mock turn；
其中 5 个 mock tool 故意 panic；
断言：
- 95 个 session 完成 turn 正常
- 5 个 session turn 变 Failed，actor 仍活
- 内存峰值 < 100 * 1 MiB（ADR-0005 §3 budget）
```

**6. Sprint 1 SLO benchmark 输出**：

```
target P99 step record write < 5 ms（ADR-0005 §3）

bench_append_throughput 输出格式：
| scenario | P50 | P95 | P99 | P999 | throughput |
| per-event fsync, 1 session    | x | x | x | x | x ev/s |
| group commit, 1 session       | x | x | x | x | x ev/s |
| per-event fsync, 100 sessions | x | x | x | x | x ev/s |
| group commit, 100 sessions    | x | x | x | x | x ev/s |

→ 写入 docs/quality/v0.1-slo-results.md 并 commit
→ 更新 ADR-0005 §3 表里的 "provisional" 标记 → "locked"
```

---

## §13 · Migration & Sprint 0 work order

本 spec ratify 后 Sprint 0 收尾的具体动作，按依赖顺序。

### 阶段 1 · workspace 拓扑修正（先做，无依赖）

| 步骤 | 动作 | 验收 |
|---|---|---|
| 1.1 | 删除 `crates/cogito-conversation/`（整个目录） | `cargo check` 失败处可见 |
| 1.2 | 新建 `crates/cogito-store/{Cargo.toml,src/lib.rs}` 骨架 | `cargo check -p cogito-store` 通过 |
| 1.3 | 更新 workspace `Cargo.toml`：members 列表删 `cogito-conversation`，加 `cogito-store`；`[workspace.dependencies]` 同步 | `cargo check --workspace` 通过 |
| 1.4 | 修正 `crates/cogito-core/Cargo.toml`：**删除** 直接依赖 `cogito-conversation`、`cogito-model`、`cogito-tools`、`cogito-sandbox`、`cogito-jobs`；只保留 `cogito-protocol` | `cargo check -p cogito-core` 通过；layer 不变量首次由 Cargo 强制 |
| 1.5 | 修正 `crates/cogito-jobs/Cargo.toml`：删除 `cogito-conversation`（已不存在）；保留 `cogito-protocol` | `cargo check -p cogito-jobs` 通过 |
| 1.6 | 修正 `crates/cogito-cli/Cargo.toml`：依赖 `cogito-core`（含 runtime） + 所有 Hands/Boundary/Session crate（Surface 层可以全引） | `cargo check -p cogito-cli` 通过 |

### 阶段 2 · protocol 新增类型（阶段 1 完成后）

| 步骤 | 动作 | 文件 |
|---|---|---|
| 2.1 | 增加 `ExecutionClass` enum + `ToolDescriptor.execution_class` 字段 | `crates/cogito-protocol/src/tool.rs` |
| 2.2 | 增加 `StreamEvent` enum（v0.1 variants：`TurnStarted` / `TurnPaused` / `TurnResumed` / `TurnCancelled` / `TurnCompleted` / `TurnFailed` / `TextDelta` / `ToolDispatchStarted` / `ToolDispatchEnded`） | `crates/cogito-protocol/src/stream.rs` |
| 2.3 | 增加 `JobCompletionEvent` 类型 + `JobManager::on_complete` trait method（default impl panic 让旧实现编译失败 → 强制实现） | `crates/cogito-protocol/src/job.rs` |
| 2.4 | 增加 `ToolErrorKind` 新 variant：`Cancelled` / `ToolPanicked` / `JobStateLost` / `AsyncFailed` | `crates/cogito-protocol/src/tool.rs` |
| 2.5 | 增加 `TurnOutcome` / `TurnFailureReason` 完整定义（之前只在文档里） | `crates/cogito-protocol/src/turn.rs` |

### 阶段 3 · runtime 骨架（阶段 2 完成后）

| 步骤 | 动作 | 文件 |
|---|---|---|
| 3.1 | 创建 `cogito-core/src/runtime/mod.rs` 含 `Runtime` / `RuntimeBuilder` / `SessionHandle` stub | 单元测试占位 |
| 3.2 | `SessionCommand` enum 定义 + `mailbox`/`events_out`/`persist_tx`/`job_completion` channel 类型 alias | `runtime/types.rs` |
| 3.3 | `SessionActor` struct 定义 + `actor_main` 函数 stub（loop body 用 `todo!()`） | `runtime/actor.rs` |
| 3.4 | `store_writer` subtask stub | `runtime/store_writer.rs` |

### 阶段 4 · CI（与阶段 1-3 并行可做）

| 步骤 | 动作 | 验收 |
|---|---|---|
| 4.1 | `.github/workflows/ci.yml` 跑 `just ci`（fmt + clippy + test） | PR 触发 CI 绿 |
| 4.2 | 加 `cargo deny check` step（ADR-0005 §4.5 security） | CI 含此 step |
| 4.3 | 加 ADR-0004 layer 检查（grep 失败：`cogito-core/src/harness` 任何文件含 `use cogito_(tools|model|sandbox|jobs|store)`） | 脚本在 `scripts/check-layer.sh` |

### Sprint 1 入场条件（本 spec 影响的部分）

Sprint 1 不能开始的硬阻塞：

- ✅ 阶段 1.1–1.6 全部完成（workspace 拓扑合规）
- ✅ 阶段 2 全部完成（protocol 新类型 lands，Sprint 1 H02 实施可用）
- ✅ 阶段 3.1 完成（runtime 模块存在，Sprint 1 的 H02 store writer 可以放进 `runtime/store_writer.rs`）
- ✅ 本 spec ratify + `writing-plans` skill 产出 Sprint 1 详细实施计划
- ✅ CI 绿

### 文档同步动作（spec ratify 后立即做）

| 文件 | 更新内容 |
|---|---|
| `ARCHITECTURE.md` §"Workspace layout" | `cogito-conversation` 已是历史，删干净；`cogito-store` 加进；`cogito-core` 行说明 `harness/` + `runtime/` 子模块分工 |
| `ARCHITECTURE.md` §"Trait contracts" | `JobManager` 增加 `on_complete` 行；新增 `ExecutionClass`、`StreamEvent` 行 |
| `ROADMAP.md` Sprint 0 复选框 | 把"13 crates created" 调成新 list，勾完；勾"CI workflow runs just ci" |
| `docs/components/H01-turn-driver.md` | 加一段"Implementation note: runs as a per-turn tokio task spawned by SessionActor; FSM is `enum TurnState` with each variant carrying the data its transition needs" |
| `docs/components/H02-step-recorder.md` | 加一段"Implementation note: in v0.1 H02 is the `persist_tx` producer side + the `store_writer` subtask consumer side; see Runtime spec §8 for batching/flush rules" |
| `docs/components/H08-tool-dispatcher.md` | 加一段"Implementation note: dispatcher branches on `ToolDescriptor.execution_class` + `InvokeOutcome` variant; contract violations are debug_assert in dev, structured `ToolResult::Error` in release" |
| 新增 `docs/adr/0006-runtime-h01-execution-model.md` | 把本 spec 的 load-bearing 决策（Q1-Q6 + 跟 Codex/Claude Code 的有意分叉）压缩成正式 ADR；spec 文件留在 `docs/superpowers/specs/` 作为详细参考；ADR 引用 spec |

---

## References

### cogito 内部文档

- `AGENTS.md` — operating manual + inviolable design principles
- `ARCHITECTURE.md` — 10-component design + workspace layout + version evolution
- `ROADMAP.md` — Sprint plan
- `docs/adr/0001-rust-workspace-layout.md`
- `docs/adr/0002-event-sourcing-conversation.md`
- `docs/adr/0003-state-machine-turn-driver.md`
- `docs/adr/0004-brain-hands-session-boundaries.md`
- `docs/adr/0005-production-scope-and-quality-gates.md`
- `docs/components/H01-turn-driver.md` 至 `H10-strategy-selector.md`

### Codex Rust 源码（本地：`/home/SENSETIME/qiannengsheng/whoami/workspaces/agents/codex/codex-rs/`）

- `core/src/codex.rs:440` — `Session` struct（`Arc + Mutex<ActiveTurn>` 模式）
- `core/src/codex.rs:2960` — `run_turn` function chain（非 FSM）
- `core/src/state/service.rs:18-32` — `SessionServices` 持有 `Sender<RolloutCmd>`
- `core/src/rollout/recorder.rs:244` — persist channel mpsc 256
- `core/src/rollout/recorder.rs:249` — `tokio::fs` + 无 fsync 写入
- `core/src/stream_events_utils.rs:43-90` — text-delta unit-by-unit 立即 record
- `core/src/agent/control.rs:96` — `interrupt_agent` 通过 `Op::Interrupt`
- `core/src/tasks/mod.rs:122,170-248` — `CancellationToken` 100ms grace
- `core/src/tools/parallel.rs:49-105,80-84` — 并行/顺序 tool 闸

### Claude Code 官方文档

- [How Claude Code works](https://code.claude.com/docs/en/how-claude-code-works.md) — "agentic loop" 术语
- [Tools reference](https://code.claude.com/docs/en/tools.md) — `Bash` `run_in_background` 行为
- [Create custom subagents](https://code.claude.com/docs/en/sub-agents.md) — 后台 subagent 行为
- [Run prompts on a schedule](https://code.claude.com/docs/en/scheduled-tasks.md) — "fires between your turns" 原则
- [Claude Code Hooks Reference](https://code.claude.com/docs/en/hooks-guide.md) — 同步/async hook 模式

### SaaS Agent Platform 调研

- [Manus Context Engineering blog](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus)
- [Manus Sandbox](https://manus.im/blog/manus-sandbox)
- [E2B: How Manus uses E2B](https://e2b.dev/blog/how-manus-uses-e2b-to-provide-agents-with-virtual-computers)
- [OpenAI Assistants runs reference](https://platform.openai.com/docs/api-reference/runs)
- [OpenAI Background mode](https://platform.openai.com/docs/guides/background)
- [neuralware: How LangGraph uses Redis](https://neuralware.github.io/posts/langgraph-redis/)
- [LangChain Data Plane docs](https://docs.langchain.com/langgraph-platform/data-plane)
- [Diagrid critique on checkpoints vs durable execution](https://www.diagrid.io/blog/checkpoints-are-not-durable-execution-why-langgraph-crewai-google-adk-and-others-fall-short-for-production-agent-workflows)
- [Inngest agent tool loops](https://www.inngest.com/docs/ai-patterns/agent-tool-loops)
- [Inngest waitForEvent reference](https://www.inngest.com/docs/reference/functions/step-wait-for-event)
- [Temporal/Lindy case study](https://temporal.io/resources/case-studies/lindy-reliability-observability-ai-agents-temporal-cloud)
- [CrewAI Production Architecture](https://docs.crewai.com/en/concepts/production-architecture)
- [Microsoft Agent Framework overview](https://learn.microsoft.com/en-us/agent-framework/overview/)

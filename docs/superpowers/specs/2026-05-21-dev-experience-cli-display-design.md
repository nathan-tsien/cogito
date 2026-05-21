# Dev Experience — Debug Log Compaction + CLI 角色着色 — 设计 Spec

> **Status**: Accepted (2026-05-21)
> **Track**: 跨 sprint 的 dev-experience polish(不属于任何编号 sprint;Sprint 4 MCP 工作的"边角料"清扫)
> **Authors**: qiannengsheng + AI brainstorm partner
>
> 本文件记录 GitLab Issue #2 "Cogito Debug Info 优化" + `cogito chat`
> REPL 显示优化的**决策轨迹**与**实施分解**。范围**故意保持极小** ——
> 不引入新 crate、不改 protocol、不引入 TUI(留给 Sprint 7)。
>
> 上游来源:
> - [GitLab Issue #2](https://gitlab.sz.sensetime.com/compass/cogito/-/issues/2):
>   开 debug 后 ModelGateway↔LLM endpoint 的 JSON 太长不易看。
> - 对话中追加的需求:`cogito chat` REPL 当前无法区分 user / agent / tool
>   消息。

---

## 1 · 目标

让"开发者跑 `cogito chat` + `RUST_LOG=debug`"这条最常用的本地调试
路径,在视觉上从"一坨流水"变成"角色清晰、信息可定位",而**不**引入
任何新的运行时抽象(no TUI、no 新 crate、no 配置项、no 协议变更、
no debug 文件落盘)。

### 1.1 In-scope

1. **Model-side**:`cogito-model` 两个 adapter(OpenAI-compat + Anthropic)
   的 DEBUG 日志统一为"一行 compact JSON",取消 pretty-print 和 ASCII
   分隔框。Anthropic adapter 补齐目前缺失的等价日志。
2. **CLI-side**:`cogito-cli` 在 `chat.rs` 旁新增一个小模块 `render.rs`,
   负责把 `StreamEvent` 翻译成 ANSI 着色 + 角色前缀的 stdout 输出;
   `IsTerminal` 检测 + 非 TTY 自动降级为纯文本。

### 1.2 Out-of-scope(故意不做)

| 不做的事 | 为什么不做 | 何时做 |
| --- | --- | --- |
| TUI / ratatui | 已经在 Sprint 7 路线里 | Sprint 7 |
| Debug 内容落盘到文件(`$XDG_CACHE_HOME/cogito/prompts.log` 之类) | 用户在 brainstorm 中明确否决 ——"先不搞复杂" | 真有需求再起 ADR |
| `--debug` CLI flag / `COGITO_PROMPT_LOG` env-var | 同上;DEBUG 走 `RUST_LOG` 既有约定 | — |
| Tool call args / result 在终端预览 | 数据已经持久化在 JSONL,`cat` 即可;同时需要扩 `StreamEvent` 字段 / 订阅第二条流,违反"无协议变更" | 真需要时再扩 `StreamEvent` |
| 响应体(SSE 事件 / 完整 response JSON)的 debug 日志 | 本次只解决"prompt 组装看不清"——这是 Issue #2 的原文 | 后续 |
| Multi-line input / 行编辑 / 历史 | 与本次显示问题无关 | — |
| 修改 tracing-subscriber 全局配置(target 显示、过滤器结构) | 已经在 `main.rs` 里,跟 REPL 渲染解耦 | — |

---

## 2 · Model-side(Issue #2 修复)

### 2.1 现状

- `crates/cogito-model/src/openai_compat/mod.rs:109-118`:
  ```rust
  if tracing::enabled!(tracing::Level::DEBUG) {
      match serde_json::to_string_pretty(&body) {
          Ok(json) => tracing::debug!(target: "cogito::prompt", url = %url,
                                       "\n── request body ──\n{json}\n──────────────────"),
          Err(e)   => tracing::debug!(target: "cogito::prompt",
                                       "request body serialization failed: {e}"),
      }
  }
  ```
  问题:`to_string_pretty` 把消息体拍成几十~几百行,ASCII 边框 + 空行
  使一次请求轻松占满终端 scrollback;开发者实际只想看"组装出来的
  prompt 长什么样"。
- `crates/cogito-model/src/anthropic/mod.rs`:**完全没有** 等价日志 ——
  在 Anthropic 端调试只能 `tcpdump`,这是缺陷不是设计。

### 2.2 目标输出

每次发请求前打一行 `tracing::debug!`,内容形如:

```
DEBUG cogito::prompt: request: {"model":"claude-opus-4-7","messages":[...]} url=https://api.anthropic.com/v1/messages
```

终端宽度不够时由终端 wrap;tracing 不再为我们额外加空行 / 分隔框。

### 2.3 变更

**`crates/cogito-model/src/openai_compat/mod.rs`(替换 109-118):**

```rust
if tracing::enabled!(tracing::Level::DEBUG) {
    match serde_json::to_string(&body) {
        Ok(json) => tracing::debug!(target: "cogito::prompt", url = %url,
                                     "request: {json}"),
        Err(e)   => tracing::debug!(target: "cogito::prompt",
                                     "request body serialization failed: {e}"),
    }
}
```

**`crates/cogito-model/src/anthropic/mod.rs`(在等价位置 —— 序列化完
请求体、`builder.send()` 之前,插入与上面字节一致的 4 行):**

```rust
if tracing::enabled!(tracing::Level::DEBUG) {
    match serde_json::to_string(&body) {
        Ok(json) => tracing::debug!(target: "cogito::prompt", url = %url,
                                     "request: {json}"),
        Err(e)   => tracing::debug!(target: "cogito::prompt",
                                     "request body serialization failed: {e}"),
    }
}
```

### 2.4 不写测试

DEBUG 级 `tracing` 副作用日志,无业务语义、无失败模式。`make ci` 已经
覆盖编译期检查。

---

## 3 · CLI-side(REPL 角色着色)

### 3.1 现状

`crates/cogito-cli/src/chat.rs:195-228` 的事件循环目前**只**消费
`StreamEvent::TextDelta`,直接 `print!("{chunk}")` 不加任何前缀:

- 用户输入的回车 / 模型回答的开头 / 工具调用 / 一次 turn 的结束,
  在屏幕上没有视觉边界。
- `TurnStarted` / `TurnCompleted` / `TurnFailed` / `TurnCancelled` /
  `ToolDispatchStarted` / `ToolDispatchEnded` 全部被 `Ok(_) => {}` 吞掉。

### 3.2 目标输出(示例)

```
> 帮我读一下 src/main.rs
agent: 好的,我看一下这个文件。
[tool] read_file …
[tool] read_file ok (12ms)
agent: 这是一个 tokio main,入口函数把 CLI 解析后分发到子命令。
>
```

- `> ` 用 cyan 着色,作为 user input 的提示符。
- `agent: ` 用 green;同一次 turn 内,只在"刚从工具回来 / 刚开始新文本块"
  时打印一次,之后的 delta 拼接到当前行尾。
- `[tool] <name> …` / `[tool] <name> ok|err (Nms)` 用 dim yellow;失败
  时 `err` 红色。
- `[paused]` / `[resumed]` / `[cancelled]` / `[error] <reason>` 在
  相应的 turn 生命周期事件出现时单独成行。

### 3.3 模块结构

新建 `crates/cogito-cli/src/render.rs`:

```rust
//! ANSI-colored REPL renderer for `cogito chat`.
//!
//! Translates `StreamEvent`s into role-tagged stdout. TTY-detection
//! degrades to plain text when stdout is not a terminal.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::time::Instant;

use cogito_protocol::stream::StreamEvent;

pub struct Renderer<W: Write> {
    out: W,
    color: bool,
    in_text: bool,
    tool_timers: HashMap<String, Instant>,
}

impl<W: Write> Renderer<W> {
    pub fn new(out: W, color: bool) -> Self { /* … */ }

    pub fn prompt_user(&mut self) -> std::io::Result<()> { /* print "> " cyan */ }

    pub fn on_stream_event(&mut self, ev: &StreamEvent) -> std::io::Result<()> {
        // dispatch table per §3.4
    }

    fn paint(&self, s: &str, code: &str) -> String { /* "\x1b[{code}m{s}\x1b[0m" or s */ }
}

// Convenience constructor used by chat.rs:
impl Renderer<std::io::Stdout> {
    pub fn for_stdout() -> Self {
        let out = std::io::stdout();
        let color = out.is_terminal();
        Self::new(out, color)
    }
}
```

### 3.4 Event → 输出对照

| `StreamEvent` 变体 | 行为 |
| --- | --- |
| `TurnStarted` | `in_text = false`;不打印 |
| `TextDelta { chunk }` | 若 `!in_text`:先打 `\nagent: `(green)再置位;然后原样追加 `chunk` |
| `ToolDispatchStarted { call_id, tool_name }` | 打印 `\n[tool] {tool_name} …`(dim yellow);`tool_timers.insert(call_id, Instant::now())`;`in_text = false` |
| `ToolDispatchEnded { call_id, ok }` | `ms = tool_timers.remove(call_id).map(|t| t.elapsed().as_millis()).unwrap_or(0)`;打印 `\n[tool] {name?} {ok ? "ok" : "err"} ({ms}ms)`<br>**Caveat**:`ToolDispatchEnded` 不携带 `tool_name`,需要从对应的 `ToolDispatchStarted` 记下来 —— 把 `tool_timers` 的 value 改为 `(Instant, String)` |
| `TurnPaused` | `\n[paused]`(dim) |
| `TurnResumed` | `\n[resumed]`(dim);`in_text = false` |
| `TurnCancelled` | `\n[cancelled]`(dim yellow) |
| `TurnFailed { reason }` | `\n[error] {reason}`(red) |
| `TurnCompleted` | `\n`(turn 边界);`in_text = false` |

ANSI 码内联:`36` cyan、`32` green、`33` yellow、`31` red、`2` dim。无依赖。

### 3.5 `chat.rs` 改造点

1. `mod render;` 注册(参考 `mod banner; mod chat;` 的位置)。
2. 进入事件循环前 `let mut renderer = render::Renderer::for_stdout();`。
3. 在 `stdin.read_until` **之前**调用 `renderer.prompt_user()?;`。
4. 事件分支:
   ```rust
   evt = sub.recv() => match evt {
       Ok(e)  => renderer.on_stream_event(&e)?,
       Err(_) => break,
   },
   ```
   替换掉今日只处理 `TextDelta` 的版本。
5. 不动 `tracing::info!` 启动行;不动 Ctrl-C handler;不动 `--config` /
   `--model` / `--session-id` 等任何 clap 参数。

### 3.6 跨 ADR-0004 层校验

`render.rs` 位于 Surface 层(`cogito-cli`),依赖只有:

- `cogito_protocol::stream::StreamEvent`(Protocol 层,允许)
- `std`

不引入对 `cogito-core` / `cogito-model` / `cogito-tools` 的任何新
依赖,不需要新 workspace dep,不动 `[workspace.dependencies]`。

---

## 4 · 测试

### 4.1 单元测试(`crates/cogito-cli/src/render.rs` 内 `#[cfg(test)] mod tests`)

- **plain_text_sequence_no_color**:`color = false`,喂入
  `TurnStarted → TextDelta("hi") → TextDelta(" there") → TurnCompleted`,
  断言输出字节恰为 `\nagent: hi there\n`。
- **tool_lifecycle_no_color**:喂入 `TurnStarted → ToolDispatchStarted("c1", "read_file") → ToolDispatchEnded("c1", true) → TurnCompleted`,
  用正则匹配 `^\n\[tool\] read_file …\n\[tool\] read_file ok \(\d+ms\)\n$`。
- **failed_tool_marks_err**:`ToolDispatchEnded { ok: false }` 触发
  `err` 字串。
- **failed_turn_prints_reason**:`TurnFailed { reason: "boom" }` 产生
  `\n[error] boom\n`。
- **color_codes_balanced**:`color = true`,统计 `\x1b[` 出现次数,
  断言为偶数(每个 open 都有 reset)。
- **text_after_tool_reprints_agent_prefix**:`TextDelta("a") →
  ToolDispatchStarted → ToolDispatchEnded → TextDelta("b")`,
  确认 `agent: ` 出现两次(in_text 在工具调用后被清掉)。

### 4.2 集成 / 手动

- 跑 `make fmt && make fix CRATE=cogito-cli && make fix CRATE=cogito-model && make test CRATE=cogito-cli && make test CRATE=cogito-model`,全绿。
- 手动 `make chat`(走 `cogito.toml` 默认 provider),验证:
  1. 输入提示符 `> ` 颜色正确;
  2. 模型回答前出现 `agent: `;
  3. 触发 `read_file` 工具时出现 `[tool] read_file …` 起止两行 + 耗时;
  4. Ctrl-C 中断 → `[cancelled]`;
  5. `cogito chat | cat` 时所有 ANSI 转义消失(`IsTerminal` 检测生效)。
- 手动 `RUST_LOG=debug make chat`,验证 Anthropic 与 OpenAI-compat 两条
  路径都打 `request: {…}` 一行 compact JSON,不再有 `── request body ──`
  分隔框。

### 4.3 不做的测试

- 不为 model-side debug 日志写测试(纯 `tracing` 副作用,无返回值)。
- 不为 `IsTerminal` 检测写测试(标准库行为,在 CI 上结果取决于运行
  环境)。Renderer 通过 `color: bool` 构造参数允许测试强制开 / 关。

---

## 5 · 验收标准

| 检查 | 通过条件 |
| --- | --- |
| `make ci` | 绿 |
| `cargo nextest run -p cogito-cli` | 绿,包含 §4.1 全部新增测试 |
| `cogito chat` 在 TTY 下 | `> ` / `agent: ` / `[tool]` / `[error]` 颜色可见 |
| `cogito chat | cat` | 输出无 ANSI 转义,语义不丢 |
| `RUST_LOG=debug` 两条 adapter 路径 | 各打 1 行 `request: {…}` compact JSON;无 `── … ──` 框 |
| ADR-0004 层检查(`make ci` 内含) | `cogito-cli` 不新增对 `cogito-model` / `cogito-core` 内部模块的依赖 |
| Protocol / 配置 / 持久化 | **零**变更 |

---

## 6 · 实施分解(给 writing-plans 用)

按依赖顺序:

1. **Model-side compact JSON(`cogito-model`)**:`openai_compat/mod.rs`
   修 4 行;`anthropic/mod.rs` 加 8 行。这一步与 §3 独立,可单独 PR
   或合 PR 都行。
2. **新建 `crates/cogito-cli/src/render.rs`**:实现 `Renderer<W>`、
   `paint()`、`prompt_user()`、`on_stream_event()`。先实现 `color =
   false` 路径 + 全部 §4.1 测试。
3. **打开 color 路径**:`paint()` 在 `self.color` 时插入 ANSI;补
   `color_codes_balanced` 测试。
4. **接入 `chat.rs`**:`mod render;` + 替换事件分支 + 调用
   `prompt_user()`。手动跑 `make chat`。
5. **README / 文档更新**:如 `crates/cogito-cli/README.md` 存在,加一
   行说明 REPL 颜色;否则跳过。

每一步独立可 commit。

---

## 7 · 风险与权衡

| 风险 | 应对 |
| --- | --- |
| 用户的终端不支持 ANSI(老 Windows cmd) | `IsTerminal` 在 stdout 不是 tty 时降级;Windows Terminal / cmd 现代版均支持 ANSI;真出问题可加 `NO_COLOR` 环境变量识别(本次不做,留给后续) |
| ANSI 转义穿透 `tracing` 输出干扰? | `tracing` 默认写 stderr,REPL 写 stdout;两条流物理隔离,互不污染 |
| `ToolDispatchEnded` 没带 `tool_name`,我们靠 `call_id` 配对 | 把 `tool_timers` value 设为 `(Instant, String)`;只有当 `Started` 漏发时才丢名字,这种情况下 fallback 打 `[tool] ? err (?ms)` 即可,不引入协议变更 |
| 一次 turn 同时进行多个 tool 调用(未来 Sprint 5 async path) | `tool_timers` 是 `HashMap<call_id, _>`,天然支持并发多 tool;只是显示顺序按事件到达顺序,无视觉冲突 |
| Sprint 7 真上 TUI 时本模块作废? | 是,可整模块删除;`StreamEvent` 是稳定边界,Sprint 7 重新挂一个 ratatui renderer 即可 |

---

## 8 · 决策日志

| 日期 | 决策 | 理由 |
| --- | --- | --- |
| 2026-05-21 | model-side **不**落盘到文件 | brainstorm 中用户明确否决"先不搞复杂" |
| 2026-05-21 | CLI 着色用方案 (b)(ANSI prefix + TTY fallback) | 比纯文本前缀视觉信号强,比 ratatui 简单一个数量级,零依赖 |
| 2026-05-21 | tool 显示用方案 (b)(name + duration) | 协议零变更;开发者最关心的是"这个工具卡不卡",args/result 已经在 JSONL |
| 2026-05-21 | 不引入 `cogito-cli` 新依赖(`owo-colors` / `nu-ansi-term`) | 5 个 ANSI 码内联即可,hand-rolled 比拉 crate 还短 |
| 2026-05-21 | 此 spec **不**绑定到任何编号 sprint | 属于 cross-sprint dev-experience polish;在 Sprint 4 旁边落地 |

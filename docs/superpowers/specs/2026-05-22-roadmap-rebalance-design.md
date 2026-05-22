# ROADMAP Rebalance · 2026-05-22 — 设计 Spec

> **Status**: Draft (2026-05-22) — pending review + ROADMAP.md / ARCHITECTURE.md
> 同步 + 新增 ADR-0020 / ADR-0021 起草
> **Scope**: 全局版本规划调整(v0.2 主题置换、新增 4 个 sprint、4 个 ADR)
> **Authors**: qiannengsheng + AI brainstorm partner
>
> 本文件是 ROADMAP rebalance 的**决策讨论轨迹**。最终可执行契约住在:
> [`ROADMAP.md`](../../../ROADMAP.md)(sprint 排布) + [`ARCHITECTURE.md`
> §"Version evolution path"](../../../ARCHITECTURE.md)(版本主题表) +
> 各 ADR 文档。此 spec 解释 **why** 和 **决策路径**;ROADMAP / ARCHITECTURE /
> ADR 定义 **what**。
>
> 本 rebalance 不破坏已 ratify 的 ADR-0001 ~ ADR-0007、ADR-0017 ~ ADR-0019。
> 调整对象:ADR-0008 ~ ADR-0015 的编号、范围、所属版本。

---

## 1 · 背景:为什么现在做 rebalance

当前 ROADMAP 制定于 v0.1 启动时,以"先把 Brain 跑起来,Storage / Multimodal /
Subagent 按 v0.2 / v0.3 顺序后置"为节奏。Sprint 4(MCP)在飞、Sprint 4.5
(Config)+ Sprint 4.7(Thinking content)已完成,v0.1 已经具备真实
production-grade core skeleton 的雏形。

但三个外部信号促成本次 rebalance:

1. **团队协作需求显现**:Context Manage 的 trait 框架(ADR-0006 amendment)
   已经在 H01 FSM 加 `ContextManaged` 状态,但内部实现还是 pass-through。
   团队成员不能在没有清晰 trait 边界的前提下并行交付不同的 Compactor /
   Projector 策略——必须把 ADR-0008 从 "post-Sprint-2 spike" 升级成 v0.1
   正式落地。

2. **Skill / Hook / Plugin 在生态上已成事实标准**:agentskills.io
   (Anthropic 主导的开放标准)已被 30+ agent 平台采用;Claude Code 的
   plugin model 已被 Codex 在 `core-skills/` 里直接复用 manifest
   schema。cogito 作为多模型 runtime,如果不在 v0.1 / v0.2 内提供
   Skill / Hook / Plugin 的运行时支持,团队成员就无法以**用户态**(写
   markdown / TOML,不动 Rust 代码)的方式贡献能力,所有扩展都要走
   Rust crate 路径,违背 cogito 作为 "嵌入式 Agent Runtime core" 的定位。

3. **多模态在用户实际使用中已通过工具解决**:目前所有图片/音视频引用都
   是 URL 字符串,通过 `read_file` / `describe_image` 类工具读取后产出
   text。`ContentBlock::Image` 一等公民在 v0.2 优先级被高估——可以推
   到 v0.5 与全 multimedia 工具集一起 ship。`StorageSystem` trait 同
   样推迟。

### 1.1 In-scope(本次 rebalance 决定)

- v0.1 sprint 序列在 Sprint 4 之后净增 2 个 sprint(原 5/6/7/8 = 4 个,
  rebalance 后 5/6/7/8/9/10 = 6 个):新增 Sprint 5 Hook 实化(原 7 上
  半前移)、Sprint 6 Context C2 trait 冻结、Sprint 7 Skill loader;原
  Sprint 5(Async Jobs)/ Sprint 6(Multi-model)/ Sprint 7(下半 TUI)/
  Sprint 8(硬化)整体后挪并合并
- v0.2 主题从 "Storage + Multimodal" 改为 **Extensibility**(Subagent
  minimal + Plugin local-only)
- v0.3 主题从 "Subagent" 改为 **Distributed Collaboration**(Subagent
  full + Plugin git fetch)
- v0.5 主题接管 "Storage + Multimodal"(原 v0.2 + 原 v0.5 多媒体广度
  合并)
- 新增 4 个 ADR(0020 Skill loader / 0021 Plugin manifest+loader /
  0022 Plugin distribution / 0023 Bundled-script execution 占位)
- 调整 ADR-0008 / 0009 / 0010 / 0011 范围与归属版本

### 1.2 Out-of-scope(本次 rebalance 不决定)

- 各 sprint 内部的具体接口签名(留给每个 sprint 的 design spec)
- ADR 文档正文(留给各 ADR 起草)
- v0.4 SaaS-ready / v0.6 Hardening / v1.0 内部细节(均不变)
- Plugin marketplace (P3) 的协议设计(v0.6 spike 范围)

---

## 2 · 决策路径回放(brainstorm 选项归档)

按时间序记录,便于未来回看为什么做这些选择。

### 2.1 Context 内部实现方式

| 选项 | 含义 | 决策 |
|---|---|---|
| C1 | Cargo features 编译期开关 | rejected:不支持运行时切换,与 SaaS 路线不兼容 |
| C2 | Protocol trait + 运行时分发 + 各自 crate | **选** |
| C3 | Skill / Hook / Plugin 作为用户态扩展 | **选**(与 C2 组合) |
| 纯 C2 | 仅 trait,不做 plugin | 与"Skill/Plugin 提前"诉求冲突 |
| 纯 C3 | 仅用户态扩展,Compactor 不可替换 | 不能满足未来"长上下文/短上下文不同 Compactor"需求 |

**决定:C2 + C3。** 引擎可替换组件走 protocol trait + impl crate;业务知识/
工作流走 Skill/Hook/Plugin 用户态。

### 2.2 版本边界

| 选项 | 含义 | 决策 |
|---|---|---|
| A | v0.2 整体改名 Extensibility,v0.1 不动 | rejected:v0.1 tag 时仍无 Skill |
| **B** | Hook 实化 + Skill + Context-C2 入 v0.1;Subagent + Plugin 入 v0.2 | **选** |
| C | 全塞 v0.1(含 Subagent + Plugin) | rejected:v0.1 严重延期 + API 锁早 |

**决定:B。** 理由:v0.1 tag 时 Skill / Hook / Context-C2 已可用,团队成员
立即可写 Skill 包;Plugin 在 v0.2 单独成主题,经 v0.1 真实使用反馈再打磨
manifest schema。

### 2.3 Subagent 形态

| 选项 | 含义 | 决策 |
|---|---|---|
| S1 | 全量 BrainSpawner + 4 工具 + 崩溃恢复 + parent_session_id event tree | rejected:API 锁太早 |
| S2 | 极简 `delegate(role, input) → output` 工具,子 session 为独立顶层 session | partial(用于 v0.2) |
| **S3** | v0.2 = S2;v0.3 = S1(`delegate` 保留为语法糖) | **选** |

**决定:S3。** v0.2 Sprint 11 用 1–1.5 天落极简 delegate;v0.3 一个独立
sprint 升级到全量 BrainSpawner。

### 2.4 Plugin 分发机制

| 选项 | 含义 | 决策 |
|---|---|---|
| P1 | 仅本地路径直引(`cogito.toml [[plugins]] path = ...`) | partial(用于 v0.2) |
| P2 | P1 + git fetch + `cogito.lock` | partial(用于 v0.3) |
| P3 | 完整 marketplace(HTTP index + 签名 + install 命令) | rejected:v0.2 风险溢出 |
| **P4** | 分档:v0.2=P1,v0.3=P2,v0.6+=P3 | **选** |

**决定:P4。** Plugin manifest schema 经 v0.2 半年真实使用考验后再加分发层;
P3 marketplace 协议一旦发布难改,推到 v0.6 spike 才决定。

### 2.5 Skill loader 激活机制(经第二轮调研修正)

**初步建议 K3(tool-call 激活)被推翻。** 三平台调研发现:

- **Codex**(本地代码 `codex-rs/core-skills/`):**文本 sigil** —— 模型在
  回答里写 `$SkillName`,harness 用正则识别 → H11 注入完整 SKILL.md
  (XML 包裹)。**外加 implicit invocation**:模型 `cat skills/X/SKILL.md`
  或 `bash skills/X/scripts/y.py` 时按路径反查自动激活。
- **Claude Code**:自然语言指令(模型被告知 "想用某 skill 就提它名字",
  harness 拦截)+ 用户 `/skill-name` + 少数内置走 `Skill` 工具。
- **Manus**:仅 `/SKILL_NAME` slash command,**没有模型自动激活**。

**关键洞察**:没有任何主流平台用 tool-call 激活——激活只是"把指令包注入上下
文",不需要一次往返;sigil/自然语言几乎零代价。

| 选项 | 含义 | 决策 |
|---|---|---|
| K1 | Eager 全量内联 | rejected:违反 progressive disclosure |
| K2 | 自然语言指令(Claude Code 风格) | rejected:依赖模型训练,多模型不可靠 |
| K3 | Tool-call(`load_skill(name)` 工具) | rejected:浪费 tool call 配额 + 无平台先例 |
| K4 | K2+K3 混合 | rejected:v0.1 工作量翻倍 |
| **K5** | sigil-based(`$SkillName`) + 用户 slash(`/skill X`)双通道 | **选** |

**决定:K5。** Codex 模式;模型不需被训练,任何 instruction-following 模型
都能写 `$name`;H06 用纯正则识别;事件可观测(`SkillActivated`);与 Plugin
namespace 天然组合。

### 2.6 Bundled scripts 处置

| 选项 | 含义 | 决策 |
|---|---|---|
| B-skip | 不实现,作者自行 read_file + bash 组合 | partial(与 B-defer 等价) |
| B-register | 每个脚本自动注册为工具 `skill__<name>__<script>` | rejected:沙箱/权限 schema 需独立 ADR |
| **B-defer** | v0.1 不实现脚本执行,ADR-0023 占位推迟到 v0.3+ | **选** |

**决定:B-defer。** 与 Codex 行为完全一致;Claude Code 的 `` !`cmd` ``
substitution 更激进,留独立 ADR。

### 2.7 Plugin manifest schema

| 选项 | 含义 | 决策 |
|---|---|---|
| 纯 TOML 原生 | 不兼容 Claude Code | rejected:生态壁垒 |
| 纯 JSON 兼容 Claude | 与 cogito 全 TOML 配置风格不符 | rejected |
| **TOML 原生 + JSON 兼容读取** | `.cogito-plugin/plugin.toml` 主格式,`.claude-plugin/plugin.json` 兼容读取 | **选** |

**决定:双格式可读。** 团队成员的 plugin 可同时给 Claude Code 和 cogito 用;
降生态壁垒。

### 2.8 多模态处置

| 选项 | 含义 | 决策 |
|---|---|---|
| 维持 v0.2 原计划 | `ContentBlock::Image` + `StorageSystem` 入 v0.2 | rejected:与 Skill/Plugin 优先级冲突 |
| **从 v0.2 推到 v0.5** | URL-as-text 已在 v0.1 可用;多模态一等公民推到 v0.5 多媒体广度版本 | **选** |
| 完全废弃多模态 | 永不实现 | rejected:违反 v1.0 公开承诺 |

**决定:推到 v0.5。** 原 v0.2 (Storage+Multimodal) + 原 v0.5 (Multimedia
breadth) 合并为新 v0.5 (Storage + Multimodal)。

---

## 3 · 新版本图谱

### 3.1 v0.1 Foundation(扩张:Sprint 4 后 4 个 → 6 个,净增 2 sprint)

| Sprint | 主题 | 来源 / 变更 | 估期 |
|---|---|---|---|
| 0 | 项目骨架 | 已完成 | — |
| 1 | H02 Step Recorder + JSONL store | 已完成 | — |
| 2 | Minimal Loop | 已完成 | — |
| 3 | Resume Coordinator | 已完成 | — |
| 4 | MCP sync tools | 现状(在飞) | 1.5–2 d |
| 4.5 | 配置文件 + base_url override | 已完成 | — |
| 4.7 | Thinking content (ADR-0019) | 已完成 | — |
| **5** | **Hook Pipeline 实化** | **新增:原 Sprint 7 上半前移** | 1 d |
| **6** | **ADR-0008 + Context C2 trait 冻结 + `cogito-context` umbrella crate + 1 个 Compactor** | **新增** | 2–2.5 d |
| **7** | **Skill loader** | **新增** | 1.5–2 d |
| 8 | Async Jobs | 原 Sprint 5(后挪) | 2 d |
| 9 | Multi-model Strategy + TUI | 原 Sprint 6 + Sprint 7 下半合并 | 2 d |
| 10 | v0.1 硬化 + tag v0.1.0 | 原 Sprint 8 | 1 d |

**v0.1 总延期估算**:相对原计划 +4–5 天(Sprint 5/6/7 净增 4.5–6.5 天,
其它 sprint 顺延零额外成本)。

**v0.1 tag 时已交付能力**:

- 完整 11-component Brain skeleton(FSM + Resume + 全部 H0X 真实化)
- Hook Pipeline 真实化(2 个示例 hook + purity 规则强制)
- Skill loader(agentskills.io 标准 + sigil 激活 + 三档 scope 发现)
- Context Manage(ADR-0008 ratified + Compactor/HistoryProjector/
  SystemPromptInjector trait 冻结 + 1 个 Compactor impl)
- Multi-model 支持(Anthropic + OpenAI-compat + vLLM/SGLang)
- TUI + CLI 双surface
- MCP sync tools(stdio + streamable-HTTP)
- Async Jobs(`JobManager` + JSONL job log)

### 3.2 v0.2 Extensibility(新主题,3 sprint)

| Sprint | 主题 | 内容要点 | 估期 |
|---|---|---|---|
| 11 | Subagent (S2 minimal) | **不开新 crate**——模块住 `cogito-core::runtime::subagent`;`delegate(role, input) → output` 工具(ToolProvider 实现也住该模块,通过 ExecCtx 拿 `Arc<dyn BrainSpawner>`);`BrainSpawner` trait 入 `cogito-protocol`;strategy YAML 加载;子 session = 独立顶层 session;ADR-0011 缩水版 | 1–1.5 d |
| 12 | Plugin (P1 local-only) | `cogito-plugin` crate;`.cogito-plugin/plugin.toml` 解析 + `.claude-plugin/plugin.json` 兼容读取;加载 `skills/`/`agents/`/`hooks/`/`mcp.toml`/`commands/`;**所有 bundled artifact 统一 `<plugin_id>:<artifact_name>` namespace**(skill、agent role、hook id、MCP server name 同规则);per-project enable/disable;ADR-0021 | 1.5–2 d |
| 13 | v0.2 硬化 + tag v0.2.0 | 集成测试(本地 plugin 端到端)+ 跨 scope 同名冲突 + resume_chaos 新增 plugin-loaded skill 场景 | 1 d |

**v0.2 主题叙事**:"Extensibility" —— 把 Skill / Hook / Subagent / MCP 用
统一的 Plugin 包封装起来,团队成员可以**完全用户态**贡献能力。

### 3.3 v0.3 Distributed Collaboration(主题替换)

原 v0.3 主题是 "Subagent";本次 rebalance 后 v0.3 内涵升级为:

| Sprint | 主题 | 内容要点 |
|---|---|---|
| X | Subagent (S1 full) | `BrainSpawner` trait + 4 工具(`spawn_agent`/`wait_agent`/`send_input`/`cancel_agent`)+ parent_session_id event tree + 父子崩溃语义;`delegate` 保留为语法糖;ADR-0011 升级 |
| Y | Plugin (P2 git fetch + lock) | `cogito plugin sync`;`cogito.lock` 文件;git URL pin;ADR-0022 |
| Z | v0.3 硬化 + tag v0.3.0 | 父子崩溃 chaos 场景 + git plugin 网络失败 fallback |

**v0.3 主题叙事**:"Distributed Collaboration" —— 多智能体编排 + 分布式
plugin 分发。

### 3.4 v0.4 SaaS-ready(主题不变,backend 命名调整)

`cogito-store --features postgres`(原 `cogito-store-postgres` crate;
合并入 umbrella `cogito-store`,见 §4.5)+ `cogito-storage-s3` +
`TenantContext` + `MetricsRecorder` + `cogito-observability-otel` +
sandbox lifecycle + credential isolation。ADR-0012/0013/0014 编号不变。

### 3.5 v0.5 Storage + Multimodal(主题接管)

原 v0.2 + 原 v0.5 内容合并:

- ADR-0009 `StorageSystem` trait + URI scheme + `ContentBlock::Image`
- ADR-0010 Multimedia tool conventions(MIME、`outputs_model_visible_multimodal` flag)
- `cogito-storage-local` crate(`file://` + `http(s)://` + `blob://`)
- `cogito-tools-multimedia` 完整工具集(`transcribe_audio` / `extract_frames` /
  `summarize_video` / `describe_image` / `analyze_frame` / `synthesize_speech`)
- `ContentBlock::Image` 通过 `ModelGateway` 适配器(Anthropic native /
  OpenAI image_url)端到端打通
- `outputs_model_visible_multimodal` flag 被 H05 实化(过滤与所选模型
  不兼容的工具)

### 3.6 v0.6 Hardening + Marketplace spike(扩展)

原 v0.6 内容 + 新增:

- Plugin marketplace (P3) 设计 spike → ADR-0023(可选,可推 v0.7)
- 其它(Hook policy maturity / load test / soak test / migration tooling /
  `cogito-storage-http` + Storage HTTP wire protocol = ADR-0015)不变

### 3.7 v1.0 API freeze(不变)

公开 API 稳定承诺、event log forward-compat 严格模式、`#[non_exhaustive]`
全应用、首次 GA 发布。

---

## 4 · Crate 增量

### 4.1 v0.1 新增

| Crate | Sprint | 角色 |
|---|---|---|
| `cogito-context` | 6 | **Umbrella crate**:容纳 Compactor / HistoryProjector / SystemPromptInjector 的所有未来实现;v0.1 范围内只内置 `compactor::truncate` 一个 Compactor;后续策略(`compactor::summarize` / `compactor::sliding` / `projector::tool_elision` / ...)按模块追加,**不开新 crate**。`build_pipeline(&ContextConfig)` 工厂住此 crate(CLAUDE.md §"Tagged-config factories")。 |
| `cogito-skills` | 7 | Skill loader:发现 / 注册 / namespace / 与 H04/H06/H11 接缝 |

### 4.2 v0.2 新增

| Crate | Sprint | 角色 |
|---|---|---|
| `cogito-plugin` | 12 | Manifest 解析(双格式)+ 加载组装 + per-project 配置 |

**注:Subagent 不开新 crate。** v0.2 S2 minimal 的 `delegate` 工具 +
`BrainSpawner` impl + ToolProvider 包装统统住 `cogito-core::runtime::subagent`
模块(约 200 行);Brain 通过 `dyn ToolProvider` 看到,层规则不破坏。

### 4.3 v0.3+ 推延项与未来抽离

- `cogito-storage-local`(原 v0.2 → 现 v0.5):保持原 crate 名,只是归属
  版本变更
- `cogito-tools-multimedia`(v0.5):同上
- **`cogito-subagent` crate 可能于 v0.3 S1 升级时抽出**:届时 BrainSpawner
  + 4 工具 + 父子事件树 + 崩溃语义代码量预计 ~1.5–2k 行,有抽出合理性;
  也可继续留在 `cogito-core::runtime::subagent`。判断点推迟到 v0.3 sprint
  开工时再做,本 rebalance 不锁。
  - 即使抽出,`delegate` 工具保留为 `spawn_agent + 同步 wait` 的语法糖,
    实质行为不变。

### 4.4 Crate 增量规则保护(CLAUDE.md §"Inviolable design rules")

CLAUDE.md 明确"添加新 crate 需要 explicit approval"。本 rebalance 净增
crate 数:

- **v0.1 净增 2 个**:`cogito-context`(umbrella)+ `cogito-skills`
- **v0.2 净增 1 个**:`cogito-plugin`
- v0.3 视情况净增 0 或 1 个(`cogito-subagent` 抽出决定)
- v0.5+ 净增 2 个(`cogito-storage-local` + `cogito-tools-multimedia`,
  原本就规划在内)

每个新 crate 映射到 ADR-0004 既有层:

- `cogito-context` → **Brain 内部组件,与 protocol 伴生**(Compactor 等
  trait 在 protocol 层;impl 在 cogito-context;Runtime 装配时注入 Brain)
- `cogito-skills` → Hands(Skill 视作"知识工具提供方")
- `cogito-plugin` → Hands(Plugin 是 ToolProvider + HookProvider +
  SkillProvider 的组合,无独立 Brain 视图)

无新层引入;Brain 仍只 import `cogito-protocol`。

### 4.5 既有 crate 命名整顿:`cogito-store-jsonl` → `cogito-store`

与 §4.1 `cogito-context` umbrella 同样的理由:把 backend 名(`jsonl`)印
在 crate 名上,等于让未来每加一种 store backend(postgres、sqlite、...)
就开新 crate。整顿为 umbrella crate `cogito-store`:

```
cogito-store/
  Cargo.toml          # [features] default = ["jsonl"], postgres, sqlite, ...
  src/
    lib.rs            # 公共 trait re-export + build_store 工厂
    jsonl/            # v0.1 默认 backend(原 cogito-store-jsonl 内容)
      mod.rs
      ...
    postgres/         # v0.4 添加(feature-gated)
      mod.rs
```

**影响面统计**:30+ 文件引用 `cogito-store-jsonl`,包括 `Cargo.toml`(workspace
manifest + 各成员 manifest)、`Cargo.lock`、`Makefile`、ARCHITECTURE.md /
ROADMAP.md / CLAUDE.md / AGENTS.md / CHANGELOG.md 等 doc、`crates/cogito-core/`
下多处 import、ADR-0006 / ADR-0007 / ADR-0019。

**执行节奏**:
- **不在 rebalance ratify 阶段顺手做**——纯粹机械重命名 +30 文件触碰,
  混进 rebalance commit 会模糊 review focus
- **作为独立 PR 处理**,目标:**v0.1.0 tag 之前完成**(即 Sprint 10 v0.1
  硬化期前后);CLAUDE.md / AGENTS.md / 新 ADR 起草若引用此 crate,**直接
  使用新名 `cogito-store`** (其它历史 ADR 不动)
- **Accepted ADR 的处理**:ADR-0006 / ADR-0007 / ADR-0019 不修改正文(冻结
  历史决策);可在 ADR docket 表格的"备注"或专门一个 ADR-0024 "Crate
  naming consolidation" 里记录重命名事实
- **v0.4 计划项 `cogito-store-postgres`**:从 ROADMAP / ARCHITECTURE 移除
  作为独立 crate 的标注,改为 `cogito-store --features postgres` 描述
- 重命名 PR 标题示例:`refactor(workspace): cogito-store-jsonl → cogito-store
  (jsonl as default feature)`

---

## 5 · ADR 增量

### 5.1 状态变更

| ADR | 原状态 | 新状态 |
|---|---|---|
| **ADR-0008** Context Management | TBD spike post-Sprint 2 | **Accepted v0.1 Sprint 6** |
| **ADR-0009** StorageSystem | TBD v0.2 | **TBD v0.5**(版本变更) |
| **ADR-0010** Multimedia tool conventions | TBD v0.2 | **TBD v0.5**(版本变更) |
| **ADR-0011** Subagent execution model | TBD v0.3 | **拆两段**:v0.2 minimal(S2)+ v0.3 full(S1);文档分两次 amendment |

### 5.2 新增 ADR

| ADR | 主题 | 归属版本 | 锁住的内容 |
|---|---|---|---|
| **ADR-0020** Skill loader | v0.1 Sprint 7 | K5 sigil 激活策略;3 档 scope 优先级(Repo > User > System);Plugin namespace 规则;`SKILL.md` frontmatter 字段(`name` / `description` / `disable-model-invocation` / `user-invocable`);bundled scripts 推迟立场 |
| **ADR-0021** Plugin manifest + loader | v0.2 Sprint 12 | TOML 主格式(`.cogito-plugin/plugin.toml`)+ JSON 兼容读取(`.claude-plugin/plugin.json`);`skills/` / `agents/` / `hooks/` / `mcp.toml` / `commands/` 默认路径;**所有 bundled artifact 统一 `<plugin_id>:<artifact_name>` namespace**;per-plugin / per-artifact enable/disable 字段;**v0.2 范围 = local path only(P1)** |
| **ADR-0022** Plugin distribution | v0.3 | git URL + commit pin + `cogito.lock` schema;`cogito plugin sync` 命令语义;缓存目录布局;失败 fallback 策略 |
| **ADR-0023** Bundled-script execution(占位) | v0.3+ TBD | 沙箱模型 / 权限边界 / 是否自动注册为工具 / 与 ExecCtx 的关系 |
| **ADR-0024** Crate naming consolidation(候选,可合入 ADR-0005 amendment) | v0.1 Sprint 10 之前 | `cogito-store-jsonl` → `cogito-store`(JSONL 改为 default feature);未来同样的命名整顿原则(crate 名只标层 / 角色,backend 走 feature);历史名映射表 |

### 5.3 ADR-0011 拆段细节

ADR-0011 (Subagent execution model) 在原 ROADMAP 是单一 v0.3 决策。本
rebalance 后:

- **ADR-0011 v0.2 范围**:`delegate(role, input) → output` 工具;子 session
  是独立顶层 session(不引入 `parent_session_id` event tree);失败语义 =
  子 session 失败 → 工具返回 `ToolResult::Error`;不持久化子 session 状态
  到父 session 事件日志(各自独立 store)。
- **ADR-0011 v0.3 amendment**:`BrainSpawner` trait + 4 工具 +
  `parent_session_id` / `depth` / `role` session metadata + 父子崩溃语义
  (parent crash → child 继续;child crash → parent 拿 `AsyncFailed` 事
  件)+ `SubagentSpawned` / `SubagentInputSent` / `SubagentCompleted`
  事件变体 + depth limit 强制。

两段共享同一个 ADR 文档,通过 amendment 章节区分(类比 ADR-0006 的
amendment 2026-05-19 模式)。

---

## 6 · ARCHITECTURE.md 同步项

`ARCHITECTURE.md` 中需要按本 spec 同步的章节:

### 6.1 §"Version evolution path" 表格

```
v0.1 Foundation        — 不变(scope 扩张但主题不变)
v0.2 Extensibility     — 主题改名(原 Storage + Multimodal)
v0.3 Distributed       — 主题升级(原 Subagent)
v0.4 SaaS-ready        — 不变
v0.5 Storage + Multimodal — 接管原 v0.2 + 原 v0.5 内容
v0.6 Hardening + Marketplace — 主题微扩(新增 marketplace spike)
v1.0 API freeze        — 不变
```

### 6.2 §"v0.1 scope (IN / OUT)" 表格

新增 IN 行:

- Hook Pipeline 实化 ✅
- Skill loader(agentskills.io 标准)✅
- Context Manage(ADR-0008 + 1 个 Compactor)✅

移出 v0.1(原 IN,现保留为 IN 但归属版本调整):无。原 v0.1 scope 不缩,
只扩。

### 6.3 §"ADR docket" 表格

按 §5 增量更新 ADR-0008 ~ ADR-0023 行。

### 6.4 §"Workspace layout"

- **新增行**(按现有表格格式追加):
  - `cogito-context`(Brain 内部 / Protocol 伴生,v0.1)
  - `cogito-skills`(Hands,v0.1)
  - `cogito-plugin`(Hands,v0.2)
- **重命名行**:`cogito-store-jsonl` → `cogito-store`(归属/层不变,v0.1)
- **移除 / 调整行**:`cogito-store-postgres`(v0.4)从独立 crate 改为
  `cogito-store --features postgres`,可在备注列注明
- **暂不入表**:`cogito-subagent`(v0.3 决定是否抽出再加,本 rebalance
  v0.2 范围内仅是 `cogito-core::runtime` 内部模块)

---

## 7 · 风险与开放问题

### 7.1 Sprint 7 H06 sigil 正则的边界情况

**风险**:K5 用类似 `\$[a-zA-Z0-9_:-]+` 的模式识别 `$SkillName`,可能误识
shell snippet 里的 `$VARIABLE`、SQL 里的 `$1` `$2` 参数占位符、模板字符串
`${name}`、bash heredoc 等。

**缓解候选**(具体规则在 ADR-0020 锁定):
- (a) 仅当 `$X` 命中已注册 skill 名才激活;未命中视作普通文本(零误激活
  风险,但用户可能困惑为何 `$known-name` 在代码块里也激活)
- (b) 跳过 fenced code block / inline code 内的 sigil(借助 H06 已有的 SSE
  事件序列识别 markdown 上下文)
- (c) 在 system prompt 里明确告知模型 "如要在回答中提到字面 `$Name`,用
  反引号包裹"(类比 Markdown 转义)
- (d) 调研 Codex 实际采用的正则与边界条件,作为 ADR-0020 §"Sigil 词法
  规则" 的起点

预计 (a) + (c) 已覆盖 95% 场景;(b) 视实现复杂度评估;(d) 是 Sprint 7
开工前的小调研项。

### 7.2 Plugin 中 hook 的运行时加载

**风险**:Sprint 5 Hook 实化时,hook 加载来源只考虑 `cogito.toml`
`[[hooks]]` 段。Sprint 12 Plugin 加入后,plugin 内的 hooks 需要追加到同
一 HookPipeline。HookPipeline 已经在 Sprint 5 锁定 trait 后,Sprint 12 要
不要扩 trait?

**缓解**:Sprint 5 设计 HookPipeline 时,Trait `HookProvider` 已经类似
`ToolProvider` 是"提供方"模式;Plugin 加载时再注册一个 `HookProvider`
即可,无需 trait 扩展。这一点在 ADR-0020 / Sprint 5 design spec 里明确。

### 7.3 Async Jobs 在 v0.1 的必要性

**风险**:本 rebalance 后 Async Jobs 推到 Sprint 8,Hook/Skill/Context-C2
都不直接需要它。如果 v0.1 不做 Async Jobs,sprint 数可压到 9,延期减少
2 天。

**评估**:
- Sprint 6 一个 Compactor 实现如果选 summarize-via-model,需要发起一次
  模型调用——这是 async tokio task,但不是 Job(不持久化、无跨进程
  resume)。Compactor 用 `tokio::spawn` 直接搞定,不依赖 `JobManager`。
- v0.3 Subagent full(S1)的"async wait_agent"语义需要 `JobManager`。
- v0.4 SaaS-ready 的多副本部署也需要 `JobManager` 的跨进程 resume。

**结论**:Async Jobs 仍留在 v0.1 Sprint 8,因为(a)v0.1 tag 时已声称
production-grade,缺 async job 难讲通;(b)v0.3 Subagent S1 强依赖。如果
sprint 节奏紧张,Sprint 8 可与 Sprint 9 部分并行(不同人手)。

### 7.4 Skill 命名空间冲突时的 UX

**风险**:Repo > User > System scope 优先级 + Plugin 自动 namespace,理论
上不冲突;但用户写 `cogito.toml` 时如果两个 plugin 都用 `id = "review"`
namespace,Plugin loader 怎么报错?

**缓解**:Sprint 12 Plugin loader 启动时全局校验 plugin id 唯一性,重复
plugin id 立即 fatal(类比 MCP server name 重复的处理)。错误信息指明
冲突的两个 plugin 路径。在 ADR-0021 锁定。

### 7.5 团队成员入门曲线

**风险**:v0.1 tag 时团队成员能写 Skill,但 Plugin 要等 v0.2。期间团队
成员怎么组织多个 Skill?

**缓解**:v0.1 范围内,Skill 通过 Repo scope (`.cogito/skills/...`) 或
User scope (`~/.cogito/skills/...`) 直接放置;团队成员可以用 git
submodule 把一组 skill 凑成 monorepo 子目录,在每个项目的 `cogito.toml`
里通过约定路径引用。等 v0.2 Plugin 落地后,这些 skill 自然升级为
plugin-bundled。

### 7.6 Subagent crate 抽离推迟决策

**情况**:v0.2 S2 minimal 把 subagent 模块住 `cogito-core::runtime::subagent`,
~200 行;v0.3 S1 升级后预计 ~1.5–2k 行。

**风险**:
- 抽出过早(v0.2 时就抽)→ 过早抽象,200 行一个 crate 体积比 Cargo.toml
  + lib.rs 模板还小,纯亏复杂度
- 抽出过晚(v0.4+ 才抽)→ `cogito-core` 已经被一堆 v0.3 subagent 代码污染,
  把"runtime 是 Brain hosting 装配点"的纯粹性破坏

**判断点**:v0.3 sprint 开工时复盘 `cogito-core::runtime::subagent` 行数与
依赖独立性,如果(a)行数过 1k 且(b)与 `cogito-core::runtime` 其它模块
共享依赖少于 30%,则抽出 `cogito-subagent` crate;否则继续留在
`cogito-core::runtime` 内。

**与 Tool 的依赖关系**:无论抽不抽,`delegate` / `spawn_agent` 等工具的
ToolProvider 实现都通过 `ExecCtx` 拿 `Arc<dyn BrainSpawner>`(`BrainSpawner`
trait 永远在 `cogito-protocol`)。Brain 端(`cogito-core::harness`)永远
只看到 `dyn ToolProvider`,不感知 subagent 实现在哪个 crate。这意味着
**抽与不抽对 Brain / Protocol API 零影响**,纯粹是物理打包决定。

### 7.7 `cogito-store-jsonl` → `cogito-store` 重命名的 scope

**情况**:30+ 文件引用 `cogito-store-jsonl`(workspace + 各 Cargo.toml +
代码 import + 多份 doc + 3 个 Accepted ADR + Cargo.lock + Makefile)。

**风险**:
- 混进 rebalance ratify commit → review focus 被 30 文件 diff 稀释
- 漏改某处导致编译失败 → CI 红
- 改动 Accepted ADR → 历史决策记录被污染(违反 ADR 不可变性原则)

**缓解 / 执行约束**:
- **独立 PR**,标题清晰标注 `refactor(workspace): cogito-store-jsonl → cogito-store`
- **目标 landing 时点**:v0.1.0 tag 之前(即 Sprint 10 v0.1 硬化 PR 系列
  之一),不晚于 v0.1.0
- **Accepted ADR 处理**:ADR-0006 / ADR-0007 / ADR-0019 **正文不动**;
  起草一个 ADR-0024 "Crate naming consolidation"(或在 ADR-0005
  production-scope amendment)记录此次重命名 + 历史 crate 名映射表,作为
  追溯指针
- **新 doc / ADR / spec**(包括本 spec)**直接使用新名 `cogito-store`**;
  老 doc 在重命名 PR 中一并更新
- **`cogito-store-postgres` 计划项处理**:同步从 ROADMAP / ARCHITECTURE
  移除,改为 `cogito-store --features postgres` 描述

---

## 8 · 实施顺序

按依赖与并行可能性给出建议执行顺序:

1. **本 spec ratify** → 更新 `ROADMAP.md` + `ARCHITECTURE.md` + 起草 4 个新 ADR
   (ADR-0020 / 0021 / 0022 / 0023 各一份占位)
2. **完成 Sprint 4 (MCP)**(在飞)
3. **Sprint 5 Hook 实化**(单人 1 天可收)
4. **Sprint 6 ADR-0008 + Context C2**(单人 2–2.5 天;**这是 v0.1 关键
   路径**,必须先于 Sprint 7 完成,因为 Skill loader 需要在 H11 注入
   skill 内容,而 H11 trait 在 Sprint 6 才稳定)
5. **Sprint 7 Skill loader**(单人 1.5–2 天)
6. **Sprint 8 Async Jobs**(可与 Sprint 9 部分并行;单人 2 天)
7. **Sprint 9 Multi-model + TUI**(2 天)
8. **Sprint 10 v0.1 硬化 + tag v0.1.0**(1 天)
9. **v0.2 启动**:Sprint 11 → 12 → 13(共 3.5–4.5 天)
10. **v0.2 tag** → 团队成员开始用 Plugin 模式贡献能力
11. v0.3 / v0.4 / v0.5 / v0.6 / v1.0 按 §3 主题推进

**关键路径里程碑**:

- **v0.1 tag**:Hook + Skill + Context-C2 + 多模型 → 团队成员可写 Skill
- **v0.2 tag**:Plugin (local) + Subagent (minimal) → 团队成员可写 Plugin
- **v0.3 tag**:Plugin (git) + Subagent (full) → 多智能体编排 + 分布式分发
- **v0.4 tag**:SaaS-ready → 多副本部署能力
- **v0.5 tag**:Storage + Multimodal → 多模态一等公民
- **v1.0 tag**:API 冻结 → 公开 GA

---

## 9 · 后续动作清单

本 spec ratify 后立即:

- [ ] 更新 `ROADMAP.md` v0.1 / v0.2 / v0.3 / v0.5 章节(按 §3);v0.4
      `cogito-store-postgres` 描述同步改为 `cogito-store --features postgres`
- [ ] 更新 `ARCHITECTURE.md` §"Version evolution path" / §"v0.1 scope" /
      §"ADR docket" / §"Workspace layout"(按 §6 + §4.5 rename)
- [ ] 起草 ADR-0020 Skill loader(锁 §2.5 + §2.6 决策)
- [ ] 起草 ADR-0021 Plugin manifest + loader(锁 §2.4 P1 + §2.7 决策)
- [ ] 起草 ADR-0022 Plugin distribution(占位,v0.3 详写)
- [ ] 起草 ADR-0023 Bundled-script execution(占位)
- [ ] ADR-0008 / 0009 / 0010 / 0011 状态行同步更新
- [ ] 起草 ADR-0024 "Crate naming consolidation"(或合并入 ADR-0005
      amendment),记录 `cogito-store-jsonl` → `cogito-store` 重命名 +
      历史名映射表
- [ ] **独立 PR**:`cogito-store-jsonl` → `cogito-store` 重命名(JSONL 改为
      default feature;30+ 文件机械重命名;Accepted ADR 正文保持不变,
      仅 ADR docket 表更新);目标 landing 早于 v0.1.0 tag
- [ ] Sprint 5 / 6 / 7 各自的 implementation plan 由 writing-plans skill 生成

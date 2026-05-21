# Sprint 4.5 · 配置文件 + base_url override — 设计 Spec

> **Status**: Accepted (2026-05-21)
> **Sprint**: v0.1 · Sprint 4.5(夹在 Sprint 4 Async Jobs 之后、Sprint 5 Multi-model Strategy 之前的小冲刺)
> **Authors**: qiannengsheng + AI brainstorm partner
>
> 本文件是 Sprint 4.5 的**决策讨论轨迹**。可执行契约住在 durable 文档:
> [`ADR-0017`](../../adr/0017-cogito-runtime-configuration-model.md) 锁
> 架构;[`docs/components/H10-strategy-selector.md`](../../components/H10-strategy-selector.md)
> 锁 strategy schema(由 Sprint 5 实现 loader)。
> 此 spec 解释 **why** 和 **实施分解**;ADR-0017 / H10 doc 定义 **what**。

---

## 1 · Sprint 目标

让 `cogito chat` 从"CLI flags + 硬编码 ENV"升级为"读 `cogito.toml`(可
选)+ ENV + CLI 三层 merge",其中:

- 提供商配置(凭据 + base_url + 鉴权头)进文件;
- Anthropic-compatible 三方端点通过 `base_url` 字段暴露(底层 `AnthropicConfig`
  已经支持,这次只是把它接到配置文件);
- 现有 `just chat --model X --provider anthropic` 工作流**零迁移成本**——
  没有 `cogito.toml` 时,行为等价 Sprint 2(legacy ENV bridge)。

这是 GitLab Issue #1 三个子需求里的**前两个**(配置 + ENV 启动;
Anthropic-compat 三方 LLM)。**第三个子需求**(OpenAI 原生 `responses`
API 适配器)留给 Sprint 5,因为它本质是一个新的 `ModelGateway` 实现,
属 `cogito-model` 范畴,跟"配置加载"是独立工作。

### 1.1 In-scope

1. 新增 crate `cogito-config`(ADR-0017 §5)。Default features 提供值
   类型 + trait + ENV loader + merge 逻辑;feature `file` 加 `toml` +
   `serde_yaml` 依赖、提供 `FileConfigLoader`。
2. `cogito-model` 新增 `ProviderConfig` enum(serde tagged on `kind`)
   + `build_gateway(cfg) -> Result<Arc<dyn ModelGateway>, ModelError>`
   工厂函数。两个 v0.1 variants:`Anthropic`、`OpenAiCompat`。
3. `cogito-cli` 重构 `chat.rs`:
   - `clap` 加 `--config <path>` 参数;
   - 启动序列:`FileConfigLoader::load() → EnvConfigLoader::load() →
     CliPatch::from(args) → merge → finalize → build providers →
     build gateway → RuntimeBuilder`;
   - Legacy ENV bridge:`cogito.toml` 不存在且 `providers` 为空时,从
     `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENAI_BASE_URL` 合成
     一个名为 `default` 的 provider(保留 Sprint 2 行为)。
4. 文件搜索路径四档(ADR-0017 §7):`--config` > `COGITO_CONFIG` > `./cogito.toml`
   > `$XDG_CONFIG_HOME/cogito/config.toml`。
5. 秘密插值 `${ENV_VAR}` + `${ENV_VAR:-default}`(ADR-0017 §6),由
   `FileConfigLoader` 在反序列化后、merge 前应用。
6. `strategies_dir` 字段反序列化保留(default `./strategies`),Sprint
   4.5 **不**遍历目录、**不**加载 YAML —— 这是 Sprint 5 工作。
7. 文档:`docs/components/H10-strategy-selector.md` 加一段注脚提及
   ADR-0017 §9(filename = strategy name;丢弃 `applicable_models`);
   `crates/cogito-cli/README.md`(如有)更新 `--config` 用法。
8. 单元 / 集成测试:见 §5。

### 1.2 Out-of-scope(明确不做,避免 scope creep)

| 不做的事 | 何时做 |
|---|---|
| OpenAI Responses adapter | Sprint 5(`cogito-model` 加新 enum variant + 新 wire 模块) |
| `strategies/*.yaml` loader | Sprint 5(`cogito-config::FileConfigLoader::load_strategies(dir)`) |
| `--strategy <name>` CLI | Sprint 5(strategy registry 落地后) |
| Plugin / Subagent 配置 section | post-v0.3(分别等机制 ADR) |
| Database `ConfigLoader` 实现 | v0.4+(由消费方 Server 代码自实现) |
| Profile / multi-env overlay | 永不(ADR-0017 §8 决定;未来 ADR 可重启) |
| Hot reload | 永不(ADR-0017 §13);进程重启接入新配置 |
| Element-wise array merge for `providers` | 永不(ADR-0017 §3);用 `${ENV_VAR}` 覆盖字段 |
| `cogito.toml` schema migration tooling | 不存在(还没有 v1 文件);v0.x 字段加减走 `#[serde(default)]` |
| Windows `%APPDATA%` 路径 | 留给后续 ADR 加;v0.1 只锁 XDG |

每一行的"为什么不在 Sprint 4.5"都在 ADR-0017 或本 spec 决策轨迹里有
直接落点。

---

## 2 · 决策轨迹(Q1–Q8 简录)

完整论证轨迹见 ADR-0017。本节只串关键 fork、不重复论证。

### Q1 · ADR-0017 锁哪些 section

**锁**:`runtime` + `providers` + `strategies`。**保留**:`plugins`、
`subagents`(等机制 ADR)。理由:strategies 是 Sprint 5 紧邻工作,
H10 doc 已有 schema 草稿,顺势锁定避免后续返工;plugins / subagents
机制未定,空设计风险大。

### Q2 · 文件物理布局

**`cogito.toml`(TOML)+ `strategies/*.yaml`** 混合。TOML 给固定大小的
runtime + providers,YAML 给"多条目 + 多行 prompt"的 strategies 注册表。
两种格式各用所长,DB source 不受影响(逻辑 schema 不变)。

### Q3 · 多源组合

**Layered partial merge**:`CLI > ENV > file/db > defaults`,每源产出
`RuntimeConfigPartial`,按优先级合并;`Some(_)` 覆盖。Provider 数组
**整体替换**不元素 merge;字段覆盖通过 `${ENV_VAR}` 在 file 内做。

### Q4 · `[[providers]]` schema

**命名 provider 实例数组**,serde tagged on `kind`。理由:Issue #1
明确"同时需要真 Anthropic + Anthropic-compat 三方"——同 kind 多实例
是真实需求;命名引用为 Sprint 5 strategy 绑定 / v0.4 多租户都铺好路。

### Q5 · crate 落点

**新增 `cogito-config` crate**,而非 feature-gate 进 `cogito-core`。
理由(用户对 feature gate 推回):cogito-core 是 Brain + Runtime 之
家,层级敏感;v0.4 数据库 source 进 core 会拉 `sqlx`,污染加剧。
独立 `cogito-config` 用 feature gates 控制 `toml`/`yaml` 依赖(default
zero-dep),数据库 source 由消费方自己 `impl ConfigLoader` 不进本 crate。
crate 增量:**+1**(`cogito-config`),后续不再涨。

### Q5.5 · 构造内聚原则(写入 CLAUDE.md)

`ProviderConfig` → `Arc<dyn ModelGateway>` 的 `match`-on-`kind` dispatch
**住在 `cogito-model`**(拥有实现的 crate),不在 surface。Surface 调用
`cogito_model::build_gateway(cfg)` 一行拿到 trait object。新增 provider
variant(Sprint 5 OpenAI Responses)只动 `cogito-model`,surface 0
改动。此原则已加入 `CLAUDE.md` §Coding standards 作为通用准则。

### Q6 · 三个小决策

- **秘密插值**:`${ENV_VAR}` + `${ENV_VAR:-default}`(shell 风格,行业最熟)。
- **搜索路径**:`--config` > `COGITO_CONFIG` > `./cogito.toml` >
  `$XDG_CONFIG_HOME/cogito/config.toml`,**首中即取**不内 merge。
- **Profile**:**不做**——dev/prod 用不同 `--config` 文件路径解决。

### Q7 · `[runtime]` 字段集

四个字段:`session_root`、`default_provider`、`default_model`、
`strategies_dir`。**故意不放**:`default_system_prompt`(strategy 概念)、
tracing(`RUST_LOG` ENV 够用)、timeout(已在 provider 上)。

缺省解析规则:`--provider X` > `runtime.default_provider` > 唯一 provider
自动选 > 报错。Model 同理但**不**自动选(model id 不在配置文件枚举)。

### Q8 · Strategy schema + Rust 类型 + CLI 向后兼容

- **Strategy schema** 引用 H10 doc;两条 ADR 级补强:filename = strategy
  name(去 YAML 内 `name:` 字段),丢弃 `applicable_models` glob。
- **Strategy provider-agnostic**:YAML **不**写 `provider:` 字段;运行时
  由 `--provider` / `runtime.default_provider` 绑定。
- **`RuntimeConfig` Rust 类型**:见 ADR-0017 §12 完整定义。
- **CLI back-compat**:`--base-url` / `--system` 作为 post-merge field
  patch 应用到选中的 provider / strategy 实例。`cogito.toml` 不存在
  时,legacy ENV bridge 合成 `default` provider,行为等价 Sprint 2。

---

## 3 · `cogito-config` crate 内部分解

新 crate 拓扑:

```
crates/cogito-config/
├── Cargo.toml
├── src/
│   ├── lib.rs            # pub re-exports
│   ├── types.rs          # RuntimeConfig, RuntimeConfigPartial, RuntimeSection*
│   ├── loader.rs         # ConfigLoader trait, ConfigError
│   ├── env.rs            # EnvConfigLoader (default feature)
│   ├── merge.rs          # merge_layers, finalize
│   ├── interpolate.rs    # ${VAR} / ${VAR:-default} 处理 (feature = "file")
│   └── file.rs           # FileConfigLoader (feature = "file")
└── tests/
    ├── merge.rs          # 多层 partial merge 行为
    ├── interpolate.rs    # 秘密插值边界(未定义 / 默认 / 转义)
    ├── file_loader.rs    # TOML 解析 + 搜索路径
    └── env_loader.rs     # ENV 解析
```

**Cargo features**:

```toml
[features]
default = []
file = ["dep:toml", "dep:serde_yaml"]

[dependencies]
cogito-protocol = { workspace = true }
cogito-model    = { workspace = true }
serde           = { workspace = true, features = ["derive"] }
serde_json      = { workspace = true }
thiserror       = { workspace = true }
async-trait     = { workspace = true }
tracing         = { workspace = true }

toml            = { workspace = true, optional = true }
serde_yaml      = { workspace = true, optional = true }
```

**Workspace 增项**(`Cargo.toml` 顶层):

- `serde_yaml = "0.9"`(若未列入)
- 复用现有 `toml`、`serde`、`thiserror` 等

---

## 4 · 修改清单(crate-by-crate)

### 4.1 新增:`crates/cogito-config/`

如 §3 所述。模块拆分明确;每个文件 < 200 行(`merge.rs` 可能略大,
但应该控制在 300 行以内)。

### 4.2 `crates/cogito-model/`

新增文件:`src/provider_config.rs` —— `ProviderConfig` enum +
`build_gateway` 工厂。

```rust
// crates/cogito-model/src/provider_config.rs

use std::sync::Arc;
use std::time::Duration;

use cogito_protocol::gateway::{ModelError, ModelGateway};
use serde::{Deserialize, Serialize};

use crate::{
    AnthropicConfig, AnthropicGateway, OpenAiCompatConfig, OpenAiCompatGateway,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderConfig {
    Anthropic {
        name: String,
        api_key: String,
        #[serde(default = "defaults::anthropic_base_url")]
        base_url: String,
        #[serde(default = "defaults::anthropic_version")]
        anthropic_version: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    OpenAiCompat {
        name: String,
        #[serde(default)]
        api_key: Option<String>,
        base_url: String,
        #[serde(default = "defaults::auth_header")]
        auth_header: String,
        #[serde(default = "defaults::auth_scheme")]
        auth_scheme: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
}

impl ProviderConfig {
    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic { name, .. } | Self::OpenAiCompat { name, .. } => name,
        }
    }
}

pub fn build_gateway(cfg: ProviderConfig)
    -> Result<Arc<dyn ModelGateway>, ModelError>
{
    match cfg {
        ProviderConfig::Anthropic {
            api_key, base_url, anthropic_version, timeout_secs, ..
        } => {
            let mut c = AnthropicConfig::with_api_key(api_key);
            c.base_url = base_url;
            c.anthropic_version = anthropic_version;
            if let Some(s) = timeout_secs { c.timeout = Duration::from_secs(s); }
            Ok(Arc::new(AnthropicGateway::new(c)?))
        }
        ProviderConfig::OpenAiCompat {
            api_key, base_url, auth_header, auth_scheme, timeout_secs, ..
        } => {
            let mut c = OpenAiCompatConfig::with_base_url(base_url);
            c.api_key = api_key;
            c.auth_header = auth_header;
            c.auth_scheme = auth_scheme;
            if let Some(s) = timeout_secs { c.timeout = Duration::from_secs(s); }
            Ok(Arc::new(OpenAiCompatGateway::new(c)?))
        }
    }
}

mod defaults {
    pub(super) fn anthropic_base_url() -> String {
        "https://api.anthropic.com".into()
    }
    pub(super) fn anthropic_version() -> String {
        "2023-06-01".into()
    }
    pub(super) fn auth_header() -> String {
        "Authorization".into()
    }
    pub(super) fn auth_scheme() -> String {
        "Bearer".into()
    }
}
```

`src/lib.rs` 加 `pub mod provider_config;` + re-export `ProviderConfig`
和 `build_gateway`。`timeout_secs` 字段直接用 `Option<u64>` 表示秒数,
不引入 `humantime-serde` 依赖(理由见 §7.1)。

### 4.3 `crates/cogito-cli/`

重构 `src/chat.rs::build_gateway`:

```rust
// Before (Sprint 2)
fn build_gateway(args: &ChatArgs) -> Result<Arc<dyn ModelGateway>> {
    // 30+ lines of env-reading + provider matching
}

// After (Sprint 4.5)
pub async fn run(args: ChatArgs) -> Result<()> {
    let config = cogito_config::load_runtime_config(&args).await?;
    let provider_cfg = config.select_provider(&args)?;
    let gateway = cogito_model::build_gateway(provider_cfg)?;
    // ... rest unchanged
}
```

`load_runtime_config` 内部串接:

```rust
// crates/cogito-config/src/lib.rs (export 一个便捷函数)
pub async fn load_runtime_config<P: AsRef<Path>>(
    config_path: Option<P>,
) -> Result<RuntimeConfig, ConfigError> {
    let file_loader = FileConfigLoader::resolve(config_path)?;
    let env_loader  = EnvConfigLoader::default();

    let layers = vec![
        file_loader.load().await?,
        env_loader.load().await?,
    ];
    let merged = merge_layers(layers);
    merged.finalize()
}
```

CLI patch(`--model` / `--provider` / `--base-url` / `--system` /
`--session-root`)在 finalize 之后应用,见 ADR-0017 §11。

`cogito-cli/Cargo.toml`:

```toml
cogito-config = { workspace = true, features = ["file"] }
```

`clap` 加新 arg:

```rust
#[derive(Debug, Args)]
pub struct ChatArgs {
    // Existing args ...
    /// Path to cogito.toml (overrides COGITO_CONFIG, ./cogito.toml, XDG).
    #[arg(long)]
    pub config: Option<PathBuf>,
}
```

### 4.4 `Cargo.toml`(workspace root)

新成员加入:

```toml
[workspace]
members = [
    # existing ...
    "crates/cogito-config",
]
```

新增 `[workspace.dependencies]` 条目:`serde_yaml`(若未列入)。

### 4.5 `docs/components/H10-strategy-selector.md`

在 §"v0.x Sprint 5 scope" 加一个 admonition:

> **2026-05-21 update**:per ADR-0017 §9, strategy file basename
> (without `.yaml`) is the canonical strategy name; the YAML body
> drops `name:` and `applicable_models:` fields. The two existing
> draft files (`strategies/claude-opus.yaml`, `strategies/gpt-4.yaml`)
> will be rewritten when Sprint 5 lands the loader.

### 4.6 `ROADMAP.md`

在 Sprint 4 和 Sprint 5 之间插入 Sprint 4.5 节,标 `[ ]`(未完成):

```markdown
#### Sprint 4.5 · 配置文件 + base_url override (0.5–1 day)

- [ ] `cogito-config` crate(value types + ConfigLoader trait + EnvConfigLoader + merge)
- [ ] `cogito-config` feature `file` → FileConfigLoader (`cogito.toml`)
- [ ] `cogito-model::ProviderConfig` + `build_gateway` 工厂
- [ ] `cogito-cli` 重构 `chat.rs`:`--config` 参数 + 三层 merge
- [ ] Legacy ENV bridge:`cogito.toml` 缺席时合成 `default` provider
- [ ] 单元/集成测试覆盖 merge、插值、搜索路径
- [ ] 文档:ADR-0017 引用、H10 doc 注脚、ROADMAP 更新
```

### 4.7 `justfile`(可选)

如果 `just chat` 当前硬编码了一些 env-only 调用方式,Sprint 4.5
不动它;legacy 路径自动兼容。后续(Sprint 5)可加一条 `just chat-with-config`
示范。

---

## 5 · 测试计划

### 5.1 单元测试(`cogito-config`)

| 测试 | 覆盖 |
|---|---|
| `merge::tests::cli_overrides_env_overrides_file` | layered merge 优先级 |
| `merge::tests::array_replaced_not_merged` | providers 数组整体替换语义 |
| `merge::tests::finalize_fills_defaults` | session_root / strategies_dir 兜底 |
| `merge::tests::auto_select_sole_provider` | `default_provider = None` + 1 provider |
| `merge::tests::ambiguous_provider_errors` | `default_provider = None` + 2+ providers |
| `interpolate::tests::expand_env_var` | `${VAR}` 取值 |
| `interpolate::tests::expand_with_default` | `${VAR:-default}` fallback |
| `interpolate::tests::missing_var_errors` | `${VAR}` 未定义 → `ConfigError::MissingEnv` |
| `interpolate::tests::no_interpolation_for_non_strings` | 数字 / bool 字段不插值 |
| `file::tests::search_path_order` | 四档路径首中即取 |
| `file::tests::file_not_found_returns_default_partial` | 找不到 → empty partial |
| `file::tests::deny_unknown_fields` | 拼错字段在 deserialize 报错 |
| `env::tests::empty_env_returns_empty_partial` | 没相关 ENV → empty partial |

### 5.2 单元测试(`cogito-model`)

| 测试 | 覆盖 |
|---|---|
| `provider_config::tests::anthropic_deserialize` | TOML/JSON 反序列化 |
| `provider_config::tests::openai_compat_deserialize` | 同上 |
| `provider_config::tests::missing_required_field` | 缺 `name` / `api_key` 报错 |
| `provider_config::tests::deny_unknown_kind` | `kind = "xxx"` 报错 |
| `provider_config::tests::build_anthropic` | 工厂构造 + base_url override |
| `provider_config::tests::build_openai_compat` | 工厂构造 |

### 5.3 集成测试(`crates/cogito-cli/tests/`)

| 测试 | 覆盖 |
|---|---|
| `tests/config_legacy_env_bridge.rs` | 无 `cogito.toml`,只设 `ANTHROPIC_API_KEY` → 合成 default provider → 与 Sprint 2 行为等价 |
| `tests/config_file_only.rs` | `cogito.toml` 写 anthropic provider + `${ANTHROPIC_API_KEY}` → 加载成功,gateway 可构造 |
| `tests/config_cli_overrides.rs` | `cogito.toml` 设 base_url=A,`--base-url=B` → 选中 provider 的 base_url=B |
| `tests/config_anthropic_compat_third_party.rs` | `cogito.toml` 声明 `[[providers]] kind = "anthropic"` + 内部 base_url → 走 `AnthropicGateway` 但请求发到内部端点 |

这四个集成测试覆盖 Issue #1 子需求 1 + 2 的全部用户可见行为。

### 5.4 不在 4.5 测试范围

- Strategy YAML loader 行为(Sprint 5)
- DB ConfigLoader 行为(v0.4+,消费方测)
- Profile / multi-env(永不)
- Hot reload(永不)

---

## 6 · 实施顺序

建议按依赖顺序提交,每一步独立可测:

1. **`cogito-config` 骨架**:lib.rs + types.rs + loader.rs + env.rs +
   merge.rs。Default features 跑通,单测 §5.1 前 7 项绿。
2. **`cogito-config` file feature**:interpolate.rs + file.rs。完成
   §5.1 剩余项。
3. **`cogito-model::provider_config`**:enum + 工厂。§5.2 全过。
4. **`cogito-cli` 接线**:`chat.rs` 重构 + legacy ENV bridge。§5.3
   集成测试全过。
5. **ROADMAP.md + H10 doc 注脚**:文档同步。
6. `just ci` 全绿;手动跑 `just chat --config tests/fixtures/cogito.toml`
   一次端到端。

每一步独立 PR 或独立 commit;实施过程中如发现 ADR 决策需要修正,
**回过头修 ADR-0017**——不在 spec 内"对决策做例外"。

---

## 7 · 风险 / 未决项 / 已知边界

### 7.1 风险

- **依赖最小化**:`timeout_secs: Option<u64>` 直接写秒数,
  不引 `humantime-serde`(否则 `timeout = "5m"` 这种人类可读时间
  值要拉一个小 crate)。4.5 依赖图最小化是显式取舍——可读性损失
  极小(`timeout_secs = 300` vs `timeout = "5m"`),依赖收益明显。
- **`deny_unknown_fields` 太严**:用户 YAML/TOML 拼错字段会直接报错。
  Pro:早发现;Con:版本迁移时需 `#[serde(rename)]` 别名。当前阶段
  字段集小,接受严格策略。
- **Legacy ENV bridge 行为漂移**:Sprint 2 用 `ANTHROPIC_API_KEY` 或
  `OPENAI_BASE_URL` 各自构造 gateway。Bridge 合成的 `default` provider
  必须**完全复现**这个行为(包括"两个 env 都设了优先 Anthropic" /
  "都没设报哪个错误"等细节),由 §5.3 第一条集成测试守住。

### 7.2 未决项(留给 Sprint 5)

- Strategy file 的 `model_id` 是否要 wildcard(如 `model_id: "claude-*"`)
  以服务多个具体 model。**ADR-0017 §9 显式拒绝**(`applicable_models`
  glob 丢弃);如果 Sprint 5 实施时发现真有需求,需新 ADR 重启该讨论。
- `--strategy <name>` CLI flag。Sprint 5 工作,本 spec 不涉及。

### 7.3 已知边界(永不在此阶段做)

见 §1.2 表格。

---

## 8 · Acceptance criteria

Sprint 4.5 完成需要满足:

1. `just ci` 全绿(fmt + clippy + test 跨所有 crate)。
2. 单元测试 §5.1 / §5.2 全过。
3. 集成测试 §5.3 全过(尤其 legacy ENV bridge)。
4. 手动 smoke test:`just chat --model claude-opus-4-7` 在仅有
   `ANTHROPIC_API_KEY` 环境下成功跑通一轮(legacy 路径)。
5. 手动 smoke test:写一份 `cogito.toml`(含真 Anthropic + Anthropic-compat
   内部 provider),`just chat --provider anthropic-internal --model
   claude-opus-4-7` 请求发往内部 base_url(用 tcpdump / mitmproxy 验证,
   或临时点错的 base_url 看错误信息)。
6. ADR-0017 与本 spec 内部一致;若实施过程修改了任一决策,**修 ADR
   不修例外**。
7. ROADMAP.md 加 Sprint 4.5 节,完成项 check 掉。

---

## 9 · References

- [ADR-0017](../../adr/0017-cogito-runtime-configuration-model.md) — 架构锁定
- [`CLAUDE.md`](../../../CLAUDE.md) §Coding standards — Tagged-config factories rule
- [`docs/components/H10-strategy-selector.md`](../../components/H10-strategy-selector.md) — strategy schema(Sprint 5 loader)
- [`crates/cogito-model/src/anthropic/mod.rs`](../../../crates/cogito-model/src/anthropic/mod.rs) — `AnthropicConfig` 已有 `base_url`
- [`crates/cogito-cli/src/chat.rs`](../../../crates/cogito-cli/src/chat.rs) — Sprint 2 surface,本 sprint 重构对象
- GitLab Issue gitlab.sz.sensetime.com/compass/cogito#1
- 2026-05-21 brainstorming transcript(Q1–Q8)

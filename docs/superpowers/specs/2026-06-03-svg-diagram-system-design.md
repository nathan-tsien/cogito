# SVG 配图体系 — 定稿文档配图升级设计

- 日期: 2026-06-03
- 状态: Implemented（2026-06-03，17 张 SVG，分支 docs/svg-diagrams）
- 范围: README.md · ARCHITECTURE.md · `docs/components/*` （核心）；`docs/configuration/overview.md`（扩展）
- 不在范围: ADR、`docs/superpowers/specs|plans`（决策/执行文档，已能表达逻辑关系，保留现有 ASCII）

## 目标

把定稿文档里的 ASCII box-drawing 配图替换为统一风格的 **SVG 矢量图**，提升阅读体验，同时：

- 不引入新的构建依赖（无 mermaid / d2 / graphviz / node）；
- 呈现"技术风格"，清晰表达**组件设计**与**职责范围**；
- 在 GitHub 的浅色与深色主题下都正常显示；
- 不破坏仓库重视的 grep 能力与可访问性。

## 方案选择

| 方案 | 依赖 | 取舍 |
|---|---|---|
| **手写 SVG + 共享设计系统**（选定） | 零 | 像素级控制，技术风格，无构建步；编辑较繁琐，但定稿文档变更频率低 |
| D2 / Mermaid / Graphviz → 编译 SVG | 需安装二进制 + CI 编译步 | 可维护性好但违反"少依赖"，且 mermaid 风格被否决 |

选定**手写 SVG**：每张图是一个独立 `.svg` 文本文件，提交进仓库，GitHub 与编辑器直接渲染，无需任何工具链。

## 设计语言（统一规范）

所有 SVG 复用同一套约定，样式通过 SVG 内嵌 `<style>` 的 CSS class 表达，便于一处调整：

- **自带浅色卡片背景**：每张图绘制自己的 `card` 圆角背景（`#f6f8fa` 填充 + `#d0d7de` 描边）。这是深色模式策略——图作为一张"浅色图卡"在浅色/深色主题下都显示一致，无需维护双份文件或 `<picture>` 切换。
- **颜色 = 层**：Brain 靛蓝 `#4c6ef5`、Session 绿 `#2da44e`、Boundary 紫、Hands 琥珀 `#d4a72c`、Runtime 石板灰 `#7d8590`。语义终态：绿=Completed、琥珀=Paused、红 `#cf222e`=Failed。
- **嵌套容器 = 职责范围**：外层容器框圈定一层/一组的边界，内部为组件框。
- **箭头 = 数据流/控制流**，边上标注触发者（如 `H06 stream → events`）。
- **排版**：标签用系统无衬线栈；代码标识符（`H01`、trait 名、`ModelInput` 等）用等宽栈 `.mono`。
- 文件位置 `docs/diagrams/<name>.svg`，kebab-case 命名。

参考样张（已落地，本次设计的视觉基准）：
- `docs/diagrams/harness-layers.svg` — 分层 + 11 组件职责带
- `docs/diagrams/turn-fsm.svg` — 回合状态机

## 嵌入约定

Markdown 中以 `<img>` 引用，附 `alt` 文本：

```markdown
<img src="./docs/diagrams/harness-layers.svg" alt="Agent Runtime shell drives the Brain; eleven components in five responsibility bands" width="780">
```

- 路径相对当前文档（README 用 `./docs/diagrams/…`，组件文档用 `../diagrams/…`）。
- GitHub markdown 不渲染内联 `<svg>`，只渲染 `<img>`/`![]()` 引用的 svg 文件——本方案据此设计。

### grep / 可访问性回退

替换 ASCII 为图片会丢失图内文字的 grep 能力。处理：

- **信息密集的图**（如 resume 入口路径、actor 拓扑）：在 `<img>` 下方保留原 ASCII，包进折叠块：

  ```markdown
  <details><summary>Text version</summary>

  ```text
  ...原 ASCII 图...
  ```
  </details>
  ```
  默认折叠不干扰阅读，但保留 grep 与纯文本可读性。
- **简单图**：仅 `alt` 文本，不保留 ASCII。

## 待制作清单（核心范围 ~21 张）

| 文档 | 现有图 | 处理 |
|---|---|---|
| README.md | 0 | 新增 1 张：分层总览（Brain/Session/Boundary/Hands/Runtime/Surface）作为门面图 |
| ARCHITECTURE.md | 10 | 全部替换：分层栈、依赖约束、回合 FSM、resume 入口路径、import 规则、actor 拓扑等 |
| docs/components/H01-turn-driver.md | 4 | 替换 |
| docs/components/H03-resume-coordinator.md | 2 | 替换 |
| docs/components/H09-hook-pipeline.md | 1 | 替换 |
| docs/components/H11-context-manage.md | 1 | 替换 |
| docs/components/cogito-subagent.md | 1 | 替换 |
| docs/components/cogito-tui.md | 1 | 替换 |

扩展（确认后再做）：`docs/configuration/overview.md`（4 张）。

无图的组件文档（H02/H04/H05/H06/H07/H08/H10）本次不强制新增配图，保持现状；如个别图能显著提升清晰度可在执行阶段按需补一张，不作为目标。

## 复用与一致性

- `harness-layers` 与 `turn-fsm` 在 ARCHITECTURE 与对应组件文档中**复用同一个 svg 文件**，不重复绘制。
- 设计 token（颜色、字号、圆角、箭头 marker）在每个文件的 `<style>` 中保持一致；新图从既有图复制 `<style>` 块起步。

## 验证

- 每个 `.svg` 通过 XML 解析校验（`python3 -c "import xml.dom.minidom; ..."`），无需新依赖。
- 关键图渲染为 PNG 目检（开发期临时用 `cairosvg`，不进仓库、不进 CI）。
- 检查每处嵌入：相对路径正确、`alt` 非空、复杂图带 `<details>` 文本回退。
- 不修改任何 Rust 代码与 CI；纯文档变更。

## 不做（YAGNI）

- 不做深色/浅色双份 SVG。
- 不引入图表 DSL 或编译步。
- 不动 ADR/spec/plan 的 ASCII 图。
- 不为所有无图组件文档强行造图。

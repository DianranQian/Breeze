# AGENTS.md — 微风（Breeze）编辑器 · AI 协作规范

> 所有在本仓库工作的 AI Agent，先读本文件，再按任务的读取清单读代码。
> 当前基线：2026-10-26，`5973581`，11 个 workspace 成员，
> `cargo test --workspace` = 1,315 passed / 0 failed / 8 ignored（源码里 `#[test]` 共 1,319 处）。
> 诊断依据：`docs/AI开发流程诊断.md`（A 结论 / C 耦合簇 / D 读取策略 / F D1）。
> 冲突处理：代码与测试是事实；文档过期就顺手修正，并同步更新本文件基线。

---

## 0. 三条核心纪律（不可协商）

### 纪律一：开源优先，拒绝造轮子

新增任何功能前，按以下顺序做选择，并在回复里说明选择理由：

1. **先用仓库已有机制**（主题、i18n、停靠、序列化、撤销栈等）。
2. **再用成熟生态**：Bevy 官方 API、`bevy_egui`、`egui`、`egui_dock`、`rapier2d`、`serde`/`ron` 等。
3. **然后查 crates.io**：维护状态、Bevy 版本兼容性、许可证。
4. **最后才考虑手写**，且必须经用户确认。

以下底层能力**严禁手搓**，除非先给出“现有方案检索结果 + 不适用原因 + 维护成本 + 回滚方式”并获确认：

- 停靠/浮动/分割布局（已有 `egui_dock`）
- 主题与语义色槽（已有 `editor_ui/src/theme.rs`）
- 国际化（已有 `messages.tsv` + `tools/gen_messages.py`）
- 序列化与存档（已有 `serde`/`ron` + `scene_doc`/`savegame`）
- 2D 物理（已有 `rapier2d`）、图像/音频解码、文件对话框、窗口与输入

禁止引入与现有栈并行的第二套 GUI、序列化或状态管理框架。

### 纪律二：防污染与最小读取（绝对红线）

**临时文件**

- 严禁在仓库根目录新建 `.verify_v1`、`.verify_*`、`.tmp/` 等临时/验证目录。
- 编译、测试、截图、日志等验证产物只能放 `target/verify/`（或系统临时目录）。
- 验证结束立即删除自己创建的目录；失败最多保留最近一次现场。
- 不得删除非本次创建的 `target/verify/` 内容；不确定先问。

**分析范围**

- 自动忽略：`target/`、`.verify*/`、`.tmp/`、`.bevy_studio/`、`explore/`。
- **禁止全库通读**。先声明读取清单，再开始读。
- 单任务首轮预算：**≤ 6 个文件、≤ 3,000 行**；超预算先说明理由。
- 定位只允许 `git grep -n <符号>` 或按行号区间读取；不要整文件读取巨型文件：
  - `crates/editor_app/src/actions/`（模块目录：`mod.rs` 2,390 行 + 13 个域文件，按域定位）
  - `crates/editor_ui/src/state.rs`（3,310 行，按结构体/枚举定位）
  - `crates/editor_app/src/plugin.rs`（2,029 行，按函数定位）
  - `crates/editor_core/i18n/messages.tsv`（1,355 行，按前缀 grep）
  - `docs/工作日志.md`（4,172 行，除考古外不要读）
- 超出清单的读取必须记录一行：
  `扩展读取：<文件> —— 原因：<为什么清单不够>`
  （写进回复；需要时也写进提交信息）。
- 每次任务回复都要列出“实际读取清单”。

### 纪律三：强制结构化汇报

每次任务结束必须使用以下五个标题，不能只写“已完成”：

```markdown
### 🎯 1. 任务总结
### 🛠️ 2. 具体实施（修改了哪些文件、核心逻辑是什么）
### ✅ 3. 验证与状态（编译/测试是否通过、临时文件是否清理）
### 🚀 4. 下一步计划
### ⚠️ 5. 潜在风险
```

未运行的验证必须明确写“未验证”及原因，不得用推断代替结论。

---

## 1. 规模税：先知道自己在动哪

| 事实 | 数字 | 含义 |
| --- | --- | --- |
| 全库 Rust | 125,753 行（`src/` 108,485 + `tests/` 17,268），11 个成员 | 瓶颈不在总量 |
| 三个中心文件 | `actions/`(5,418，`mod.rs` 2,390 + 13 个域文件 3,028) + `state.rs`(3,310) + `plugin.rs`(2,029) ≈ 10,757 行 | 几乎每次改动都会经过 |
| 单一执行器 | `mod.rs` 的 `执行单个动作` 628 行（13 个路由跳转 + 16 条总表分支 + 13 组哨兵），13 个 `&mut` 参数已收进 `动作上下文` | 新增行为 = 先找对应域文件，找不到才加进总表 |
| 动作变体 | `UiAction` 186 个，其中 170 个（91%）已由域文件认领 | 定义/处理/摘要三处 + 无编译期约束的构造点 |
| 共享状态 | `界面状态` 86 个 `pub` 字段 | 17 个面板同时 `&mut`，影响面不可知 |
| 数据扇出 | `绘制环境`(14) → `绘制上下文`(21) → 面板 | 新增一个数据源 = 6 个触点 |
| 面板扇出 | 元数据 1 张表（`docks.rs` 869 行里的 `面板注册表`，17 行）+ 分发 1 处 `match tab` + 预设由注册表派生（`workspaces.rs` 1,824 行） | 手工触点收到 `lib.rs` 那处 match，漏了直接编译报错 |

**推论**：优先小步、可回滚、只加不改的改动；不要顺手大重构。

**`actions/` 是模块目录**：`mod.rs` 保留分派入口、剩余总表、以及被各域共用的
辅助函数与运行时系统（前部路由与共享辅助 + 执行器 628 行 + 后部约 1,131 行）；
13 个域各占一个子文件——
`resources` / `animation` / `edit` / `tilemap` / `theme` / `build` / `agent` /
`project` / `ui` / `save` / `settings` / `run_debug` / `prefab`。
新增动作时先找对应域文件，跨域或无处安放时才动 `mod.rs`；
总表只剩 16 条（试听 / 剪贴板 / 资源管理器 / 编辑器内运行 / 脚本 / 项目运行）。

**面板元数据只剩一张表**（D21 第一、二步）：`crates/editor_ui/src/docks.rs`（869 行）里的
`面板注册表` 是唯一一份人工同步的面板元数据 —— 17 行，每行 `tab` / 标题键 / 菜单键 /
中文标题 / 可关闭 / 分组。标题、菜单项、"菜单文本 → 面板"反映射、`Tab::全部()`、
`Tab::全部含智能体()`，以及四个内置工作区出厂布局的兜底标签组，全部由它派生。
`Tab::描述` 读编译期构建的 `面板描述表`：注册表少一行或重复登记都表现为
`error[E0080]` 编译失败，运行期不再有 panic 分支。
新增面板仍然要动 `editor_ui/src/lib.rs` 的 `match tab`（绘制分发）—— 这是刻意保留的
编译期穷尽护栏，不是第二份元数据表。

---

## 2. 同名不同层：读之前先确认是哪一层

| 名字 | 权威/底层 | 界面/适配层 |
| --- | --- | --- |
| `state.rs` | `crates/editor_core/src/state.rs`（权威编辑状态） | `crates/editor_ui/src/state.rs`（界面状态 + `UiAction`） |
| `*_bridge` | `crates/editor_app/src/scene_bridge.rs`（场景↔Bevy） | `crates/bsn_bridge/`（BSN↔`scene_doc`） |
| `asset_preview` | `crates/editor_app/src/asset_preview.rs`（读盘/投影） | `crates/editor_ui/src/asset_preview.rs`（绘制） |
| 三种“运行” | `actions::推进运行时`（编辑器内） | `run.rs` 子进程 / `pie_proto` 编辑器内原型 |

**PIE 明确是原型**（`pie_proto`），不要并入 `run.rs` 的职责。

---

## 3. 按任务类型的读取清单（先读这些，再扩展）

| 任务类型 | 必读 | 可选 | 禁止 |
| --- | --- | --- | --- |
| **T1 新增/修改面板** | `crates/editor_ui/src/docks.rs`（`面板注册表` + 派生清单 + `补全标签组`）、`workspaces.rs`（预设兜底栏）、`crates/editor_ui/src/lib.rs`（分发 match）、`state.rs`（目标字段+相关 `UiAction`）、`crates/editor_app/src/plugin.rs`（动作摘要+绘制环境）、`messages.tsv`（相关前缀）、目标面板文件 | `visual_spec.rs`、`theme.rs`、`menu_bar.rs`（window 分组已派生）、`crates/editor_app/tests/multi_panel_workspace.rs` | 其余 15 个面板；`preview`/`run`/`agent*`/`pie_*`/`debug_bridge`；`examples/`；除 `架构设计.md` 外的 `docs/` |
| **T2 新增 `UiAction`** | `state.rs`（`pub enum UiAction` 段+动作队列）、`actions/`（`mod.rs` 路由器+目标域子文件）、`plugin.rs`（`动作摘要`）、`crates/editor_app/tests/action_reachability.rs` | 目标域既有处理函数、`crates/editor_app/tests/action_loop.rs` | `editor_core/*`、`scene_doc/*`（除非动作要读写文档） |
| **T3 新增组件类型** | 目标组件族模块（`scene_bridge`/`components3d`/`physics_components`/`ui_components`/`audio_components`）、`scene_bridge.rs`（trait+注册表+同类 impl 范例）、`inspector_panel.rs`（字段绘制）、`messages.tsv` | 对应投影模块（若可渲染）、同类组件测试 | `actions/mod.rs`（`构造默认组件` 已泛化）、`agent*`、`gui_probe` |
| **T4 改渲染/预览** | 目标投影模块、`crates/editor_app/src/preview.rs`、`scene_bridge.rs`（投影段）、`state.rs`（相机/视口段）、`viewport_panel.rs` **或** `viewport3d.rs` | `theme.rs`（叠加层配色）、`crates/editor_app/tests/preview_aspect_ratio.rs` | `agent*`、`assets_panel.rs`、`timeline_panel.rs`、`工作日志.md` |
| **T5 新增文件格式/资产类型** | `editor_core` 新模块 + `lib.rs`、`actions/`（载入/保存动作，放对应域文件）、`plugin.rs`（资源初始化）、`messages.tsv` | `project.rs`（资源列表/导入状态）、`assets_panel.rs` | `preview.rs`、`pie_proto.rs`、`gui_probe` |
| **T6 改界面文案** | `messages.tsv`（按前缀 grep）、`tools/gen_messages.py`（扫描目标/应排除）、目标文件 | — | 其余一切 |
| **T7 修 bug（有报错/栈）** | 报错指向的文件 + 它直接调用的接口文件 + 覆盖它的测试 | 同簇的第二个文件 | 全库；无关 crate |

---

## 4. 耦合簇与联合读取包（一次读完，不要拆散）

| 簇 | 联合读取包 | 行数量级 |
| --- | --- | --- |
| 1 动作总线 | `state.rs`（`UiAction` 段）+ `actions/`（`mod.rs` 路由器 + 目标域文件）+ `plugin.rs`（摘要） | ~900 |
| 2 面板注册 | `docks.rs`（全）+ `workspaces.rs`（预设段）+ `menu_bar.rs`（window 段）+ `editor_ui/lib.rs`（分发段） | ~1,000 |
| 3 共享状态 | `state.rs`（`界面状态` + `绘制上下文`）+ `plugin.rs`（`绘制环境` + `绘制一帧`） | ~470 |
| 4 渲染投影 | 目标投影模块 + `scene_bridge.rs`（trait+一个 impl）+ `preview.rs`（相机/渲染目标） | ~900 |
| 5 组件扩展 | 组件族模块 + `scene_bridge.rs`（trait+注册表）+ `inspector_panel.rs`（字段绘制） | ~700 |
| 6 i18n | `tools/gen_messages.py`（全）+ `messages.tsv`（前缀 grep 结果） | ~300 |
| 7 运行调试 | `run.rs`（三表注释+启动）+ `build.rs` + `editor_core/run_entry.rs` | ~700 |
| 8 AI Agent | `agent.rs`（数据模型）+ `agent_dsh.rs`（协议）+ `agent_runtime.rs`（进程） | ~1,000 |
| 9 资产引用 | `scene_doc/document.rs`（属性）+ `asset_refs.rs` + `editor_core/asset_index.rs` | ~700 |

**不要**一次读完 11 个 `agent*` 文件；**不要**在任务不涉及 PIE 时读 `pie_proto.rs`。

---

## 5. 改动前后检查单

**动手前**

1. 判定任务类型（T1–T7）与所属耦合簇。
2. 写出读取清单；确认没有全库通读。
3. 检索现有实现与生态方案（纪律一），记录结论。
4. 确认改动是否需要 i18n 词条、布局存档迁移或注册表同步。

**动手时**

- 可见性优先 `pub(crate)`，不要随手放宽 `pub`。
- 新增中文字面量必须同步 `messages.tsv`；界面文案不要硬编码在绘制代码里。
- 格式化只作用于本次改动文件（`rustfmt <文件>` / `cargo fmt -- <文件>`），
  不要 `cargo fmt --all`，避免碰到并行工作线。
- 每步一个可回滚的 commit；重构与行为变更不要混在一起。

**收尾**

```bash
cargo check --workspace --all-targets
cargo test -p <改动的 crate>      # 先跑受影响 crate
cargo test --workspace            # 合并前跑全量
python tools/gen_messages.py --check
git status --short                # 确认没有根目录临时产物
```

- 验证产物只落 `target/verify/`，结束即删。
- 测试数量以 `cargo test` 输出为准；里程碑后同步更新本文件顶部与
  `docs/项目现状总览.md` 的“当前事实”。

---

## 6. 文档体系与优先级

| 文档 | 角色 | 使用方式 |
| --- | --- | --- |
| `AGENTS.md` | 协作规则与当前基线 | 每次任务先读 |
| `docs/AI开发流程诊断.md` | 只读诊断（A–F） | 查痛点、耦合簇、读取策略 |
| `docs/engine_capability_map.md` | 能力成熟度权威（M0–M5） | 判断“已开发/半成品/未开发” |
| `docs/项目现状总览.md` | **历史快照**（基线 `193bf26`） | 只作模块说明，数字不作当前事实 |
| `docs/下一阶段开发路线图.md` | 计划与优先级 | 接任务前对照 |
| `docs/验收证据/` | 可复核的 GUI/测试证据 | 引用时必须确认文件真实存在 |
| `docs/工作日志.md` | 历史过程 | 除考古外不要读 |

- 文档里的路径必须真实存在；失效证据索引要改成现存路径或明确标注“现场未保留”。
- 不在多处重复维护同一事实；数量、基线只在本文件顶部与 `项目现状总览.md` 顶部更新。

---

## 7. 本轮明确不做（防跑偏）

- 不按层重划 crate（`editor_core` 不拆五个）。
- 不给 `UiAction` 加宏/代码生成。
- 不重构 AI Agent 簇的代码结构（最多补一张事件→模块表）。
- 不把 PIE 并入 `run.rs`；PIE 保持“原型”定位。
- 不启动 3D 物理、粒子、寻路、导航网格、动画状态机、命令面板、LSP、多窗口等
  产品能力，除非用户明确排期。
- 不做“大爆炸”重构；优先 D1–D5 的“只加不改”，D6 之后每步单独可回滚。
- `actions/` 已按域拆完 13 个域（170/186 个变体有归属）；继续拆就沿用
  `Result<(), UiAction>` 路由器 + 分组哨兵的模式，每个域一个 commit，
  恢复成单文件或按层重划都不做。
- 剩下 16 条总表动作（试听 / 剪贴板 / 资源管理器 / 编辑器内运行 / 脚本 / 项目运行）
  暂时不动：它们分属不同耦合簇，且"编辑器内运行（PIE）"与"项目运行（子进程）"
  混在一个域文件里会模糊 PIE 的原型定位。
- `mod.rs` 想再往下压，下一刀该动的是文件里那约 1,131 行共用辅助函数与运行时系统
  （`推进运行时` / `轮询构建` / `绘制编辑器界面` / `启动项目运行` 等），
  而不是继续搬分支；这一刀要同时改 `plugin.rs` 的引用路径，单独排期。

---

## 8. 扩展读取记录格式

```text
扩展读取：<文件> —— 原因：<为什么清单不够>
```

“实际读取清单”写在回复的「🛠️ 2. 具体实施」下；没有扩展读取则写“无”。

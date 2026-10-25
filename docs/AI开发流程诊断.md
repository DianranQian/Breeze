# 微风（Breeze）AI 开发流程诊断报告

> 性质：**只读诊断**。本文不含任何代码改动；所有结论附文件路径。
> 基线：`8afeeff`（2026-10-25，工作区干净）。
> 取证方式：见附录 A。分析已排除 `.verify*` / `.tmp` / `.bevy_studio` / `target` / `explore`。
> 状态口径沿用 `docs/engine_capability_map.md` §0 的 M0–M5，并映射到用户要求的
> 「已开发 / 半成品 / 未开发 / 废弃 / 待确认」。

---

## A. 诊断结论（10 条）

**先说边界**：这个仓库的工程质量明显高于同类个人项目——1,237 个 `#[test]`、模块头
几乎都写了"为什么这样设计、否决了哪些方案"、`docs/` 有 17 篇可核对的事实清单。
所以下面的问题**不是"代码烂"**，而是"规模长到 10 万行后，原来正确的结构开始收税"。

1. **瓶颈是三个中心文件，不是代码总量。**
   全库 118,163 行 Rust（`src/` 101,271 + `tests/` 15,384，11 个 workspace 成员）。其中
   `editor_app/src/actions/`(5,339：`mod.rs` 2,375 + 12 个域文件 2,964) +
   `editor_ui/src/state.rs`(3,250) + `editor_app/src/plugin.rs`(2,023) = 10,612 行，
   占 9.0%，却承担几乎每一次功能改动。
   AI 每接一个任务都要先啃这三个文件里的一大段。

2. **单一执行器已经瘦身，重心转移到了文件剩余部分。**
   `actions/mod.rs` 的 `执行单个动作` 现为 **615 行**（630–1,244），
   13 个 `&mut` 参数已收进一个 `动作上下文`（2 个参数），函数体只剩 12 个路由跳转 +
   16 条总表分支 + 12 组域哨兵。原来那 2,260 行的"一个函数读到底"已经消解。
   `mod.rs` 现在由三块组成：前部 629 行（快捷键门控、文件对话框回填、动作队列、
   `动作上下文`）、执行器 615 行、后部约 1,131 行被各域共用的辅助函数与运行时系统
   （`推进运行时` / `轮询构建` / `绘制编辑器界面` / `启动项目运行` 等）。
   后两块才是文件体积的大头，下一刀是分流它们，不是继续搬分支。
   新增任何行为仍要先判断有没有对应域文件；只有找不到归属时才加进总表。

3. **动作层与界面层没有编译期联系。**
   `UiAction` 有 182 个变体，其中 166 个（91%）的处理逻辑已在按域拆出的 12 个子文件里。
   历史上三次出现"处理逻辑写完了、界面上却没有入口"
   （`crates/editor_app/tests/action_reachability.rs:1-18` 逐条记录了
   `试听选中实体音频`、`移除最近项目`、`删除动画轨道`）。
   现在靠"扫源码构造点"的测试兜底（剔除分派函数后，每个变体至少 1 个构造点），
   而不是类型系统或注册表。

4. **共享状态是一个 85 字段的单体结构。**
   `界面状态`（`editor_ui/src/state.rs:1200-1474`，274 行，85 个 `pub` 字段）
   被 16 个面板同时 `&mut` 借用。任何面板都能读写任何面板的字段，
   编译器无法告诉你"这次改动影响谁"。

5. **数据依赖是三层同构扇出。**
   `绘制环境`（`editor_app/src/plugin.rs:1616`，14 字段）→
   `绘制上下文`（`editor_ui/src/state.rs:2726`，21 字段）→ 面板。
   新增**一个**数据源（本轮加的 `瓦片图集`）需要改：两个结构体 + `plugin::绘制一帧`
   的构造 + `editor_ui::面板查看器::视图` 的构造 + 资源初始化 = **6 个触点**。

6. **新增一个面板功能的实测扇出 = 18 个触点。**
   瓦片地图这条线改了 16 个手写源码文件（3 个 `editor_core` 新模块、2 个
   `editor_app` 新模块、1 个 `editor_ui` 新面板、10 个既有文件的登记与分发）
   + `messages.tsv` + `tools/gen_messages.py`，再生成 `table.rs`。
   其中 6 处是**必须人工同步的登记表**：`Tab` 枚举与三套键（`docks.rs` 107 处
   `Tab::`）、四个工作区预设（`workspaces.rs` 68 处）、菜单项与映射
   （`menu_bar.rs` 24 处）、面板分发（`editor_ui/src/lib.rs` 44 处）、
   模式默认布局（`dev_modes.rs` 39 处）、`动作摘要`（`plugin.rs`）。

7. **i18n 是全局硬约束，而且是事后报错。**
   `tools/gen_messages.py` 的 `扫描目标` 是 15 个文件的白名单，任何新中文字面量
   都会让脚本以退出码 1 结束。它保证了双语一致，但"改一句界面文案"要动 3 个文件
   （源码 / `messages.tsv` / 生成的 `table.rs`），且错误只在跑脚本或构建时出现。
   当前仍有 4 条历史缺口（`inspector_panel.rs` 3 条、`viewport_panel.rs` 1 条）。

8. **"按文档修订号重建"的约定复制了 6 份，没有抽象。**
   `preview`(1526) / `pie_proto`(1282) / `ui_preview`(491) / `light_preview`(394) /
   `asset_preview`(1002) / `audio_preview`(235)，另有绘制侧的
   `editor_ui/asset_preview`(700)。新加一类可渲染组件时，"该进哪个模块"没有
   类型层面的答案，只能读注释或复制一份重建逻辑。

9. **同名不同层是 AI 误读的最大来源（假设：AI 按文件名检索）。**
   两个 `state.rs`（`editor_core` 权威状态 1,875 行 / `editor_ui` 界面状态 3,250 行）、
   两个 `*_bridge`（`scene_bridge` 场景↔Bevy / `bsn_bridge` BSN↔scene_doc）、
   两个 `asset_preview`（读盘侧 / 绘制侧）、三种"运行"
   （`actions::推进运行时` / `run.rs` 子进程 / `pie_proto` 编辑器内实时）。
   文件名不携带层次信息。

10. **文档开始漂移，而且证据索引已失效。**
    `docs/项目现状总览.md` 自述"停在 `193bf26`、6 个 crate、377 个测试"，
    实际 11 个成员、1,237 个测试；`docs/engine_capability_map.md` §13 的证据索引
    指向 `.verify/...`，而按新的工作区纪律这些目录已删除。

---

## B. 开发地图

状态：**已开发**（有实现 + 有测试/闭环）/ **半成品**（有基础能力但缺口明确）/
**未开发**（grep 与能力地图双重确认无实现）/ **废弃** / **待确认**。
负责人一列全部 **待定**。

| # | 模块 | 功能 | 状态 | 证据文件 |
| --- | --- | --- | --- | --- |
| 1 | 场景文档 `scene_doc` | 实体/组件/属性/变换、RON 原子持久化、损坏回退 | 已开发 | `crates/scene_doc/src/{document,storage,property,path,registry,asset_refs}.rs`；`tests/scene_persistence.rs`(534)；能力地图 C1=M5 |
| 2 | 编辑状态与命令 `editor_core::{state,command,undo,delta,selection,instance}` | 权威状态、撤销/重做、差量合并、复制粘贴/实例化 | 已开发 | `state.rs`(1714)、`command.rs`、`undo.rs`、`delta.rs`；`tests/edit_loop.rs`(840) |
| 3 | 项目管理 `editor_core::{project,cargo_project,cargo_metadata,template,package}` | 新建/打开/保存、资源列表、导入状态、原生 Cargo 工程、模板、打包计划 | 已开发 | `project.rs`(1826)；`tests/project_and_asset_management.rs`(771)、`native_cargo_project.rs`(285) |
| 4 | 资源引用与改名 `editor_core::{asset_index,asset_uid}` + `editor_app::asset_ops` | 跨场景引用、安全删除、改名同步、UID 重连 | 已开发 | `asset_index.rs`、`asset_uid.rs`、`asset_ops.rs`(412)；能力地图 E3–E8=M4 |
| 5 | 编辑器壳层 `editor_ui::{docks,workspaces,workspace_bar,dev_modes,dev_mode_bar}` | 16 面板停靠、4 内置工作区、6 主模式、布局存档 v3 | 已开发 | `docks.rs`(511)、`workspaces.rs`(1550)、`dev_modes.rs`(788)；`tests/workspace_persistence.rs`(178) |
| 6 | 主题与视觉 `editor_ui::{theme,theme_editor,visual_spec,icons,fonts}` | 3 内置 + 用户主题、38 语义槽位、图标烘焙、字体回退 | 已开发 | `theme.rs`(1167)、`theme_editor.rs`(538)、`visual_spec.rs`(380)；`tests/theme_persistence.rs`(143) |
| 7 | 场景编辑面板 `editor_ui::{hierarchy_panel,inspector_panel,viewport_panel,gizmo,gizmo2d,arrange_bar}` | 树、检查器、2D 视口与 gizmo、对齐分布 | 已开发 | `hierarchy_panel.rs`(1114)、`inspector_panel.rs`(1143)、`viewport_panel.rs`(1327)；能力地图 C2–C5 / D1–D2 |
| 8 | 3D 视口 `editor_ui::{viewport3d,gizmo3d,picking3d}` | 轨道相机、网格/轴、屏幕拾取、框选 | 半成品 | `viewport3d.rs`(794)、`gizmo3d.rs`(1146)、`picking3d.rs`(288)；能力地图 D3=M4(部分)，仍非射线求交 |
| 9 | 资源浏览器与预览 `editor_ui::{assets_panel,asset_preview,asset_refs_panel}` + `editor_app::{asset_preview,audio_preview}` | 浏览/搜索/预览/引用查找/音频试听 | 已开发 | `assets_panel.rs`(787)、`asset_preview.rs`(700 / 1000)；`docs/资源预览.md`；能力地图 E1/E10=M4 |
| 10 | 瓦片地图 `editor_core::{tileset,tilemap,terrain}` + `editor_app::{tile_terrain,tilemap_textures}` + `editor_ui::tilemap_panel` | 瓦片集/地图资产、双网格自动瓦片、调色板绘制 | 半成品 | `tileset.rs`(520)、`tilemap.rs`(439)、`tile_terrain.rs`(1083)、`tilemap_panel.rs`(1086)；能力地图 F4=M3（无碰撞/物理接入） |
| 11 | 动画 `editor_core::animation` + `editor_ui::timeline_panel` + `actions::推进动画播放` | 关键帧轨道、三次曲线、时间轴、动画事件 | 已开发 | `animation.rs`(1604)、`timeline_panel.rs`(858)；`tests/animation_events.rs`(297)；能力地图 I1–I3=M4 |
| 12 | 动画状态机 / 混合树 | — | 未开发 | 能力地图 I4=M1；`grep 混合树` **0 命中** |
| 13 | 运行与构建 `editor_app::{run,build,process_tree,runtime_acceptance}` + `editor_core::{build,run_entry,package}` | 构建/运行/停止、进程树清理、打包计划 | 已开发 | `run.rs`(1108)、`build.rs`(557)；`tests/native_run_workbench.rs`(486)、`native_bevy_e2e_acceptance.rs`(316) |
| 14 | 运行时调试 `editor_app::debug_bridge` + `editor_core::runtime_debug` + `runtime_protocol` + `breeze_debug_adapter` | 协议握手、快照、属性写回、历史 | 已开发 | `debug_bridge.rs`(1377)、`runtime_debug.rs`(1295)、`runtime_protocol/lib.rs`(1245)；`tests/runtime_debug_bridge.rs`(645) |
| 15 | Play-in-Editor `editor_app::{pie_proto,pie_input}` | 主 World 内独立渲染层、暂停/单步、输入捕获 | 半成品 | `pie_proto.rs`(1282)、`pie_input.rs`(501)；`tests/pie_lifecycle.rs`(137)；能力地图 Q5=原型 M3/M4，资产加载 M0 |
| 16 | 2D 物理 `editor_app::{physics_runtime,physics_components}` + `editor_core::collision` + `editor_ui::collision_overlay` | Rapier2D 运行态模拟、碰撞轮廓叠加 | 半成品 | `physics_runtime.rs`(427)、`physics_components.rs`(440)；`tests/physics_simulation.rs`(313)；能力地图 J1=M3（仅运行态） |
| 17 | 3D 物理 | — | 未开发 | `Cargo.toml` 仅 `rapier2d`；`grep rapier3d` 0 命中；能力地图 J2=M1 |
| 18 | 游戏 UI `editor_core::ui_layout` + `editor_app::{ui_components,ui_preview}` | 锚点/布局组件、预览投影 | 半成品 | `ui_layout.rs`、`ui_components.rs`(515)、`ui_preview.rs`(491)；能力地图 M1–M4=M3/M2 |
| 19 | 音频 `editor_app::{audio_components,audio_preview}` | 音频组件、编辑器内试听 | 半成品 | `audio_preview.rs`(235)、`audio_components.rs`(202)；能力地图 N1=M3（无波形） |
| 20 | 存档 `savegame` + `editor_ui::savegame_panel` | 槽位/版本/原子写/备份回退、只读浏览器 | 已开发 | `crates/savegame/src/lib.rs`(578)、`savegame_panel.rs`(140)；`tests/save_slots.rs`(127)；能力地图 Y1=M4 |
| 21 | 崩溃恢复 `editor_core::recovery` + `editor_app::recovery_autosave` + `editor_ui::recovery_panel` | 自动保存、恢复点对话框 | 已开发 | `recovery.rs`、`recovery_autosave.rs`(182)、`recovery_panel.rs`(101)；`tests/autosave_recovery.rs`(213) |
| 22 | 诊断中心 `editor_core::diagnostics` + `editor_app::diagnostics` + `editor_ui::build_panel` | 构建/运行/资源/Agent 统一诊断与跳转 | 已开发 | `diagnostics.rs`(1210 / 447)、`build_panel.rs`(384)；能力地图 R1=M4 |
| 23 | 代码编辑 `editor_ui::script` | 语法高亮、查找、保存/另存为 | 已开发 | `script.rs`(701)；能力地图 P1=M4（无 LSP） |
| 24 | AI Agent `editor_ui::agent*` + `editor_app::agent*` | DSH 会话、任务、文件审查、自动修复、worktree | 已开发 | `agent.rs`(1032)、`agent_dsh.rs`(788)、`agent_runtime.rs`(1000)、`agent_repair.rs`(586)；`tests/agent_*.rs` 5 个；能力地图 AA1=M4 |
| 25 | BSN 桥 `bsn_bridge` | 官方 BSN 实例化 → `scene_doc` 单向提取 | 半成品 | `crates/bsn_bridge/src/{capability,extract,host,template}.rs`；`docs/BSN渐进式接入计划.md`；仅 `editor_app/bsn_templates.rs` 6 处引用 |
| 26 | 架构预留 `editor_core::architecture_reserved` | 源码锚点/资源引用行级扫描/运行时通道的数据模型 | 半成品 | `architecture_reserved.rs`；能力地图 W1=M2（无注册式扩展点） |
| 27 | i18n `editor_core::i18n` + `messages.tsv` + `table.rs` + `gen_messages.py` | 中英双语、词表生成、扫描校验 | 已开发 | `messages.tsv`(1330 条)、`table.rs`(1346)、`tools/gen_messages.py`；能力地图 V1=M4，但 4 条历史缺口未清 |
| 28 | GUI 验收 `gui_probe` + `tools/gui_acceptance.ps1` + `editor_app::capture` | 真实窗口截图 + 32 项像素/几何断言 | 已开发 | `gui_probe/src/*.rs`(5648)；`docs/验收证据/`(74 个跟踪文件)；能力地图 R3=M4 |
| 29 | 命令面板 / 快捷键自定义 | — | 未开发 | `grep 命令面板` 0 命中；能力地图 B3/B5=M1（快捷键表是编译期常量） |
| 30 | 粒子 / 寻路 / 导航网格 | — | 未开发 | `grep 粒子/寻路/导航网格` 均 0 命中；能力地图 O1/K1=M1 |
| 31 | 文档 `docs/项目现状总览.md` | 模块事实清单（历史快照） | 废弃 | 文档自述基线 `193bf26`、6 crate、377 测试；实际 11 crate、1,237 测试。已被 `engine_capability_map.md` 取代（其首段自己标注了这一点） |
| 32 | `crates/editor_core/src/terrain.rs`(429) | 旧 `.terrain.ron` 字符网格模型 | 待确认 | 与 `tilemap.rs` 长期并存还是只做迁移兼容，代码里没有写明期限；`tile_terrain.rs` 同时保留两条烘焙路径 |
| 33 | `examples/bevy_debug_demo` | 独立 Cargo 示例工程 | 待确认 | 不在 workspace；`.gitignore` 里仍有 `/examples/bevy_debug_demo/target/` 规则，说明曾被构建过；当前无引用 |
| 34 | `tools/miniruntime`(126) | "运行项目"的最小可执行测试对象 | 待确认 | `Cargo.toml` 注释称它是测试夹具而非示例；未在 docs 中出现在能力地图 |

---

## C. 耦合簇

判据：**同一处改动必须同时打开的文件**（来自本轮瓦片地图改动的实测路径）。
"最小读取集"= 做这个簇的典型任务时，**不读这些就不敢改**的文件。

### 簇 1 · 动作总线
- **模块**：`editor_ui/src/state.rs`（`UiAction` 182 变体 + 动作队列）、
  `editor_app/src/actions/`（`mod.rs` 路由器 + 16 条剩余总表 + 12 组域哨兵；
  12 个域文件 `resources` / `animation` / `edit` / `tilemap` / `theme` / `build` /
  `agent` / `project` / `ui` / `save` / `settings` / `run_debug` 认领 166 个变体）、
  `editor_app/src/plugin.rs`（`动作摘要` 穷尽匹配）、
  `crates/editor_app/tests/action_reachability.rs`（可达性门禁）
- **耦合原因**：一个变体要改 3 处（定义 / 处理 / 摘要），第 4 处（构造点）没有编译期
  约束，只能靠扫描测试。域文件与总表哨兵还各自维护一份变体清单（拆表期已知的双份风险）。
- **建议接口**：已落地——`动作上下文` 收敛 13 个参数；按域拆成
  `fn 处理_域(动作, &mut 上下文) -> Result<(), UiAction>` 一族（返回 `Err(动作)`
  表示"不归我管"，由路由器继续），穷尽性由总表哨兵 + 现有测试保证。
  下一刀不再搬分支：`mod.rs` 里剩下的约 1,120 行共用辅助函数与运行时系统
  （`推进运行时` / `轮询构建` / `绘制编辑器界面` / `启动项目运行` 等）才是文件体积的大头。
- **最小读取集**：`state.rs` 的 `pub enum UiAction` 段 → `actions/` 的目标域文件
  → `plugin.rs` 的 `动作摘要` 对应行 → `tests/action_reachability.rs`。

### 簇 2 · 面板注册
- **模块**：`editor_ui/{docks,workspaces,menu_bar,dev_modes,help}.rs`、
  `editor_ui/src/lib.rs`（分发）、`editor_app/src/plugin.rs`（窗口菜单动作）、
  `editor_app/src/actions/ui.rs`（`聚焦面板` / `浮动面板`）
- **耦合原因**：16 个面板 × 6 张同步表。`Tab::` 在 8 个文件出现 299 次
  （`docks.rs` 107 / `workspaces.rs` 68 / `lib.rs` 44 / `dev_modes.rs` 39 /
  `menu_bar.rs` 24 / `actions/` 12 / `plugin.rs` 4 / `help.rs` 1）。
  漏一处不会编译失败，只会"面板打不开"或"布局恢复后消失"。
- **建议接口**：单一 `面板注册表`：`&'static [面板描述 { tab, 标题键, 菜单键, 图标, 默认分组 }]`，
  `Tab::全部()`、窗口菜单、图标、i18n 校验全部从表派生；
  `match tab { ... }` 分发保留（编译期穷尽有价值），但"登记"只写一处。
- **最小读取集**：`docks.rs`（Tab + 三套键）→ `workspaces.rs` 的 `预设` 段 →
  `menu_bar.rs` 的 `window` 分组 → `editor_ui/src/lib.rs` 的分发 `match` →
  目标面板文件。

### 簇 3 · 共享状态与绘制上下文
- **模块**：`editor_ui/src/state.rs`（`界面状态` 85 字段 + `绘制上下文` 21 字段）、
  `editor_app/src/plugin.rs`（`绘制环境` 14 字段 + `绘制一帧`）、
  `editor_ui/src/lib.rs`（`面板查看器::视图`）
- **耦合原因**：面板拿到的是"整个界面状态"，不是"自己那片"。新增数据源 6 个触点（结论 5）。
  AI 想改一个面板，必须先读 274 行字段定义才能确认"这个字段归谁"。
- **建议接口**：按面板切片——每个面板一个 `XxxState`，`界面状态` 只做聚合；
  `绘制上下文` 改为"只读共享视图 + `&mut 本面板状态`"。先做 2 个面板试点，
  其余保持 `&mut 界面状态` 不动（渐进，不追求一次切完）。
- **最小读取集**：`state.rs` 的 `界面状态` 字段段 + 目标面板的 `XxxState` →
  `plugin.rs` 的 `绘制环境`/`绘制一帧` → `editor_ui/src/lib.rs` 的 `视图`。

### 簇 4 · 渲染投影（文档 → ECS）
- **模块**：`editor_app/{preview,ui_preview,light_preview,tile_terrain,pie_proto,asset_preview}.rs`
  + `scene_bridge.rs`（组件类型注册 + 投影 + 运行时宿主，1,586 行）
- **耦合原因**：6 个模块共享同一条约定"比 `文档修订号`，变了就整体重建"，
  但没有 trait；`scene_bridge` 同时是注册表、投影器和运行时宿主，三种职责共用一个文件。
- **建议接口**：`trait 文档投影 { fn 修订号(&self, 文档) -> u64; fn 重建(&mut self, 世界, 文档); fn 清理(&mut self); }`，
  让 6 个模块的重复约定收敛；`scene_bridge` 三分
  （`组件类型注册` / `预览投影` / `运行时宿主`），对外 API 不变。
- **最小读取集**：目标投影模块 → `scene_bridge.rs` 的 `组件类型` impl 与 `默认注册表`
  → `preview.rs`（相机与渲染目标）→ 对应测试。

### 簇 5 · 组件类型扩展
- **模块**：`editor_app/{scene_bridge,components3d,physics_components,ui_components,audio_components}.rs`
  + `editor_ui/inspector_panel.rs` + `messages.tsv`
- **耦合原因**：新增一个组件 = 1 个 `组件类型` impl + 1 处注册 + N 个 i18n 键
  +（可渲染时）1 处投影接入。trait 本身分得很干净，痛点是"注册"分散在 5 个文件。
- **建议接口**：把 5 处 `.注册(Box::new(...))` 集中成一张 `&'static [Box<dyn 组件类型>]` 表；
  字段描述的 i18n 键约定（`xxx_field_yyy` / `_tooltip`）写进 `docs/`，让 AI 不用猜。
- **最小读取集**：目标组件族模块 → `scene_bridge.rs`（trait + 注册表 + 一个同类 impl 当范例）
  → `inspector_panel.rs`（字段绘制）→ `messages.tsv` 相关段。

### 簇 6 · i18n
- **模块**：`editor_core/i18n/messages.tsv`、`editor_core/src/i18n/table.rs`（生成）、
  `tools/gen_messages.py`、15 个被扫描的源文件
- **耦合原因**：反向扫描 + 白名单。脚本失败是"事后"的，而且失败信息是"这 6 句没有词条"，
  不告诉你去哪加。
- **建议接口**：`扫描目标` 从 15 个文件改成目录级（`crates/editor_ui/src/**`、
  `crates/editor_app/src/scene_bridge.rs`）；加 `--list`（只列不校验）与
  `--fix`（按模板追加空行）两个模式；4 条历史缺口要么补词条要么进 `应排除` 并写理由。
- **最小读取集**：`messages.tsv` 的相关段（按前缀 grep，别整文件读 1,330 行）→
  目标文件 → `tools/gen_messages.py` 的 `扫描目标`/`应排除` 段。

### 簇 7 · 运行 / 构建 / 调试
- **模块**：`editor_app/{run,build,process_tree,debug_bridge,pie_proto,runtime_acceptance}.rs`、
  `editor_core/{build,run_entry,package}.rs`、`runtime_protocol`、`breeze_debug_adapter`、
  `editor_ui/runtime_panel.rs`
- **耦合原因**：跨 6 个 crate，且有"三种运行"（编辑器内预览 / 独立预览子进程 / 真实游戏）
  与"两种调试"（远程协议 / PIE 原型）。进程生命周期、端口、令牌、退出清理互相牵连。
- **建议接口**：保持现有只读快照边界（`运行快照` / `运行时调试快照`），
  把"三种运行"在类型上钉死（`运行目标类型` 已存在，需在 UI 文案与文档里一致）；
  PIE 明确标注为原型，不并入 `run`。
- **最小读取集**：`run.rs`（三张表的那段注释必读）→ `build.rs` → `editor_core/run_entry.rs`
  → `debug_bridge.rs` 的协议段 → `runtime_panel.rs`。**不要**读 `pie_proto.rs`，除非任务涉及 PIE。

### 簇 8 · AI Agent
- **模块**：`editor_ui/{agent,agent_panel,agent_store,agent_repair,agent_review,agent_permission}.rs`、
  `editor_app/{agent_dsh,agent_runtime,agent_context,agent_worktree,agent_persistence}.rs`
- **耦合原因**：11 个文件约 5,000 行。分层是清楚的（界面模型 / 协议 / 进程 / 上下文 / 文件审查），
  但 `agent_repair`(586)、`agent_review`(507)、`agent_permission`(368) 三者的职责边界
  需要读模块头才能分清。
- **建议接口**：**不改结构**。只在 `docs/` 补一张"agent 事件 → 哪个模块处理"的表，
  让 AI 不必读 11 个文件。这是本簇唯一值得做的事。
- **最小读取集**：`agent.rs`（数据模型）→ `agent_dsh.rs`（协议）→
  `agent_runtime.rs`（进程/取消/验证）→ 目标面板。**不要**一次读完 11 个。

### 簇 9 · 场景文档与资产引用
- **模块**：`scene_doc/*`、`editor_core/{asset_index,asset_uid,project}.rs`、`editor_app/asset_ops.rs`
- **耦合原因**：引用扫描横跨"文档 / 项目索引 / 磁盘"，改名要三处一致；
  跨场景改写是直接写盘、不进撤销栈（能力地图 E4 的已知缺口）。
- **建议接口**：已基本收敛在 `scene_doc::asset_refs`（单点扫描），保持；
  只需在 `docs/` 写明"进撤销栈 / 不进撤销栈"的边界，避免 AI 假设可撤销。
- **最小读取集**：`scene_doc/document.rs`（属性结构）→ `asset_refs.rs` →
  `editor_core/asset_index.rs` → `asset_ops.rs`。

---

## D. AI Agent 读取策略

### D.0 三条硬规矩

1. **先读清单，再扩展。** 超出清单的每次读取都要在提交信息/回复里记一行
   `扩展读取：<文件> —— 原因：<为什么清单不够>`。
2. **单任务首轮预算**：≤ 6 个文件、≤ 3,000 行。超预算先说明，不要"顺手全读"。
3. **禁止全库通读**。禁止的目录见每条任务的"禁止"列；全局搜索只允许
   `git grep -n <符号>` 定位，不允许 `Get-Content` 整个大文件。

### D.1 按任务类型的读取清单

| 任务类型 | 必读 | 可选 | 禁止 |
| --- | --- | --- | --- |
| **T1 新增/修改面板** | `editor_ui/src/docks.rs`(Tab+三套键)、`workspaces.rs`(预设)、`menu_bar.rs`(window 分组)、`editor_ui/src/lib.rs`(分发 match)、`state.rs`(目标字段+相关 UiAction)、`plugin.rs`(动作摘要+绘制环境)、`messages.tsv`(相关前缀)、目标面板文件 | `visual_spec.rs`(间距/配色 API)、`theme.rs`(槽位)、`tests/multi_panel_workspace.rs` | 其余 15 个面板；`editor_app/{preview,run,agent*,pie_*,debug_bridge}.rs`；`examples/`；`docs/`（除 `架构设计.md`） |
| **T2 新增 UiAction** | `state.rs`(`pub enum UiAction` 段+动作队列)、`actions/`(`mod.rs` 路由器+目标域子文件)、`plugin.rs`(`动作摘要`)、`tests/action_reachability.rs` | 目标域的既有处理函数、`tests/action_loop.rs` 的同类用例 | `editor_core/*`、`scene_doc/*`（除非动作要读写文档） |
| **T3 新增组件类型** | 目标组件族模块（`scene_bridge` / `components3d` / `physics_components` / `ui_components` / `audio_components`）、`scene_bridge.rs`(trait+注册表+同类 impl 范例)、`inspector_panel.rs`(字段绘制)、`messages.tsv` | 对应投影模块（若可渲染）、`tests/` 同类组件测试 | `actions/mod.rs`（`构造默认组件` 已泛化，不需要改）、`agent*`、`gui_probe` |
| **T4 改渲染/预览** | 目标投影模块、`preview.rs`、`scene_bridge.rs`(投影段)、`state.rs`(相机/视口段)、`viewport_panel.rs` **或** `viewport3d.rs` | `theme.rs`(叠加层配色)、`tests/preview_aspect_ratio.rs` | `agent*`、`assets_panel.rs`、`timeline_panel.rs`、`docs/工作日志.md`(3,075 行) |
| **T5 新增文件格式/资产类型** | `editor_core` 新模块 + `editor_core/src/lib.rs`、`editor_app/actions/`(载入/保存动作，放对应域文件)、`plugin.rs`(资源初始化)、`messages.tsv` | `project.rs`(资源列表/导入状态)、`assets_panel.rs` | `preview.rs`、`pie_proto.rs`、`gui_probe` |
| **T6 改界面文案** | `messages.tsv`(按前缀 grep)、`tools/gen_messages.py`(扫描目标/应排除)、目标文件 | — | 其余一切 |
| **T7 修 bug（有报错/栈）** | 报错指向的文件 + 它直接调用的接口文件 + 覆盖它的测试 | 同簇的第二个文件 | 全库；无关 crate |

### D.2 按耦合簇的"联合读取包"

| 簇 | 联合读取包（一次读完，不要拆散） | 行数量级 |
| --- | --- | --- |
| 1 动作总线 | `state.rs`(UiAction 段 ~600 行) + `actions/`(`mod.rs` 路由器 + 目标域文件 ~200 行) + `plugin.rs`(摘要 ~60 行) | ~900 |
| 2 面板注册 | `docks.rs`(全) + `workspaces.rs`(预设段 ~300) + `menu_bar.rs`(window 段 ~120) + `editor_ui/lib.rs`(分发段 ~80) | ~1,000 |
| 3 共享状态 | `state.rs`(界面状态 274 + 绘制上下文 90) + `plugin.rs`(绘制环境 40 + 绘制一帧 60) | ~470 |
| 4 渲染投影 | 目标投影模块 + `scene_bridge.rs`(trait+一个 impl ~250) + `preview.rs`(相机/渲染目标 ~200) | ~900 |
| 5 组件扩展 | 组件族模块 + `scene_bridge.rs`(trait+注册表 ~200) + `inspector_panel.rs`(字段绘制 ~150) | ~700 |
| 6 i18n | `gen_messages.py`(全 200) + `messages.tsv`(前缀 grep 结果) | ~300 |
| 7 运行调试 | `run.rs`(三表注释+启动 ~300) + `build.rs`(~200) + `run_entry.rs`(~200) | ~700 |
| 8 AI Agent | `agent.rs`(数据模型 ~400) + `agent_dsh.rs`(协议 ~300) + `agent_runtime.rs`(进程 ~300) | ~1,000 |
| 9 资产引用 | `scene_doc/document.rs`(属性 ~300) + `asset_refs.rs`(~200) + `asset_index.rs`(~200) | ~700 |

### D.3 落地方式

在仓库根新增 **`AGENTS.md`**（约 80 行）：把 D.0 三条规矩 + D.1 的表格 + D.2 的簇名
放进去。Agent 进入仓库时会自动读到它，这比任何重构都更快见效，且零风险。

---

## E. 重构与新功能优先级

### E.1 高收益 · 低风险（先做，都是"只加不改"）

| # | 事项 | 收益 | 依据 | 工作量 |
| --- | --- | --- | --- | --- |
| 1 | 新增 `AGENTS.md`（读取清单 + 预算 + 扩展记录） | AI 每次任务少读 5,000–20,000 行 | D 节 | 0.5 天 |
| 2 | 修文档漂移：`项目现状总览.md` 标注基线、`engine_capability_map.md` §13 证据索引改指 `docs/验收证据/` | 去掉 AI 最容易踩的两个错误前提 | 结论 10 | 0.5 天 |
| 3 | `gen_messages.py` 改目录级扫描 + `--list`/`--fix` + 清掉 4 条历史缺口 | 改文案从"跑脚本看报错"变成"一次过" | 结论 7 | 1 天 |
| 4 | 动作处理器收敛 13 个参数为 `动作上下文` 结构体 | 后续所有动作改动都变简单 | 结论 2 | 1 天 |
| 5 | 组件注册集中成一张表（5 处 `.注册` → 1 处） | 新增组件少读 4 个文件 | 簇 5 | 1 天 |
| 6 | 顺手项：`docks.rs:40` 瓦片地图的过期注释、`circle-x.png` 未接入枚举 | 消除误导 | 能力地图 §5/§10 | 0.2 天 |

### E.2 高收益 · 高风险（分步做，每步可回滚）

| # | 事项 | 收益 | 风险 | 缓解 |
| --- | --- | --- | --- | --- |
| 7 | `actions/` 按域拆分（**已完成 12 个域**：资源/动画/编辑/瓦片/主题/构建/智能体/项目/界面/保存/设置/运行调试） | `mod.rs` 4,288 → 2,375；总表只剩 16 条分支；`执行单个动作` 2,260 → 615 行 | 借用冲突、漏改分支、哨兵与域文件双份变体清单 | #4 已完成；沿用 `Result<(), UiAction>` 路由器 + 分组哨兵，每域一个 commit。下一刀不是继续搬分支，而是把 `mod.rs` 里约 1,120 行共用辅助函数与运行时系统分流到各自模块（要同步改 `plugin.rs` 引用路径），另做一轮 |
| 8 | 面板注册表（6 张同步表 → 1 张） | 新增面板从 18 触点降到 ~8 | 布局存档 v3 兼容、4 个预设顺序 | 保留 v1/v2/v3 迁移测试 + 新增"注册表与分发一一对应"测试 |
| 9 | `scene_bridge` 三分（注册 / 投影 / 运行时宿主） | 1,586 行 → 3 个 ≤ 600 | 组件注册顺序、测试断言 | 先纯搬移（不改逻辑），单 commit |
| 10 | 界面状态按面板切片（85 字段） | AI 读状态从 274 行降到 ~30 行 | 16 个面板全动 | **只切 2 个试点**（瓦片地图 + 动画时间轴），其余不动 |
| 11 | 渲染投影 trait 化（6 个 preview） | 新可渲染类型不用复制重建逻辑 | 渲染回归难自动化 | 先只在 2 个模块试点（light_preview + tile_terrain），截图对比 |

### E.3 暂缓（本轮明确不做）

| 事项 | 理由 |
| --- | --- |
| 按层重划 crate（`editor_core` 拆 5 个） | 大爆炸重构；当前 crate 边界（core 无 UI / ui 无 Bevy）已经是对的，问题在文件粒度不在 crate 粒度 |
| 命令面板 / LSP / 多窗口 / 3D 物理 / 粒子 / 动画状态机 | 与"AI 开发效率"无直接关系，属于产品能力；且部分受 Bevy 0.20 限制（能力地图 §2.4） |
| 给 `UiAction` 加宏/代码生成 | 引入新工具链，收益不如 #4 + #7；且 182 个变体的宏会让 IDE 跳转变差 |
| 把 `docs/工作日志.md`(3,075 行) 拆开 | 是历史记录，拆分收益低于成本；只需在 `AGENTS.md` 里标注"除考古不要读" |

---

## F. 两周可执行清单与风险

前提：每天结束工作区干净（新规矩），验证产物只落 `target/verify/`。

### 第 1 周：只加不改（除 #4 外全是新增文件/文档）

| 天 | 目标 | 涉及文件 | 验收标准 | 风险 / 回滚 |
| --- | --- | --- | --- | --- |
| D1 | 落 `AGENTS.md`；修两处文档漂移 | 新增 `AGENTS.md`；`docs/项目现状总览.md`、`docs/engine_capability_map.md`(§13) | 新任务按清单执行一次，记录 1 条扩展原因；`git grep '.verify/'` 不再出现在证据索引 | 纯文档，`git revert` 即回滚 |
| D2 | 诊断确认会：逐簇过一遍，确认优先级 | 本报告 | 用户确认 E.1/E.2 的取舍与 E.3 的暂缓 | 无 |
| D3 | `gen_messages.py` 目录级扫描 + `--list` | `tools/gen_messages.py` | `--list` 输出与当前白名单一致；4 条缺口清零后脚本退出码 0 | 脚本行为变更会影响所有后续文案改动 → 保留 `--strict` 兼容旧行为 |
| D4 | `动作上下文` 结构体（#4） | `editor_app/src/actions/mod.rs`（只加 struct + 改 `执行单个动作` 签名与内部取值） | `cargo check --workspace --all-targets` 通过；`cargo test -p editor_app` 全绿；`执行单个动作` 参数从 13 降到 3 | 13 个参数全是 `&mut`，borrow 冲突可能一次改不完 → 按参数分批，每批一个 commit |
| D5 | 组件注册集中（#5） | `scene_bridge.rs`、`components3d.rs`、`physics_components.rs`、`ui_components.rs`、`audio_components.rs` | 5 处 `.注册` 变 1 张表；`默认注册表包含全部内置组件` 测试仍绿；新增 1 条"表与注册数一致" | 注册顺序可能影响 Inspector 组件排序 → 断言顺序不变 |

### 第 2 周：小步重构（每步一个 commit，可单独 revert）

| 天 | 目标 | 涉及文件 | 验收标准 | 风险 / 回滚 |
| --- | --- | --- | --- | --- |
| D6 | `actions/` 拆第 1 个域（资源）**已完成** | `actions/mod.rs` → 新增 `actions/resources.rs` | 行为不变；`action_loop.rs`、`project_and_asset_management.rs` 全绿 | 拆分后 `pub` 可见性要放宽 → 用 `pub(super)` / `pub(crate)`，不放 `pub` |
| D7 | 拆第 2、3 个域（动画 / 编辑）**已完成** | 同上（`animation.rs` / `edit.rs`） | 每域文件 ≤ 800 行；`动作摘要` 与 `执行` 仍穷尽（编译器保证） | 三个域同时改容易混 → 一天一个域 |
| D7+ | 域拆分**已全部落地（12 个域）**：瓦片 / 主题 / 构建打包 / 智能体 / 项目 / 界面 / 保存 / 设置 / 运行调试，各一个 commit | `actions/{tilemap,theme,build,agent,project,ui,save,settings,run_debug}.rs`；交互与变换事务并回 `edit.rs` | `mod.rs` 4,288 → 2,375；`执行单个动作` 2,260 → 615 行；166/182 变体有归属；`cargo test -p editor_app` 422 passed / 0 failed | 每域保留分组哨兵，穷尽性由编译器兜底；哨兵与域文件是双份变体清单，"一一对应"检查留到后续版本 |
| D8 | 面板注册表（#8）第 1 步：只建表不改行为 | `docks.rs` 新增 `面板描述` 表；`Tab::全部()` 改为从表派生 | `Tab::全部()` 元素与顺序不变（测试断言）；布局恢复测试全绿 | 表与 `match` 不一致 → 新增"注册表与分发一一对应"测试 |
| D9 | 面板注册表第 2 步：窗口菜单 + 图标从表派生 | `menu_bar.rs`、`icons.rs`、`help.rs` | `窗口菜单映射到正确的面板` 测试绿；GUI 菜单截图 1 张 | 菜单顺序变化会影响 GUI 断言 → 断言按"包含"而非"顺序" |
| D10 | 界面状态切片试点（2 个面板） | `state.rs`、`tilemap_panel.rs`、`timeline_panel.rs`、`plugin.rs` | 两个面板只依赖自己的 `XxxState`；其余面板编译不变；`cargo test -p editor_ui` 全绿 | 与并行工作线冲突概率最高 → 单独分支，先 rebase |

### 风险清单（按概率 × 影响）

| 风险 | 概率 | 影响 | 应对 |
| --- | --- | --- | --- |
| `actions/` 拆分时 borrow 冲突反复 | 中 | 中 | 先做 `动作上下文`；分批改；每域一个 commit。实测一次踩点：唯一会递归回分派入口的动作（`打开最近项目`）在子模块里拿不到 13 份逐字段借用，改为把整条上下文交回 `执行单个动作`，未触发编译错误 |
| 布局存档 v3 兼容被破坏（面板注册表） | 中 | 高（用户布局丢失） | 保留三级迁移测试；表驱动只影响"新布局"，不改序列化格式 |
| 界面状态切片与用户并行工作线冲突 | 高 | 中 | 只切 2 个面板；单独分支；一次一个 |
| 渲染投影 trait 化引入画面回归 | 中 | 高（难自动断言） | 本轮不做；若做，先 light_preview 试点 + `docs/验收证据` 截图对比 |
| 文档再次漂移 | 高 | 中 | 把"更新基线"写进每个里程碑的验收项；`AGENTS.md` 里注明文档基线 |
| `target/verify/` 变成本地垃圾场 | 中 | 低 | 沿用本次纪律：失败只留最新一次，成功即删 |
| 全量测试耗时（1,237 条）拖慢迭代 | 中 | 中 | 单 crate 先跑（`-p editor_app`），合并前跑全量 |

### 需要你确认的 3 件事

1. **`AGENTS.md` 放仓库根可以吗？** 它会被 Agent 自动读到，是本次投入产出比最高的一项；
   如果你们用别的约定（`.cursor/rules`、`CLAUDE.md`），我改成对应文件。
2. **两周清单从哪条开始？** 我的建议是 D1–D5 全做（只加不改），D6–D10 视并行工作线情况
   再定。若你更想先看到"新增功能变快"，则优先 D4 + D6–D7（动作总线）。
3. **`explore/` 与 `style-explorer.html` 怎么处理？** 我已把它们加进 `.gitignore`（本地保留、
   不入库），没有删除。要彻底删掉还是移进 `docs/验收证据/`？

---

## 附录 A · 取证方式（可复现）

| 结论 | 命令 / 方法 |
| --- | --- |
| 代码规模 | 按 crate 汇总 `*.rs` 行数（排除 `target/`） |
| 中心文件 | 按单文件行数排序取前 30 |
| `执行单个动作` 规模 | `git grep -n "^fn \|^pub fn " crates/editor_app/src/actions/mod.rs`（630 → 1,244，即 615 行；拆分前为 619 → 2,881） |
| `UiAction` 变体数 | 解析 `pub enum UiAction` 块的大括号深度，取深度 1 的条目（182；哨兵认领 166 + 总表 16） |
| 界面状态字段数 | 定位 `pub struct 界面状态` 块，数 `^\s{4}pub `（85 / 274 行） |
| 上下文扇出 | 读 `plugin.rs:1609`（`绘制环境`）与 `state.rs:2727`（`绘制上下文`）的字段表 |
| 域认领统计 | 数 `actions/mod.rs` 各哨兵块里的 `UiAction::` 出现（12 组共 166 个），与枚举深度解析的 182 相减得总表 16 |
| 面板扇出 | `git grep -c "Tab::"` 按文件统计（8 个文件 299 处） |
| 实测改动面 | `git show --numstat HEAD~1 -- 'crates/**/*.rs'`（瓦片地图提交） |
| 未开发项 | `git grep -c 粒子/寻路/导航网格/混合树/命令面板/多窗口/材质资产`（均 0 命中）+ 能力地图 M1 |
| 状态判定 | 以 `docs/engine_capability_map.md` 的 M0–M5 为主证据，代码与测试为复核 |
| 测试总量 | 全仓 `#[test]` 计数（1,237） |

**假设（需要时请纠正）**

- 假设 AI 的检索方式是"按文件名 + 关键词 grep"，因此把"同名不同层"列为主要误读源。
  如果实际用的是语义检索/符号索引，结论 9 的权重应下调。
- 假设并行工作线（UI 升级、瓦片地形）已合并，后续只有单人/单线推进。若长期多线并行，
  簇 3 与簇 2 的重构应全部延后（冲突成本会超过收益）。

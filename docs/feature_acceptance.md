# 微风 可视化功能通用验收标准

> 目的：让"做完了"有统一、可复核的定义。任何标 M4 以上的能力都必须满足
> 本文对应条目。本文不替代各功能的专项测试，而是规定**最低证据集**。
>
> 配套：能力成熟度见 [`engine_capability_map.md`](./engine_capability_map.md)；
> 计划见 [`engine_roadmap.md`](./engine_roadmap.md)。

## 0. 五条总则

1. **数据真实**：界面显示的内容必须来自文档/项目/运行时的真实数据，
   不允许示例数据、占位数据或"上次看起来不错"的缓存。
2. **闭环完整**：创建/导入 → 编辑 → 保存 → 重新打开 → 运行/消费 → 诊断 → 调整。
   功能只覆盖其中一段时，如实标 M2/M3。
3. **失败诚实**：失败必须给出可读原因，不得假装成功；超时/断线不得当作成功。
4. **证据可复核**：自动测试 + 真实 GUI/运行时证据都要有路径，能重跑。
5. **边界写清**：不支持的操作要在界面上说清，而不是静默忽略。

## 1. 数据正确性

| 检查 | 要求 | 示例（本轮资源链路） |
| --- | --- | --- |
| 合法输入 | 编辑结果到达真实数据模型 | 改资源路径后文档 `资源表` 与组件内嵌引用都指向新路径 |
| 非法输入 | 被拒绝且数据不变 | 目标路径已存在 / 改扩展名 / 目录穿越 → `Err`，文件与文档不动 |
| 缺失数据 | 有明确诊断 | 引用资源不存在 → 使用报告列出 |
| 多义数据 | 不静默猜测 | 同一资源被多个场景引用 → 全部列出 |

## 2. 保存与重新打开

- 修改后保存 → 关闭 → 重新打开，状态一致（场景：`场景持久化` 测试）。
- 跨文件操作要么全成功，要么回滚到原文（示例：`asset_ops` 写回失败回滚）。
- 旧格式迁移、未知字段、损坏文件都有测试或明确说明。

## 3. 运行时真实性

- 影响游戏行为的功能必须启动真实进程或用真实运行时世界验证。
- 检查器里的数值 ≠ 运行时生效；需要运行时调试桥/脚本断言证明。
- 本轮资源链路不改变运行时行为，因此以"磁盘 + 文档一致性"为验收对象；
  引用删错会在下次打开场景时暴露，测试必须覆盖这一点。

## 4. 撤销 / 重做 / 恢复

| 场景 | 要求 |
| --- | --- |
| 单步编辑 | 一次操作恰好一条撤销记录 |
| 连续编辑（拖动滑块） | 合并成一条；撤销回到拖拽前 |
| 失败操作 | 不产生撤销记录，数据回到操作前 |
| 跨文件操作 | 明确说明哪些部分可撤销（本轮：当前文档可撤销，其它场景直接写盘） |

## 5. 跨模块集成

- 场景、资源、检查器、视口、运行、诊断、AI 上下文使用**同一份**数据。
- 资源索引必须同时看见"当前内存文档（含未保存修改）"和"磁盘上的其它场景"。
- 新增 `UiAction` 必须有构造点（`crates/editor_app/tests/动作可达性.rs`）。

## 6. GUI 验收

- 使用 `crates/gui_probe` 启动真实应用，从渲染目标截图并做像素/几何断言。
- 至少覆盖：目标界面出现、关键区域有内容、无异常空白/裁切、
  文案语言与主题符合预期。
- 模态对话框必须用**真实状态**截图（可用环境变量入口驱动，
  如 `BEVY_STUDIO_OPEN_ASSET_DIALOG=rename|delete`），
  不允许为了截图构造假数据。
- 判定：采集失败（3）与界面缺陷（2）必须区分，不能混为一谈。

## 7. 性能与规模

- 对资源多、引用多的项目做一次合理规模验证（例如引用扫描 1000 实体量级）。
- 不得在每帧做全项目扫盘：资源索引按需构建，面板只读结果。
- 记录已知复杂度（例如 `文档资源引用::扫描` 为 O(组件字段数)）。

## 8. 失败与边界

| 边界 | 期望行为 |
| --- | --- |
| 没有打开项目 | 动作给出警告，不 panic |
| 场景文件损坏 | 该场景计入"无法读取"，删除守卫拒绝 |
| 目标资源不存在 | 返回明确错误，界面显示 |
| 只读/占用文件 | 返回系统错误文本，不回滚已成功部分以外的内容 |
| 功能未实现 | 菜单项禁用并说明，不提供假入口 |

## 9. 本轮（R0）验收清单

| 项 | 命令/证据 | 结果 |
| --- | --- | --- |
| 单元测试 | `cargo test -p scene_doc asset_refs` | 4 通过 |
| 单元测试 | `cargo test -p editor_core asset_index` | 6 通过 |
| 单元测试 | `cargo test -p editor_app --lib asset_ops` | 5 通过 |
| 集成测试 | `cargo test -p editor_app --test asset_ops_loop` | 5 通过 |
| 全量 | `cargo test --workspace` | 1048 通过 / 0 失败（`.verify/asset_ops/baseline_tests.log`） |
| GUI 回归 | `powershell -File tools/gui_acceptance.ps1 -Filter assets` | 1 通过 |
| GUI 对话框 | `.verify/asset_ops/delete_dialog/`、`.verify/asset_ops/rename_dialog/` | probe 通过（14 断言） |
| 代码风格 | `cargo clippy -p scene_doc -p editor_core -p editor_app -p editor_ui --all-targets` | 本轮新增代码 0 警告（既有告警另见报告） |
| 词表 | `python tools/gen_messages.py --check` | 本轮新增键全部就位；仍有 5 条来自并行 UI 工作线未提交文案 |
| R1 单元 | `cargo test -p editor_core --lib asset_uid` | 7 通过 |
| R1 项目层 | `cargo test -p editor_core --lib project::测试` | 含 UID 同步/写盘/回连/冲突 4 条 |
| R1 应用层 | `cargo test -p editor_app --lib asset_ops` | 7 通过（新增重命名保 UID、删除移除 UID） |
| R1 集成 | `cargo test -p editor_app --test asset_uid_reconnect` | 3 通过 |
| R1 全量 | `cargo test --workspace` | 1064 通过 / 0 失败（`.verify/asset_ops/uid_tests.log`） |
| R1 GUI | `.verify/asset_ops/uid_dialog/` | 真实打开示例副本，生成 `.breeze/asset_uids.ron`，删除对话框仍通过 14 条断言 |
| R1.5 单元 | `cargo test -p editor_core --lib recovery` | 7 通过 |
| R1.5 集成 | `cargo test -p editor_app --test autosave_recovery` | 6 通过（含"保存后恢复点必须消失"） |
| R1.5 全量 | `cargo test --workspace` | 1077 通过 / 0 失败（`.verify/recovery/all_tests.log`） |
| R1.5 GUI | `.verify/recovery/dialog/` | 真实恢复点 + 对话框，probe 14 条断言通过 |
| R3 单元 | `cargo test -p editor_core --lib animation` | 32 通过（含 6 条事件语义） |
| R3 集成 | `cargo test -p editor_app --test animation_events` | 5 通过 |
| R3 全量 | `cargo test --workspace` | 1088 通过 / 0 失败（`.verify/anim_events/all_tests.log`） |
| R3 GUI | `.verify/anim_events/timeline_clean/` | 时间轴事件行两个标记，probe 14 条断言通过 |
| R4 集成 | `cargo test -p editor_app --test asset_reference_search` | 5 通过 |
| R4 全量 | `cargo test --workspace` | 1093 通过 / 0 失败（`.verify/find_refs/all_tests.log`） |
| R4 GUI | `.verify/find_refs/dialog/` | 真实引用位置 + 定位按钮，probe 14 条断言通过 |
| R5 单元 | `cargo test -p editor_ui --lib picking3d` | 8 通过 |
| R5 全量 | `cargo test --workspace` | 1101 通过 / 0 失败（`.verify/multi_select/all_tests.log`） |
| R5 GUI | `.verify/multi_select/3d_demo/` | 3D 模式选中 3 个实体，场景树/检查器/视口一致 |
| R6 单元 | `cargo test -p editor_ui --lib picking3d` | 11 通过 |
| R6 真实指针 | `gui_probe drag`（`.verify/box_select/drag3/`） | 逻辑 (650,380)→(860,520)，选择数 0→3，日志"执行选择动作 设为(3 项)" |
| R6 全量 | `cargo test --workspace` | 1104 通过 / 0 失败（`.verify/box_select/all_tests.log`） |
| R7 单元 | `cargo test -p editor_core --lib arrange` | 8 通过 |
| R7 集成 | `cargo test -p editor_app --test align_distribute` | 4 通过（含"已对齐不产生撤销步"） |
| R7 全量 | `cargo test --workspace` | 1116 通过 / 0 失败（`.verify/arrange/all_tests.log`） |
| R7 GUI | `.verify/arrange/bar3/` | 2D 多选 3 个实体时视口右上出现对齐条（浮层），probe 14 条断言通过 |
| R8 单元 | `cargo test -p editor_core --lib instance` | 13 通过（含刷新保留位置/拒绝非实例/拒绝空源） |
| R8 集成 | `cargo test -p editor_app --test scene_instance_refresh` | 3 通过（带入新内容、一步撤销、拒绝非实例） |
| R8 全量 | `cargo test --workspace` | 1122 通过 / 0 失败（`.verify/instance_refresh/all_tests.log`） |
| R8 GUI | `.verify/instance_refresh/gui/` | 真实刷新：名称「门口宝箱」与位置 (120,60) 保留，来源组件可见 |
| R9 单元 | `cargo test -p breeze_debug_adapter 资产统计` | 3 通过（真实 Assets 夹具） |
| R9 协议 | `cargo test -p runtime_protocol 资产` | 2 通过（旧格式缺字段兼容、往返） |
| R9 E2E | `cargo test -p editor_app --test runtime_debug_bridge -- --ignored` | 1 通过（真实握手；demo 无资产时统计为空） |
| R9 全量 | `cargo test --workspace` | 1127 通过 / 0 失败（`.verify/runtime_assets/all_tests.log`） |
| R10 集成 | `cargo test -p editor_app --test animation_events` | 7 通过（新增改名+撤销、拒绝空名/失效下标） |
| R10 全量 | `cargo test --workspace` | 1129 通过 / 0 失败（`.verify/anim_event_edit/all_tests.log`） |
| R10 GUI | `.verify/anim_event_edit/dialog2/` | 事件编辑区可见（类型/名称/参数/时间），probe 14 条断言通过 |
| R11 集成 | `cargo test -p editor_core --test scene_uid_completion` | 3 通过（打开标脏、保存后重开不脏、无资源不脏） |
| R11 全量 | `cargo test --workspace` | 1132 通过 / 0 失败（`.verify/uid_on_open/all_tests.log`） |
| R11 GUI | `.verify/uid_on_open/gui3/` | 控制台"已为 12 个资源补全稳定标识"，状态栏未保存 |

## 10. 最终回归结果（R0–R11 收尾）

本轮收尾跑了一遍完整门禁，并如实记录**哪些失败不属于本轮**。

| 项 | 命令 | 结果 |
| --- | --- | --- |
| 格式 | `cargo fmt --all -- --check` | 通过（本轮自有文件已格式化；共享文件只格式化本轮 hunk 所在区域） |
| 全量测试 | `cargo test --workspace` | **1132 通过 / 0 失败**（`.verify/final/all_tests.log`） |
| 词表 | `python tools/gen_messages.py --check` | **6 → 5 条失败**；本轮新增键全部就位，剩余 5 条全部来自并行 UI 工作线未提交文案（`inspector_panel.rs` / `viewport_panel.rs` / `视觉规范` 模块名） |
| 图标 | `cargo run -p icon_bake -- --check` | 通过（62 个图标，32×32） |
| 全量 clippy | `cargo clippy --workspace --all-targets` | 仅剩既有告警：`bsn_bridge` 的 `impl Default` 可派生（来自并行线未提交文件）、`runtime_acceptance` 的 `len == 1`、BSN 测试的多余借用 |
| GUI 回归 | `powershell tools/gui_acceptance.ps1 -Filter assets` | 通过（1 场景，14 条断言） |

### 本轮修掉的一处 i18n 真实缺陷

`查找引用行::文本()` 里把「(已删除实体)」硬编码成中文——英文界面会冒出一行中文。
现在改为走词表（`asset_find_refs_deleted_entity`），并让函数接收语言参数。
扫描脚本因此少了一条失败（6 → 5）。

## 11. R12 游戏存档体系

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 单元 | `cargo test -p savegame` | 9 通过（往返、倒序列表、写入失败不毁旧档、备份恢复、版本过新、槽位名、删除、迁移、编码往返） |
| 集成 | `cargo test -p editor_app --test save_slots` | 3 通过（扫描+损坏标记、无项目、刷新看到新槽位） |
| 全量 | `cargo test --workspace` | **1144 通过 / 0 失败**（`.verify/savegame/all_tests.log`） |
| 端到端（真实进程） | `savegame_demo` 六个场景 | 全部 exit 0（`.verify/savegame/demo_run.log`）：init / play / load（读回初始态）/ verify（两槽互不覆盖）/ corrupt（备份恢复）/ future（拒绝新版本且不删文件） |
| 静态检查 | `cargo clippy -p savegame -p editor_ui -p editor_app --all-targets` | 无新增告警 |
| GUI | `gui_probe run` + `BEVY_STUDIO_OPEN_SAVES=1` | 存档浏览器显示 slot_2/slot_1 的预览、格式/数据版本、保存时间、游玩时长，并把 `slot_broken.sav` 列为无法读取（`.verify/savegame/gui2/`） |

## 12. R13 Play-in-Editor 最小实时渲染实验

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 单元 | `cargo test -p editor_app --lib pie_proto` | 5 通过（撞墙掉头、指针优先、键盘覆盖、dt 夹取、加速） |
| 集成 | `cargo test -p editor_app --test pie_prototype` | 3 通过（生成相机/地面/方块且相机渲染到 Image、停止清空、三次启停计数递增、文档零污染） |
| 全量 | `cargo test --workspace` | **1152 通过 / 0 失败**（`.verify/pie/all_tests.log`） |
| 实时（连续帧） | `gui_probe run --frame 90/240/300` 同会话日志 | 方块 x 逐 30 帧变化：0.05 → 4.55 → 3.00 → … |
| 画面变化（像素） | `.verify/pie/frame_diff.txt` | 帧 90 vs 帧 300，PIE 区域 **1.570%** 像素不同 |
| 输入（真实鼠标） | `gui_probe drag --from 886,467 --to 1140,467` | 日志 `指针 Some(0.5097)`、`输入驱动 48 帧`，方块 x −1.499 → 3.000 |
| 启停/重启 | `BEVY_STUDIO_PIE_SCENARIO=restart` | `已停止（120 帧）` → `已启动（第 2 次）` → 继续推进到 240 帧 |
| 编辑态隔离 | 三次 GUI 证据 json | `document_dirty=false`、`实体 0`、场景文件路径不变 |

## 13. R13.1 PIE 输入捕获与焦点路由

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 单元 | `cargo test -p editor_app --lib pie_input` | 11 通过 |
| 集成 | `cargo test -p editor_app --test pie_input` | 6 通过（路由驱动步进、未捕获全空、Esc 释放、停止清空、重启不继承、真实 `ButtonInput` → 路由） |
| 全量 | `cargo test --workspace` | **1169 通过 / 0 失败**（`.verify/pie_input/all_tests.log`） |
| GUI 点击捕获 | `gui_probe run --pie-keys "click=886,467;..."` | 真实鼠标点击后日志 `PIE 输入焦点：已捕获` |
| GUI 键盘路由 | 同上 | 帧 120/150 `键盘 左=true`（按住 A，方块到 -6 边界），帧 180 `键盘 右=true`（切到 D） |
| GUI Esc 释放 | 同上 | `原因 用户按 Esc` → `未捕获`，后续帧 `键盘 左=false 右=false` |
| GUI 快捷键门控 | `.verify/pie_input/shortcut/` | 捕获期 Ctrl+F1：日志 `PIE 捕获中：编辑器快捷键已停用`，未出现 `float_help` |
| GUI 快捷键恢复 | `.verify/pie_input/after_release/` | Esc 后 F1：证据区域出现 `float_help` |
| GUI 停止/重启 | `.verify/pie_input/stop_restart/` | 已捕获 → 停止 → `无会话` → 重启 → `会话 2 · 未捕获` |
| 方向键 | 同 GUI 键盘路由 | **未通过**：VK 0x25/0x27 注入后 Bevy 未收到（补扩展键标志仍无效）；WASD 已验证 |
| 真实 Alt+Tab 失焦 | — | **未执行**（前台锁）；由单元测试覆盖 |

## 14. R13.2 PIE 运行生命周期（暂停/继续/单步）

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 单元 | `cargo test -p editor_app --lib pie_proto` | 14 通过 |
| 集成 | `cargo test -p editor_app --test pie_lifecycle` | 5 通过（真实 `World` 跑 `推进原型`） |
| 全量 | `cargo test --workspace` | **1183 通过 / 0 失败**（`.verify/pie_lifecycle/all_tests.log`） |
| GUI 暂停 | `BEVY_STUDIO_PIE_SCENARIO=lifecycle` | `[生命周期] 第 90 帧：暂停`；此后 PIE 帧号停在 90，方块 x 不变 |
| GUI 单步 | 同上 | `PIE 单步执行 #1（帧 89 → 90）`、`#2（帧 90 → 91）`；两次请求 = 两次执行 |
| GUI 继续 | 同上 | `[生命周期] 第 180 帧：继续`；帧号继续增长到 172（截图） |
| GUI 编辑器可用 | 同上截图 | 暂停/单步期间菜单、场景树、检查器、资源面板照常显示 |
| 时间缩放 | — | **未实现**（明确不纳入本轮，理由见路线图 R13.2） |
| 按钮点击注入 | — | **未执行**：PIE 面板控件矩形不进入区域证据；按钮可用性由截图目视确认 |

## 15. R14 紧急修复：真实原生 Bevy 项目运行（构建退出清理误杀）

真实用户项目：`C:\Spaces\Share\10_Code\10_Bevy\修仙冒险游戏`（单包、
`src/main.rs` → bin `xiuxian_adventure`、Bevy 0.20、中文目录、项目本地 `target/`，
**不是 workspace**）。

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 对照：直接 Cargo | 项目根 `cargo build` / 直接运行 `target\debug\xiuxian_adventure.exe` | 构建退出码 0（0.5s）；进程存活，stderr 出现 `[动画探针]` |
| 对照：编辑器（修复前） | `BEVY_STUDIO_OPEN_PROJECT=… BEVY_STUDIO_BUILD_AND_RUN=1` | 只有「开始构建」；25 秒无产物、无成功/失败；系统里没有 `cargo.exe`；运行面板空闲 |
| 根因定位 | 在 `Last` 调度打印停止来源 | `停止来源 = 退出时停止构建`；构建状态由「运行中」→「已取消」 |
| 修复 | `退出时停止构建` 读取 `MessageReader<AppExit>` | 未收到退出消息时直接返回；与 `退出时停止项目运行` 语义一致 |
| 单元回归（新增） | `cargo test -p editor_app --lib build::` | 8 通过；含「没有退出消息时构建不会被最后一帧取消」「收到退出消息时构建会被停止」 |
| 集成回归 | `cargo test -p editor_app --test native_build_and_run` 等 | 见 `.verify_r14/` 日志 |
| 全量 | `cargo test --workspace` | 见 `.verify_r14/workspace_test3.log` |
| 真实 GUI（第 1 次） | 打开项目 → 「构建并运行」 | `构建成功，用时 0.9 秒` → `产物：…\target\debug\xiuxian_adventure.exe` → `运行目标：Cargo 可执行目标 xiuxian_adventure::xiuxian_adventure` → 游戏 `[动画探针]` / `[心跳]` 进入运行面板；截图 `.verify_r14/` |
| 真实 GUI（停止/退出） | 采集结束触发编辑器退出 | 退出后 `tasklist` 无 `xiuxian_adventure.exe` / `cargo.exe` 残留 |
| 真实 GUI（第 2 次） | 再次「构建并运行」 | 新 PID（首次 12632 → 第二次 10056），未继承旧进程状态；退出后同样无残留 |
| 目标项目未被修改 | 项目根 `git status --short` | 仍只有本轮之前就存在的 ` M Cargo.toml`（mtime 早于本轮） |
| 环境限制（如实记录） | 本会话沙箱对部分可执行文件的写入范围有限制 | `cargo test` 的 `%TEMP%` 写入被拒（os error 5），因此回归时用仓库外 `CARGO_TARGET_DIR` 运行测试；**不影响产品代码与游戏本地 `target/`** |

## 16. R15 资源浏览器与资产预览（单击即可看到真实内容）

验收项目：`.verify_r15/project`（资源全部是真实项目文件的逐字节副本，
来源与哈希见 `.verify_r15/SOURCES.md` 与报告里的 `asset_hashes`）；
场景预览另在真实示例项目 `examples/3d_demo` 上直接验证。

| 类别 | 命令 / 步骤 | 结果 |
| --- | --- | --- |
| 自动化：状态机 | `cargo test -p editor_ui --lib`（资源预览 7 条） | 325 通过 / 0 失败（含既有用例） |
| 自动化：预览计算 | `cargo test -p editor_app --test asset_preview` | 14 通过 / 0 失败 |
| 自动化：WAV 头 | `cargo test -p editor_app --lib asset_preview` | 3 通过 / 0 失败 |
| 真实 GUI（可重跑） | `bash tools/asset_preview_acceptance.sh` | **9/9 通过**；报告 `docs/验收证据/68_资源预览_验收报告.json` |
| 图片 | 真实单击 `floor.png` → `wall.png` | `ready` / `image` / `has_surface=true` / `256×256` 与磁盘一致；截图 60 |
| 模型 | 真实单击 `Character_Soldier.gltf`（2.2MB） | `ready` / `model` / 网格 60 / 有画面（独立层渲染）；截图 61 |
| 场景 | 真实示例项目单击 `scenes/tps_arena.scene.ron` | `ready` / `scene` / 36 实体；截图 62 |
| 音频 | 单击 `beep.wav` → 真实点击预览里的「试听」 | `audio_playable=true`；点击后 `auditioning=assets/beep.wav`；截图 63 |
| 字体 | 单击 `NotoSansSC-Subset.ttf` | 家族名 `Noto Sans SC`（来自 ttf 表）/ 7866 字形 / 真实字体渲染样例；截图 64 |
| 不支持 | 单击 `unknown.bin` | `unsupported` + 可读说明；截图 65 |
| 快速切换 | 依次单击 `floor.png` → `Crate.gltf` → `wall.png` | 最终 `assets/wall.png`，代次 3，尺寸正确；截图 66 |
| 未打开项目 | 不传项目启动 | `idle` / 无选中 / 0 资源行；截图 67 |
| 只读性 | 预览六类资源后比对场景哈希 | 验收项目场景与真实示例项目**逐字节一致** |
| 生命周期（集成） | `关闭项目清空资源预览与试听状态` 等 | 切换/关闭项目清空选中与预览、试听状态；删除/重命名跟随或失效 |

**环境限制（如实记录，不属于产品缺陷）**：本机沙箱子进程只在工作区根与
`target/` 下可写，`%TEMP%` 与工作区子目录写入被拒。依赖 `tempfile` 的既有测试
需要 `TEMP=<workspace>/target/tmp TMP=<workspace>/target/tmp`；
`cargo test --workspace` 仍有 3 条既有失败，原因全部是"临时目录落在仓库内被
cargo 当成工作区成员"，与本轮改动无关（清单见 `docs/验收证据/69_资源预览_自动化测试日志.log`）。

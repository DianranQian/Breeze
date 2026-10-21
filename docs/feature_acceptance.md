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
| 集成测试 | `cargo test -p editor_app --test 资源操作闭环` | 5 通过 |
| 全量 | `cargo test --workspace` | 1048 通过 / 0 失败（`.verify/asset_ops/baseline_tests.log`） |
| GUI 回归 | `powershell -File tools/gui_acceptance.ps1 -Filter assets` | 1 通过 |
| GUI 对话框 | `.verify/asset_ops/delete_dialog/`、`.verify/asset_ops/rename_dialog/` | probe 通过（14 断言） |
| 代码风格 | `cargo clippy -p scene_doc -p editor_core -p editor_app -p editor_ui --all-targets` | 本轮新增代码 0 警告（既有告警另见报告） |
| 词表 | `python tools/gen_messages.py --check` | 本轮新增键全部就位；仍有 5 条来自并行 UI 工作线未提交文案 |
| R1 单元 | `cargo test -p editor_core --lib asset_uid` | 7 通过 |
| R1 项目层 | `cargo test -p editor_core --lib project::测试` | 含 UID 同步/写盘/回连/冲突 4 条 |
| R1 应用层 | `cargo test -p editor_app --lib asset_ops` | 7 通过（新增重命名保 UID、删除移除 UID） |
| R1 集成 | `cargo test -p editor_app --test 资源标识重连` | 3 通过 |
| R1 全量 | `cargo test --workspace` | 1064 通过 / 0 失败（`.verify/asset_ops/uid_tests.log`） |
| R1 GUI | `.verify/asset_ops/uid_dialog/` | 真实打开示例副本，生成 `.breeze/asset_uids.ron`，删除对话框仍通过 14 条断言 |
| R1.5 单元 | `cargo test -p editor_core --lib recovery` | 7 通过 |
| R1.5 集成 | `cargo test -p editor_app --test 自动保存恢复` | 6 通过（含"保存后恢复点必须消失"） |
| R1.5 全量 | `cargo test --workspace` | 1077 通过 / 0 失败（`.verify/recovery/all_tests.log`） |
| R1.5 GUI | `.verify/recovery/dialog/` | 真实恢复点 + 对话框，probe 14 条断言通过 |
| R3 单元 | `cargo test -p editor_core --lib animation` | 32 通过（含 6 条事件语义） |
| R3 集成 | `cargo test -p editor_app --test 动画事件` | 5 通过 |
| R3 全量 | `cargo test --workspace` | 1088 通过 / 0 失败（`.verify/anim_events/all_tests.log`） |
| R3 GUI | `.verify/anim_events/timeline_clean/` | 时间轴事件行两个标记，probe 14 条断言通过 |
| R4 集成 | `cargo test -p editor_app --test 资源查找引用` | 5 通过 |
| R4 全量 | `cargo test --workspace` | 1093 通过 / 0 失败（`.verify/find_refs/all_tests.log`） |
| R4 GUI | `.verify/find_refs/dialog/` | 真实引用位置 + 定位按钮，probe 14 条断言通过 |
| R5 单元 | `cargo test -p editor_ui --lib picking3d` | 8 通过 |
| R5 全量 | `cargo test --workspace` | 1101 通过 / 0 失败（`.verify/multi_select/all_tests.log`） |
| R5 GUI | `.verify/multi_select/3d_demo/` | 3D 模式选中 3 个实体，场景树/检查器/视口一致 |
| R6 单元 | `cargo test -p editor_ui --lib picking3d` | 11 通过 |
| R6 真实指针 | `gui_probe drag`（`.verify/box_select/drag3/`） | 逻辑 (650,380)→(860,520)，选择数 0→3，日志"执行选择动作 设为(3 项)" |
| R6 全量 | `cargo test --workspace` | 1104 通过 / 0 失败（`.verify/box_select/all_tests.log`） |
| R7 单元 | `cargo test -p editor_core --lib arrange` | 8 通过 |
| R7 集成 | `cargo test -p editor_app --test 对齐分布` | 4 通过（含"已对齐不产生撤销步"） |
| R7 全量 | `cargo test --workspace` | 1116 通过 / 0 失败（`.verify/arrange/all_tests.log`） |
| R7 GUI | `.verify/arrange/bar3/` | 2D 多选 3 个实体时视口右上出现对齐条（浮层），probe 14 条断言通过 |
| R8 单元 | `cargo test -p editor_core --lib instance` | 13 通过（含刷新保留位置/拒绝非实例/拒绝空源） |
| R8 集成 | `cargo test -p editor_app --test 场景实例刷新` | 3 通过（带入新内容、一步撤销、拒绝非实例） |
| R8 全量 | `cargo test --workspace` | 1122 通过 / 0 失败（`.verify/instance_refresh/all_tests.log`） |
| R8 GUI | `.verify/instance_refresh/gui/` | 真实刷新：名称「门口宝箱」与位置 (120,60) 保留，来源组件可见 |
| R9 单元 | `cargo test -p breeze_debug_adapter 资产统计` | 3 通过（真实 Assets 夹具） |
| R9 协议 | `cargo test -p runtime_protocol 资产` | 2 通过（旧格式缺字段兼容、往返） |
| R9 E2E | `cargo test -p editor_app --test 运行时调试桥 -- --ignored` | 1 通过（真实握手；demo 无资产时统计为空） |
| R9 全量 | `cargo test --workspace` | 1127 通过 / 0 失败（`.verify/runtime_assets/all_tests.log`） |
| R10 集成 | `cargo test -p editor_app --test 动画事件` | 7 通过（新增改名+撤销、拒绝空名/失效下标） |
| R10 全量 | `cargo test --workspace` | 1129 通过 / 0 失败（`.verify/anim_event_edit/all_tests.log`） |
| R10 GUI | `.verify/anim_event_edit/dialog2/` | 事件编辑区可见（类型/名称/参数/时间），probe 14 条断言通过 |
| R11 集成 | `cargo test -p editor_core --test 场景Uid补全` | 3 通过（打开标脏、保存后重开不脏、无资源不脏） |
| R11 全量 | `cargo test --workspace` | 1132 通过 / 0 失败（`.verify/uid_on_open/all_tests.log`） |
| R11 GUI | `.verify/uid_on_open/gui3/` | 控制台"已为 12 个资源补全稳定标识"，状态栏未保存 |

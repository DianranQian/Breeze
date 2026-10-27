// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bevy Studio 的 **BSN 适配层**。
//!
//! # 这个 crate 是什么
//!
//! Bevy 0.20 引入了官方场景系统 BSN（Bevy Scene Notation）：`bsn!` 宏可以在
//! Rust 源码里声明场景，`World::spawn_scene` 把它实例化进真实 ECS。
//! 与此同时，本项目已经有一整套自定义场景文档系统 `scene_doc`（稳定 RON 格式、
//! UUID 身份、撤销重做、未知字段保留）。两者解决的不是同一个问题，
//! 也不该互相取代。
//!
//! 本 crate 是**两者之间唯一被允许存在的桥**，负责三件事：
//!
//! 1. [`capability`]：把"当前 Bevy 版本到底支持什么"写成**可被测试校验的数据**，
//!    而不是散文；
//! 2. [`template`]：用官方 `bsn!` 表达编译期已确定的场景模板，并实例化进真实世界；
//! 3. [`extract`]：把实例化后的真实 ECS 实体提取成 `scene_doc` 文档，
//!    让它们进入编辑器既有的撤销重做、身份与保存管线。
//!
//! # 分层与依赖方向
//!
//! ```text
//!                    ┌──────────────────────────────────────┐
//!                    │  editor_app：编辑器业务、UI、运行时   │
//!                    └───────────────┬──────────────────────┘
//!                                    │ 依赖
//!                    ┌───────────────▼──────────────────────┐
//!                    │  bsn_bridge：BSN 适配层（本 crate）   │
//!                    │  capability / template / extract     │
//!                    └───────┬───────────────────┬──────────┘
//!                            │                   │
//!              ┌─────────────▼──────┐   ┌────────▼──────────┐
//!              │  scene_doc（无 Bevy）│   │  bevy / bevy_scene │
//!              └────────────────────┘   └───────────────────┘
//! ```
//!
//! 依赖是单向的：`scene_doc` 不知道 Bevy 存在，适配层同时认识两者，
//! 编辑器只通过适配层接触 BSN。**界面层不解析 BSN**，
//! 也不会有任何模块把"某种场景文本格式"直接嵌进业务流程。
//!
//! # 支持的方向（只有一个）
//!
//! ```text
//!   bsn! 编译期场景  ──实例化──▶  真实 Bevy World  ──提取──▶  scene_doc 文档
//!                                                              │
//!                                          ──✗ 无法反向──  ◀──┘
//! ```
//!
//! 反向转换（文档 → `bsn!`）在编译期宏机制下不成立，详见 [`extract`] 模块文档。
//! 本 crate **不提供**也不会假装提供这个能力。
//!
//! # 当前官方能力的真相（Bevy 0.20.0）
//!
//! * `bsn!` / `bsn_list!` 宏：**有**；
//! * `World::spawn_scene` / `queue_spawn_scene`：**有**；
//! * `.bsn` 文件资产格式与加载器：**没有**。官方源码原话是
//!   “Bevy does not currently have support for `.bsn` files”，
//!   `bsn!` 语法表里 `:"scene.bsn"` 标注为 “Asset format not yet implemented!”。
//!
//! 因此本 crate 的模板全部是**编译期写死的 Rust 代码**，不读取也不产出任何 `.bsn`
//! 文件。这一点由 [`capability`] 里的测试固定住。

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod capability;
pub mod error;
pub mod extract;
pub mod host;
pub mod template;

pub use capability::{
    按状态筛选, 查能力, 目标Bevy版本, 能力条目, 能力清单, 能力状态, 边界摘要
};
pub use error::BSN错误;
pub use extract::{
    bevy变换转文档, 已知类型, 提取产物, 提取并校验, 提取报告, 提取文档, 提取配置, 未转换组件,
    组件提取器, 默认引擎派生组件,
};
pub use host::{BSN宿主, 在世界上实例化};
pub use template::{
    BSN场景模板, 三层嵌套模板, 内置模板列表, 内置模板说明, 基础关卡模板, 带参数模板, 查模板说明,
    模板说明,
};

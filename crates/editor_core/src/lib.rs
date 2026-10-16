//! Bevy Studio 编辑器核心。
//!
//! # 这一层与 `scene_doc` 的分工
//!
//! `scene_doc` 只回答"场景数据长什么样、怎么安全地存下来"。
//! 本 crate 回答"编辑器怎么操作这些数据"：
//!
//! | 关注点 | 实现 |
//! | --- | --- |
//! | 一次修改如何表达 | [`command::编辑器命令`] |
//! | 修改如何被撤销 | [`undo::撤销栈`] |
//! | 当前选中了谁 | [`selection::选择集`] |
//! | 项目与场景如何组织 | [`project::项目描述`] |
//! | 以上如何组合成可驱动的一体 | [`state::编辑器状态`] |
//!
//! # 与 Bevy 的关系
//!
//! **本 crate 不依赖 Bevy**。这不是洁癖，而是为了让"编辑器逻辑"能
//! 在没有窗口、没有 GPU、没有渲染后端的 CI 环境里被完整测试。
//! 历史上有太多编辑器把逻辑和 UI 混在一起，结果只能靠手点验证。
//!
//! 与 Bevy 世界的映射（实体 ⇄ ECS Entity、资源引用 ⇄ `Handle<T>`）
//! 属于 `editor_app` 的 `scene_bridge` 模块职责。

pub mod command;
pub mod i18n;
pub mod project;
pub mod selection;
pub mod state;
pub mod undo;

pub use command::{命令结果, 编辑器命令};
pub use i18n::{语言, 词表, 词条};
pub use project::{
    清理文件名, 目录名, 资源条目, 资源移动结果, 项目描述, 项目格式版本, 项目清单, 项目清单文件名,
    项目结果, 项目错误,
};
pub use selection::选择集;
pub use state::{场景会话, 日志条目, 日志级别, 编辑器状态};
pub use undo::{撤销条目, 撤销栈, 默认内存预算字节, 默认最大步数};

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

pub mod animation;
pub mod build;
pub mod collision;
pub mod command;
pub mod delta;
pub mod i18n;
pub mod instance;
pub mod package;
pub mod project;
pub mod selection;
pub mod state;
pub mod ui_layout;
pub mod undo;

// 模块名是中文，但 Rust 的模块文件查找只接受 ASCII 文件名，
// 所以这里用 `#[path]` 指向中文文件名 —— 与仓库里"测试文件用中文命名"的
// 习惯保持一致，同时不牺牲源码的可读性。
#[path = "模板.rs"]
pub mod 模板;

pub use animation::{
    关键帧, 关键帧值, 动画会话, 动画属性, 动画播放状态, 动画文档, 动画格式版本, 动画轨道, 时间容差,
};
pub use build::{
    代码位置, 构建快照, 构建报告, 构建状态, 构建诊断, 构建配置, 构建配置类型, 编译事件,
    解析编译事件, 诊断级别,
};
pub use command::{命令结果, 编辑器命令};
pub use delta::文档差量;
pub use i18n::{词条, 词表, 语言};
pub use instance::{实例化为, 实例化结果, 实例来源, 实例标记组件, 来源属性};
pub use package::{
    人类可读字节, 打包报告, 打包条目, 打包计划, 打包配置, 执行打包, 生成打包计划, 默认输出目录,
};
pub use project::{
    清理文件名, 目录名, 资源条目, 资源移动结果, 项目描述, 项目格式版本, 项目清单, 项目清单文件名,
    项目结果, 项目设置, 项目错误,
};
pub use selection::选择集;
pub use state::{场景会话, 日志条目, 日志级别, 编辑器状态};
pub use undo::{
    合并键, 撤销条目, 撤销栈, 默认内存预算字节, 默认合并窗口, 默认最大步数
};
pub use 模板::{
    从根集合发现, 从环境值发现, 发现模板, 复制模板到, 复制结果, 复制选项, 打开已有项目目录,
    模板来源, 模板目录环境变量, 模板错误, 解析资源路径, 项目摘要,
};

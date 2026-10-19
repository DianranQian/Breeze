//! Bevy Studio 编辑器应用。
//!
//! # 模块结构
//!
//! | 模块 | 职责 |
//! | --- | --- |
//! | [`scene_bridge`] | 场景文档 ⇄ Bevy ECS 的单向桥接、组件注册表 |
//! | [`bsn_templates`] | 官方 BSN 模板 → 场景文档的适配入口（实验性接入） |
//! | [`plugin`] | Bevy 应用装配、资源定义、文件对话框线程 |
//! | [`actions`] | 界面意图的执行器（唯一修改编辑器状态的地方） |
//!
//! # 分层依赖方向
//!
//! ```text
//! editor_app  ──>  editor_ui  ──>  editor_core  ──>  scene_doc
//!     │                                                  ▲
//!     └──────────────────────────────────────────────────┘
//!              （scene_bridge 只用 scene_doc + Bevy）
//! ```
//!
//! 依赖是单向的：下层完全不知道上层的存在。因此 `scene_doc` 能被
//! 快速单元测试覆盖，`editor_ui` 能在没有窗口的环境里编译，
//! 而只有 `editor_app` 需要拉起真正的 Bevy 应用。

pub mod actions;
pub mod audio_components;
pub mod audio_preview;
pub mod bsn_templates;
pub mod build;
pub mod capture;
pub mod components3d;
pub mod config;
pub mod light_preview;
pub mod physics_components;
pub mod physics_runtime;
pub mod plugin;
pub mod preview;
pub mod run;
pub mod scene_bridge;
pub mod ui_components;
pub mod ui_preview;

pub use build::构建宿主;
pub use config::{
    布局待保存, 编辑器配置, 编辑器配置资源, 配置文件路径, 配置目录
};
pub use plugin::{
    BevyStudio插件, EditorStateHolder, UiStateHolder, 动作摘要, 文件对话框结果, 文件对话框请求,
    文件对话框通道, 构建快照资源, 绘制一帧, 绘制环境, 缩略图缓存, 诊断资源,
};
pub use preview::{
    编辑器界面相机, 预览实体, 预览来源, 预览渲染目标, 预览相机, 预览相机状态
};
pub use scene_bridge::{
    实体映射表, 构建世界, 构建统计, 运行时宿主, 运行状态, 默认组件注册表
};

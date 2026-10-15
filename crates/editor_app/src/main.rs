//! Bevy Studio 编辑器入口。
//!
//! # 为什么主世界这么"空"
//!
//! 这个 `App` 里几乎没有游戏实体，只有编辑器界面所需的资源。
//! 这不是偷懒，而是架构决定的：**游戏内容不住在主世界里**。
//! 编辑器文档是一份独立的、以 RON 持久化的树状数据；
//! 点"运行"时才用它构建一个**独立的** `World`
//! （见 `scene_bridge::运行时宿主`）。
//!
//! 这样安排的直接结果是：编辑器界面永远不会被游戏逻辑干扰，
//! 游戏运行时也永远改不到编辑器文档。两个世界，两份数据。

use bevy::prelude::*;
use bevy::window::WindowResolution;

use editor_app::BevyStudio插件;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Studio".to_string(),
                // Bevy 0.20 的 `WindowResolution` 用 `new(宽, 高)` 构造，
                // 不再支持从 `(f32, f32)` 元组 `into()`（旧版本 API）。
                resolution: WindowResolution::new(1600, 950),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(BevyStudio插件)
        .run();
}

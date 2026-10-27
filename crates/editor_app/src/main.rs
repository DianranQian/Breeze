// SPDX-License-Identifier: MIT OR Apache-2.0
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

/// 默认窗口尺寸。
///
/// 1600×950 的依据：三栏布局（场景树 + 视口 + 检查器）在这个宽度下
/// 每栏都能拿到可用宽度，底部资源浏览器也还能保持两行以上的显示高度。
/// 更窄的窗口会在 `cargo test` 的布局几何测试里被单独覆盖。
const 默认窗口尺寸: (u32, u32) = (1600, 950);

fn main() {
    // 允许用环境变量覆盖窗口尺寸：不同尺寸下的布局验收必须能自动跑，
    // 否则"窗口变窄时状态栏被挤坏"这类问题只能靠人去拉窗口。
    let (宽, 高) = std::env::var(editor_app::capture::窗口尺寸环境变量)
        .ok()
        .and_then(|值| editor_app::capture::解析窗口尺寸(&值))
        .unwrap_or(默认窗口尺寸);

    App::new()
        .add_plugins(
            DefaultPlugins
                // 编辑器要加载"当前打开项目"里的模型/贴图，而资产根在启动时固定。
                // Bevy 0.20 默认拒绝根目录外的绝对路径（日志：is unapproved），
                // 这里显式放开：路径由编辑器自己拼装，不来自不可信输入。
                .set(bevy::asset::AssetPlugin {
                    unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "微风 (Breeze)".to_string(),
                        // Bevy 0.20 的 `WindowResolution` 用 `new(宽, 高)` 构造，
                        // 不再支持从 `(f32, f32)` 元组 `into()`（旧版本 API）。
                        resolution: WindowResolution::new(宽, 高),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(BevyStudio插件)
        .run();
}

// SPDX-License-Identifier: MIT OR Apache-2.0
//! 最小 Rust 游戏项目：一个自转的立方体。
//!
//! 这个示例演示用户项目的完整形态：
//!
//! 1. `Cargo.toml` 只依赖 `breeze_runtime`（**不写 `bevy`**）；
//! 2. `启动带扩展` 挂上项目自己的 System，其余（窗口、投影、物理、运行时）
//!    由运行时装配；
//! 3. `project.ron` 声明运行目标与构建目标，导出时由它决定打哪个 exe。
//!
//! 场景文件里只有一台 `Camera3d`；立方体由项目代码生成，因此自转逻辑
//! 完全属于用户项目，不依赖编辑器里的任何组件。

use breeze_runtime::prelude::*;

fn main() {
    breeze_runtime::启动带扩展(|应用| {
        应用.add_systems(Startup, 生成立方体)
            .add_systems(Update, 自转);
    });
}

#[derive(Component)]
struct 演示立方体;

/// 生成一颗受光立方体，放在场景相机看向的位置。
fn 生成立方体(
    mut 命令: Commands,
    mut 网格资源: ResMut<Assets<Mesh>>,
    mut 材质资源: ResMut<Assets<StandardMaterial>>,
) {
    命令.spawn((
        演示立方体,
        Mesh3d(网格资源.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(材质资源.add(StandardMaterial {
            base_color: Color::srgb(0.91, 0.51, 0.18),
            perceptual_roughness: 0.45,
            ..default()
        })),
        Transform::from_xyz(0.0, 1.0, 0.0),
    ));
}

/// 每帧绕 Y 轴转一点：帧率无关，基准模式（`--bench`）下也能看出在动。
fn 自转(时间: Res<Time>, mut 查询: Query<&mut Transform, With<演示立方体>>) {
    for mut 变换 in &mut 查询 {
        变换.rotate_y(时间.delta_secs() * 1.2);
    }
}

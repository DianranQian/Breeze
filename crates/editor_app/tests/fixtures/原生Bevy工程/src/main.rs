//! 端到端验收用的**真实**原生 Bevy 工程（不是微风的示例项目）。
//!
//! 它有意做三件事：
//!
//! 1. 用真实的 `bevy::App` + ECS（`Component` / `Query` / `add_systems`），
//!    证明微风能识别并运行一个真的 Bevy 工程，而不是一个空壳 `fn main`；
//! 2. 打印可断言的日志（`fixture-log-ok`）与一条 **panic** 分支
//!    （`--panic`），用于验证"运行日志 → 统一诊断 → 跳源码"；
//! 3. 运行几帧后自己退出，使自动化验收不需要人工关窗口。
//!
//! 崩溃分支的 panic 位置是**真实的源码位置**（`src/main.rs:行:列`），
//! 因此诊断里的文件/行号可以直接和磁盘上的文件对照。

use bevy::prelude::*;

/// 一个真实的 ECS 组件。
#[derive(Component)]
struct 计帧 {
    帧数: u32,
}

/// 验收开关（Bevy 的资源必须是命名类型，不能直接用 `bool`）。
#[derive(Resource)]
struct 验收开关 {
    要崩溃: bool,
}

fn main() {
    let 要崩溃 = std::env::args().any(|参数| 参数 == "--panic");
    println!("fixture-log-ok 启动：panic={要崩溃}");

    App::new()
        .add_plugins(MinimalPlugins)
        .insert_resource(验收开关 { 要崩溃 })
        .add_systems(Startup, 生成实体)
        .add_systems(Update, 推进计帧)
        .run();
}

fn 生成实体(mut 命令: Commands) {
    命令.spawn(计帧 { 帧数: 0 });
    println!("fixture-log-ok 已生成 1 个实体");
}

fn 推进计帧(开关: Res<验收开关>, mut 查询: Query<&mut 计帧>) {
    let Ok(mut 计帧组件) = 查询.single_mut() else {
        return;
    };
    计帧组件.帧数 += 1;
    if 计帧组件.帧数 < 5 {
        return;
    }
    if 开关.要崩溃 {
        panic!("fixture-panic-probe：第 5 帧崩溃");
    }
    println!("fixture-log-ok 完成 5 帧，正常退出");
    std::process::exit(0);
}

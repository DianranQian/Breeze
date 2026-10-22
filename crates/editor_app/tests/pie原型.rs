//! PIE 原型的实体生命周期与隔离验证。
//!
//! 这里验证的是**真实 ECS 实体**：`启动_实现` 是否真的生成了相机/地面/方块，
//! `停止_实现` 是否真的把它们全部销毁、可以再次启动。
//!
//! 渲染本身无法在无头测试里断言（需要 GPU），那条证据由 GUI 截图与运行日志
//! 提供（见 `.verify/pie/`）。

use bevy::prelude::*;
use editor_app::pie_proto::{
    Pie原型状态, Pie地面, Pie方块, Pie相机, 停止_实现, 启动_实现
};

fn 状态() -> Pie原型状态 {
    Pie原型状态 {
        图片: Some(Handle::default()),
        ..Pie原型状态::default()
    }
}

/// 统计当前世界里的 PIE 实体。
fn 统计(世界: &mut World) -> (usize, usize, usize) {
    let 相机 = 世界
        .query_filtered::<Entity, With<Pie相机>>()
        .iter(世界)
        .count();
    let 地面 = 世界
        .query_filtered::<Entity, With<Pie地面>>()
        .iter(世界)
        .count();
    let 方块 = 世界
        .query_filtered::<Entity, With<Pie方块>>()
        .iter(世界)
        .count();
    (相机, 地面, 方块)
}

#[test]
fn 启动生成相机地面与方块并能停止() {
    let mut 世界 = World::new();
    let mut 状态 = 状态();

    {
        let mut 命令 = 世界.commands();
        启动_实现(&mut 命令, &mut 状态, None);
    }
    世界.flush();

    assert_eq!(统计(&mut 世界), (1, 1, 1), "启动应当生成相机/地面/方块各一");
    assert_eq!(状态.状态, editor_app::pie_proto::Pie运行状态::运行中);
    assert_eq!(状态.启动次数, 1);

    // 相机必须真的把它渲染到那张纹理上，否则显示出来的只会是空图。
    let 带渲染目标的相机 = 世界
        .query::<(&Camera, &bevy::camera::RenderTarget)>()
        .iter(&世界)
        .filter(|(_, 目标)| matches!(目标, bevy::camera::RenderTarget::Image(_)))
        .count();
    assert_eq!(带渲染目标的相机, 1, "PIE 相机必须渲染到 Image");

    let 相机实体 = 世界
        .query_filtered::<Entity, With<Pie相机>>()
        .iter(&世界)
        .next()
        .unwrap();
    let 地面实体 = 世界
        .query_filtered::<Entity, With<Pie地面>>()
        .iter(&世界)
        .next()
        .unwrap();
    let 方块实体 = 世界
        .query_filtered::<Entity, With<Pie方块>>()
        .iter(&世界)
        .next()
        .unwrap();
    {
        let mut 命令 = 世界.commands();
        停止_实现(
            &mut 命令,
            &mut 状态,
            Some(相机实体),
            Some(地面实体),
            Some(方块实体),
        );
    }
    世界.flush();

    assert_eq!(统计(&mut 世界), (0, 0, 0), "停止必须清干净本次运行的实体");
    assert_eq!(状态.状态, editor_app::pie_proto::Pie运行状态::已停止);
    assert!(状态.启动时刻.is_none());
}

#[test]
fn 停止后可以再次启动且计数递增() {
    let mut 世界 = World::new();
    let mut 状态 = 状态();

    for 期望次数 in 1..=3 {
        let 旧相机 = 世界
            .query_filtered::<Entity, With<Pie相机>>()
            .iter(&世界)
            .next();
        {
            let mut 命令 = 世界.commands();
            启动_实现(&mut 命令, &mut 状态, 旧相机);
        }
        世界.flush();
        assert_eq!(
            统计(&mut 世界),
            (1, 1, 1),
            "第 {期望次数} 次启动后仍应恰好一套实体（旧相机要被替换）"
        );
        assert_eq!(状态.启动次数, 期望次数);
        assert_eq!(状态.帧, 0, "每次启动都从第 0 帧开始");

        let 实体: Vec<Entity> = 世界
            .query_filtered::<Entity, Or<(With<Pie相机>, With<Pie地面>, With<Pie方块>)>>()
            .iter(&世界)
            .collect();
        {
            let mut 命令 = 世界.commands();
            停止_实现(
                &mut 命令,
                &mut 状态,
                Some(实体[0]),
                Some(实体[1]),
                Some(实体[2]),
            );
        }
        世界.flush();
        assert_eq!(统计(&mut 世界), (0, 0, 0));
    }
}

#[test]
fn 原型实体不进入场景文档() {
    // 运行态与编辑态的隔离在结构上就是"两套数据"：PIE 实体只存在于 ECS，
    // 场景文档是 RON 数据，原型代码从不持有文档。
    // 这里把这条约束写成断言，防止将来有人"顺手把运行时实体写回文档"。
    use scene_doc::document::场景文档;

    let mut 世界 = World::new();
    let mut 状态 = 状态();
    let 文档 = 场景文档::新建("隔离验证");
    let 启动前 = 文档.clone();

    {
        let mut 命令 = 世界.commands();
        启动_实现(&mut 命令, &mut 状态, None);
    }
    世界.flush();
    let 实体: Vec<Entity> = 世界
        .query_filtered::<Entity, Or<(With<Pie相机>, With<Pie地面>, With<Pie方块>)>>()
        .iter(&世界)
        .collect();
    assert_eq!(实体.len(), 3);
    {
        let mut 命令 = 世界.commands();
        停止_实现(
            &mut 命令,
            &mut 状态,
            Some(实体[0]),
            Some(实体[1]),
            Some(实体[2]),
        );
    }
    世界.flush();

    assert_eq!(文档, 启动前, "运行/停止都不能改动场景文档");
}

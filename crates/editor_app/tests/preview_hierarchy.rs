//! 中文名：预览层级
//! 预览父子层级：节点挂接与 `GlobalTransform` 传播的集成测试。
//!
//! # 为什么这组测试存在
//!
//! "预览按世界坐标渲染"曾经是一个错误假设：旧实现把文档里的**局部**变换
//! 直接当成世界坐标写给视觉实体，父实体移动时子实体并不会跟着动。
//! 现在的结构是"每个文档实体一个节点 + 视觉实体挂在节点下"，
//! 世界坐标完全由 `TransformPlugin` 的 `GlobalTransform` 传播产生。
//!
//! 这条链路跨了三层（文档遍历 → 节点接线 → Bevy 调度），
//! 任何一层写错，纯单元测试都发现不了，所以这里跑真实的 `App`。

use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use editor_app::preview::{同步预览世界变换, 构建预览层级, 预览世界变换快照};
use scene_doc::document::{变换数据, 场景文档, 实体标识};

/// 一个只保留变换传播与快照采集的最小应用。
fn 最小应用() -> App {
    let mut 应用 = App::new();
    应用.add_plugins((MinimalPlugins, TransformPlugin));
    应用.init_resource::<预览世界变换快照>();
    应用.add_systems(
        PostUpdate,
        同步预览世界变换.after(bevy::transform::TransformSystems::Propagate),
    );
    应用
}

fn 平移(x: f32, y: f32) -> 变换数据 {
    变换数据 {
        平移: (x, y, 0.0),
        旋转: (0.0, 0.0, 0.0, 1.0),
        缩放: (1.0, 1.0, 1.0),
    }
}

/// 父在原点、子局部 X=100 的两层文档。
fn 造两层文档() -> (场景文档, 实体标识, 实体标识) {
    let mut 文档 = 场景文档::新建("层级");
    let 父 = 文档.生成实体("父", None);
    let 子 = 文档.生成实体("子", Some(父));
    for (标识, 变换) in [(父, 平移(0.0, 0.0)), (子, 平移(100.0, 0.0))] {
        if let Some(实体) = 文档.取可变(标识) {
            实体.变换 = Some(变换);
        }
    }
    (文档, 父, 子)
}

/// 子的 `GlobalTransform` 必须是"父 × 子"的结果，而不是子的局部变换。
#[test]
fn 子实体世界变换累加父变换() {
    let (文档, 父, 子) = 造两层文档();
    let mut 应用 = 最小应用();

    let 节点表 = {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, &文档)
    };
    应用.world_mut().flush();
    应用.update();

    let 父节点 = 节点表[&父];
    let 子节点 = 节点表[&子];

    let 子全局 = 应用
        .world()
        .entity(子节点)
        .get::<GlobalTransform>()
        .expect("节点带 Transform，就必须有传播后的 GlobalTransform");
    assert!(
        (子全局.translation().x - 100.0).abs() < 1e-3,
        "父在原点、子局部 X=100 时世界 X 应为 100，实际 {}",
        子全局.translation().x
    );

    let 父关系 = 应用
        .world()
        .entity(子节点)
        .get::<ChildOf>()
        .expect("子节点必须挂在父节点下");
    assert_eq!(父关系.parent(), 父节点);
    assert!(
        应用.world().entity(父节点).get::<ChildOf>().is_none(),
        "顶层实体的节点不应有父"
    );
}

/// 父节点移动后，子的世界坐标必须跟着变：这是"局部变换不再当世界用"的直接证明。
#[test]
fn 父节点移动后子世界坐标跟随() {
    let (文档, 父, 子) = 造两层文档();
    let mut 应用 = 最小应用();

    let 节点表 = {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, &文档)
    };
    应用.world_mut().flush();
    应用.update();

    let 父节点 = 节点表[&父];
    let 子节点 = 节点表[&子];
    应用
        .world_mut()
        .entity_mut(父节点)
        .insert(Transform::from_translation(Vec3::new(10.0, 20.0, 0.0)));
    应用.update();

    let 子全局 = 应用
        .world()
        .entity(子节点)
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform");
    let 平移 = 子全局.translation();
    assert!(
        (平移.x - 110.0).abs() < 1e-3 && (平移.y - 20.0).abs() < 1e-3,
        "父移动 (10, 20) 后子应落在 (110, 20)，实际 ({}, {})",
        平移.x,
        平移.y
    );
}

/// 快照必须与传播结果同帧一致，否则视口包围框会落后一帧。
#[test]
fn 世界变换快照与传播结果一致() {
    let (文档, 父, 子) = 造两层文档();
    let mut 应用 = 最小应用();

    {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, &文档);
    }
    应用.world_mut().flush();
    应用.update();

    let 快照 = 应用.world().resource::<预览世界变换快照>();
    assert_eq!(
        快照.变换[&父].平移,
        (0.0, 0.0, 0.0),
        "父节点快照应等于其世界变换"
    );
    let 子快照 = 快照.变换[&子].平移;
    assert!(
        (子快照.0 - 100.0).abs() < 1e-3,
        "子节点快照必须来自 GlobalTransform 而不是文档局部变换，实际 {子快照:?}"
    );
}

//! 中文名：世界位移拖拽
//! 父级带旋转/缩放时，世界拖拽增量换算成父级局部位移的端到端验证。
//!
//! # 为什么这组测试存在
//!
//! 预览把父子关系交给 `TransformPlugin` 之后，文档里的 `平移` 只表示
//! 相对父级的位置；视口拖动算出的却是世界增量。父级绕 Z 转 45°、放大
//! 2 倍时，世界 +X 的 100 单位在父级空间里是 `(35.36, -35.36)`。
//! 若直接写成 `(100, 0)`，子实体会朝右上方飞出去。
//!
//! 这里跑真实的 `App`：先用 Bevy 传播得到 `GlobalTransform`，快照抄出
//! 逆矩阵，再走与用户点击同一条动作队列。最后把文档结果重新灌回节点，
//! 断言子实体的**世界**位置恰好是 +X 100 —— 两个方向都闭环。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::preview::{同步预览世界变换, 构建预览层级, 预览世界变换快照};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_ui::state::{UiAction, 选择意图};
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

/// 父节点绕 Z 转 45°、放大 2 倍；子节点在父级原点。
fn 造旋转缩放父级文档() -> (场景文档, 实体标识, 实体标识) {
    let 半角 = std::f32::consts::FRAC_PI_4 * 0.5;
    let 父变换 = 变换数据 {
        平移: (0.0, 0.0, 0.0),
        旋转: (0.0, 0.0, 半角.sin(), 半角.cos()),
        缩放: (2.0, 2.0, 1.0),
    };
    let mut 文档 = 场景文档::新建("换算");
    let 父 = 文档.生成实体("父", None);
    let 子 = 文档.生成实体("子", Some(父));
    for (标识, 变换) in [(父, 父变换), (子, 变换数据::单位())] {
        if let Some(实体) = 文档.取可变(标识) {
            实体.变换 = Some(变换);
        }
    }
    (文档, 父, 子)
}

/// 准备动作队列需要的宿主资源；测试不打开项目，允许项目为空。
fn 装入动作资源(应用: &mut App) {
    let 世界 = 应用.world_mut();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(布局待保存::default());
    世界.insert_resource(项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(诊断中心::default());
    世界.insert_resource(编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
}

fn 推动作(应用: &mut App, 动作: UiAction) {
    应用
        .world_mut()
        .resource_mut::<UiStateHolder>()
        .动作
        .推(动作);
}

fn 执行动作(应用: &mut App) {
    应用
        .world_mut()
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 文档变换(应用: &App, 标识: 实体标识) -> 变换数据 {
    应用
        .world()
        .resource::<EditorStateHolder>()
        .状态
        .会话
        .文档
        .取(标识)
        .and_then(|实体| 实体.变换)
        .expect("变换")
}

/// 拖拽流：选中子实体 → 按下 → 世界 +X 100 → 松手。
#[test]
fn 旋转缩放父级下拖拽写回局部位移而不是世界增量() {
    let (文档, 父, 子) = 造旋转缩放父级文档();
    let mut 应用 = 最小应用();
    let 节点表 = {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, &文档)
    };
    应用.world_mut().flush();
    应用.update();

    装入动作资源(&mut 应用);
    {
        let 持有 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        持有.应用已载入场景(文档, None);
    }

    推动作(&mut 应用, UiAction::选择(选择意图::设为(vec![子])));
    推动作(
        &mut 应用,
        UiAction::开始拖动实体 {
            实体: 子,
            追加选择: false,
            按下世界: (0.0, 0.0),
        },
    );
    推动作(
        &mut 应用,
        UiAction::拖动实体到 {
            世界偏移: (100.0, 0.0),
        },
    );
    推动作(&mut 应用, UiAction::结束拖动);
    执行动作(&mut 应用);

    let 子平移 = 文档变换(&应用, 子).平移;
    assert!(
        (子平移.0 - 35.355_34).abs() < 1e-3 && (子平移.1 + 35.355_34).abs() < 1e-3,
        "世界 +X 100 必须写成局部 (35.36, -35.36)，不能直接加 100；实际 {子平移:?}"
    );
    assert!(子平移.2.abs() < 1e-4);

    let 父平移 = 文档变换(&应用, 父).平移;
    assert_eq!(父平移, (0.0, 0.0, 0.0), "父实体不能被拖拽带着走");

    应用
        .world_mut()
        .entity_mut(节点表[&子])
        .insert(Transform::from_xyz(子平移.0, 子平移.1, 子平移.2));
    应用.update();
    let 世界位置 = 应用
        .world()
        .entity(节点表[&子])
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform")
        .translation();
    assert!(
        (世界位置.x - 100.0).abs() < 1e-3 && 世界位置.y.abs() < 1e-3,
        "局部位移经父级传播后应当恰好是 (100, 0)，实际 ({}, {})",
        世界位置.x,
        世界位置.y
    );
}

/// 父子同时选中：同一世界增量只能让父级写局部平移，子级保持零位移。
#[test]
fn 父子同时选中时子级不重复位移() {
    let (文档, 父, 子) = 造旋转缩放父级文档();
    let mut 应用 = 最小应用();
    {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, &文档);
    }
    应用.world_mut().flush();
    应用.update();

    装入动作资源(&mut 应用);
    {
        let 持有 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        持有.应用已载入场景(文档, None);
    }

    推动作(&mut 应用, UiAction::选择(选择意图::设为(vec![父, 子])));
    推动作(
        &mut 应用,
        UiAction::开始拖动实体 {
            实体: 父,
            追加选择: false,
            按下世界: (0.0, 0.0),
        },
    );
    推动作(
        &mut 应用,
        UiAction::拖动实体到 {
            世界偏移: (100.0, 0.0),
        },
    );
    推动作(&mut 应用, UiAction::结束拖动);
    执行动作(&mut 应用);

    assert_eq!(
        文档变换(&应用, 父).平移,
        (100.0, 0.0, 0.0),
        "根级父实体的世界位移就是局部平移"
    );
    assert_eq!(
        文档变换(&应用, 子).平移,
        (0.0, 0.0, 0.0),
        "子级由父级带动，局部平移必须保持零"
    );
}

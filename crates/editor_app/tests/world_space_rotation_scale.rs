//! 中文名：世界旋转与缩放操纵器
//! 父级带旋转/缩放时，旋转与缩放操纵器的求解结果端到端验证。
//!
//! # 为什么这组测试存在
//!
//! 移动的换算（P5-2）已经闭环，但旋转与缩放还各有一个语义陷阱：
//!
//! * 旋转环绕的是**世界**轴。把世界轴增量直接右乘到局部四元数上，
//!   等价于绕局部轴转；父级转了 90° 时轴就歪了。正确做法是在父级空间
//!   里共轭：`局部新 = P⁻¹ × R(世界轴, θ) × P × 局部旧`。
//! * 缩放手柄沿**世界**轴。父级转 90° 后，拖世界 X 手柄改变的是实体的
//!   局部 Y 缩放；按枚举直接写 x 分量，视觉上就会沿错误的轴伸缩。
//!
//! 这里跑真实的 `App`：Bevy 传播出 `GlobalTransform`，快照抄出世界变换与
//! 逆矩阵，操纵器求解直接用这份快照。最后把结果写回节点再传播一次，
//! 断言**世界**侧的效果与拖拽一致。

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use editor_app::preview::{同步预览世界变换, 构建预览层级, 预览世界变换快照};
use editor_ui::gizmo::{拖拽基准, 操纵器部件};
use editor_ui::state::{三维相机, 视口相机};
use editor_ui::world_space::世界空间;
use editor_ui::{gizmo2d, gizmo3d};
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

fn 绕_z(角: f32) -> (f32, f32, f32, f32) {
    let 半 = 角 * 0.5;
    (0.0, 0.0, 半.sin(), 半.cos())
}

/// 建一个父/子文档（两级变换都给全）。
fn 造场景(
    父变换: 变换数据, 子变换: 变换数据
) -> (场景文档, 实体标识, 实体标识) {
    let mut 文档 = 场景文档::新建("旋转缩放");
    let 父 = 文档.生成实体("父", None);
    let 子 = 文档.生成实体("子", Some(父));
    for (标识, 变换) in [(父, 父变换), (子, 子变换)] {
        if let Some(实体) = 文档.取可变(标识) {
            实体.变换 = Some(变换);
        }
    }
    (文档, 父, 子)
}

/// 建预览层级并跑一轮传播（快照在这一步被填满）。
fn 建预览(应用: &mut App, 文档: &场景文档) -> HashMap<实体标识, Entity> {
    let 节点表 = {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, 文档)
    };
    应用.world_mut().flush();
    应用.update();
    节点表
}

/// 四元数分量是否与期望一致（允许共轭反号）。
fn 同一旋转(a: Quat, b: Quat) -> bool {
    a.angle_between(b).abs() < 1e-3
}

#[test]
fn 父级旋转九十度时子级局部旋转叠加成世界旋转() {
    let (文档, _父, 子) = 造场景(
        变换数据 {
            旋转: 绕_z(std::f32::consts::FRAC_PI_2),
            ..变换数据::单位()
        },
        变换数据 {
            旋转: 绕_z(std::f32::consts::FRAC_PI_4),
            ..变换数据::单位()
        },
    );
    let mut 应用 = 最小应用();
    let 节点表 = 建预览(&mut 应用, &文档);

    let 世界旋转 = 应用
        .world()
        .entity(节点表[&子])
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform")
        .rotation();
    assert!(
        同一旋转(世界旋转, Quat::from_rotation_z(135f32.to_radians())),
        "父级 90° × 子级局部 45° 应得到世界 135°，实际 {世界旋转:?}"
    );

    let 相机 = 视口相机 {
        中心: (0.0, 0.0),
        缩放: 1.0,
        ..视口相机::default()
    };
    let 尺寸 = egui::Vec2::new(800.0, 600.0);
    let 角度 = 30f32.to_radians();
    let 局部指针 = 相机.世界转屏幕((角度.cos() * 50.0, 角度.sin() * 50.0), 尺寸);
    let mut 基准 = 拖拽基准::旋转 {
        上次角: 0.0,
        累计角: 0.0,
    };
    let 起 = [(子, 文档.取(子).and_then(|实体| 实体.变换).unwrap())];
    let 结果 = {
        let 快照 = 应用.world().resource::<预览世界变换快照>();
        let 世界 = 世界空间::新建(&文档, &快照.逆矩阵).带世界变换(&快照.变换);
        gizmo2d::求解(
            操纵器部件::旋转平面,
            &mut 基准,
            (0.0, 0.0),
            局部指针,
            &相机,
            尺寸,
            &起,
            世界,
        )
    }
    .expect("应当求解成功");
    let 局部旋转 = 结果[0].1.旋转;
    assert!(
        同一旋转(
            Quat::from_xyzw(局部旋转.0, 局部旋转.1, 局部旋转.2, 局部旋转.3),
            Quat::from_rotation_z(75f32.to_radians())
        ),
        "世界 Z 再转 30° 应写回局部 45° + 30° = 75°，实际 {局部旋转:?}"
    );

    应用
        .world_mut()
        .entity_mut(节点表[&子])
        .insert(Transform::from_rotation(Quat::from_xyzw(
            局部旋转.0,
            局部旋转.1,
            局部旋转.2,
            局部旋转.3,
        )));
    应用.update();
    let 新世界旋转 = 应用
        .world()
        .entity(节点表[&子])
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform")
        .rotation();
    assert!(
        同一旋转(新世界旋转, Quat::from_rotation_z(165f32.to_radians())),
        "写回后世界旋转应是 90° + 75° = 165°，实际 {新世界旋转:?}"
    );
}

#[test]
fn 非均匀缩放父级下世界_x_缩放改局部_y且世界比例一致() {
    let (文档, _父, 子) = 造场景(
        变换数据 {
            旋转: 绕_z(std::f32::consts::FRAC_PI_2),
            缩放: (2.0, 1.0, 1.0),
            ..变换数据::单位()
        },
        变换数据::单位(),
    );
    let mut 应用 = 最小应用();
    let 节点表 = 建预览(&mut 应用, &文档);

    let 相机 = 三维相机::default();
    let 尺寸 = egui::Vec2::new(800.0, 600.0);
    let 局部指针 = 相机
        .投影((1.5, 0.0, 0.0), 尺寸)
        .expect("世界 X 上的 1.5 应可见");
    let 起 = [(子, 文档.取(子).and_then(|实体| 实体.变换).unwrap())];
    let 结果 = {
        let 快照 = 应用.world().resource::<预览世界变换快照>();
        let 世界 = 世界空间::新建(&文档, &快照.逆矩阵).带世界变换(&快照.变换);
        gizmo3d::求解(
            操纵器部件::缩放X,
            &mut 拖拽基准::轴向 { 起点参数: 1.0 },
            (0.0, 0.0, 0.0),
            局部指针,
            &相机,
            尺寸,
            &起,
            世界,
        )
    }
    .expect("应当求解成功");
    let 缩放 = 结果[0].1.缩放;
    assert!(
        (缩放.1 - 1.5).abs() < 1e-3,
        "父级转 90° 后世界 X 手柄应改局部 Y：{缩放:?}"
    );
    assert!(
        (缩放.0 - 1.0).abs() < 1e-6 && (缩放.2 - 1.0).abs() < 1e-6,
        "未映射到的局部轴必须保持原值：{缩放:?}"
    );

    应用
        .world_mut()
        .entity_mut(节点表[&子])
        .insert(Transform::from_scale(Vec3::new(缩放.0, 缩放.1, 缩放.2)));
    应用.update();
    let 世界点 = 应用
        .world()
        .entity(节点表[&子])
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform")
        .affine()
        .transform_point3(Vec3::new(0.0, 1.0, 0.0));
    assert!(
        (世界点.x + 1.5).abs() < 1e-3 && 世界点.y.abs() < 1e-3,
        "局部 Y 拉长 1.5 倍后，世界 X 上的长度应为 1.5：{世界点:?}"
    );
}

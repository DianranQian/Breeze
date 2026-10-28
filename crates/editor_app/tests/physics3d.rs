//! 中文名：三维物理
//! 三维物理的端到端验收：工具栏创建 → 文档组件 → 运行态模拟 → 写回 → 停止丢弃。
//!
//! # 为什么这些用例必须走真实动作与真实系统
//!
//! 三维物理有四个容易"看起来接上了、实际没通"的接缝：
//!
//! 1. **创建 → 自动挂载**：图元必须有刚体与尺寸匹配的碰撞体；
//! 2. **插件切面**：物理只允许在「运行」后的独立世界推进，预览必须静止；
//! 3. **文档隔离**：物理姿态只写运行时 `Transform`，绝不回写场景文档；
//! 4. **停止清理**：停止后物理世界必须整体丢弃，再次运行从文档重新初始化。
//!
//! 因此这里推真实 `UiAction`、跑真实 `推进运行时`，并同时断言
//! "运行时在动"与"文档没动"。

use std::time::Duration;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::physics3d_components::{刚体3D组件名, 碰撞体3D组件名};
use editor_app::plugin::{EditorStateHolder, UiStateHolder};
use editor_ui::dev_modes::开发模式;
use editor_ui::state::{UiAction, 基础图元类型};
use scene_doc::document::{场景文档, 实体标识};
use scene_doc::property::属性值;

/// 组装"动作 + 运行态世界"所需的最小资源集。
///
/// 与二维物理的测试夹具一致，另外把组件注册表换成完整表：创建图元要
/// 查 `Primitive3d` / `刚体3d` / `碰撞体3d`，插件启动不参与测试。
fn 建世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.注册表 = editor_app::scene_bridge::全部内置组件注册表();
    }
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(editor_app::scene_bridge::运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(editor_app::plugin::文件对话框通道::default());
    世界.insert_resource(editor_app::plugin::诊断资源::default());
    世界.insert_resource(editor_app::plugin::缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(editor_app::build::构建宿主::新建());
    世界.insert_resource(editor_app::diagnostics::诊断中心::default());
    世界.insert_resource(editor_app::config::编辑器配置资源::default());
    世界.insert_resource(editor_app::plugin::导入快照资源::default());
    世界.insert_resource(Time::<()>::default());
    世界
}

/// 建一个空的三维场景。
fn 建三维场景(世界: &mut World) {
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建场景("三维物理测试");
    }
    世界.resource_mut::<UiStateHolder>().状态.模式 = 开发模式::三维;
}

/// 把相机观察目标设成 `目标`，下一次创建图元就落在那里。
fn 放置目标(世界: &mut World, 目标: (f32, f32, f32)) {
    世界.resource_mut::<UiStateHolder>().状态.三维相机.目标 = 目标;
}

fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

/// 推入一条动作并立即执行。
fn 执行(世界: &mut World, 动作: UiAction) {
    推动作(世界, 动作);
    执行动作(世界);
}

/// 在场景文档里按名称找实体。
fn 按名称<'a>(文档: &'a 场景文档, 名称: &str) -> &'a scene_doc::document::场景实体 {
    文档
        .文档遍历()
        .into_iter()
        .filter_map(|标识| 文档.取(标识))
        .find(|实体| 实体.名称 == 名称)
        .unwrap_or_else(|| panic!("找不到实体 `{名称}`"))
}

fn 取浮点(实体: &scene_doc::document::场景实体, 组件名: &str, 键: &str) -> f32 {
    实体
        .取组件(组件名)
        .and_then(|组件| 组件.属性.get(键))
        .and_then(属性值::取浮点)
        .unwrap_or_else(|| panic!("{组件名}.{键} 应当是浮点"))
}

fn 取枚举<'a>(
    实体: &'a scene_doc::document::场景实体, 组件名: &str, 键: &str
) -> &'a str {
    实体
        .取组件(组件名)
        .and_then(|组件| 组件.属性.get(键))
        .and_then(|值| match 值 {
            属性值::枚举(枚举) => Some(枚举.变体名.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{组件名}.{键} 应当是枚举"))
}

/// 跑 `帧数` 帧，每帧 16ms，驱动真实的 `推进运行时`。
fn 跑若干帧(世界: &mut World, 帧数: u32) {
    for _ in 0..帧数 {
        {
            let mut 时间 = 世界.resource_mut::<Time>();
            let time = &mut *时间;
            time.advance_by(Duration::from_millis(16));
        }
        世界
            .run_system_once(actions::推进运行时)
            .expect("推进运行时应当能运行");
    }
}

/// 运行时世界里某实体的 y 坐标。
fn 运行时_y(世界: &World, 文档标识: 实体标识) -> Option<f32> {
    let 运行时 = 世界.resource::<editor_app::scene_bridge::运行时宿主>();
    let 实体 = 运行时.映射.取运行时(文档标识)?;
    let 世界引用 = 运行时.世界.as_ref()?;
    世界引用
        .get::<Transform>(实体)
        .map(|变换| 变换.translation.y)
}

/// 文档里某实体的 y 坐标。
fn 文档_y(世界: &World, 文档标识: 实体标识) -> f32 {
    let 编辑器 = 世界.resource::<EditorStateHolder>();
    编辑器
        .状态
        .会话
        .文档
        .取(文档标识)
        .and_then(|实体| 实体.变换)
        .map(|变换| 变换.平移.1)
        .expect("实体应当有变换")
}

fn 找标识(世界: &World, 名称: &str) -> 实体标识 {
    let 编辑器 = 世界.resource::<EditorStateHolder>();
    编辑器
        .状态
        .会话
        .文档
        .实体表
        .iter()
        .find(|(_, 实体)| 实体.名称 == 名称)
        .map(|(标识, _)| *标识)
        .unwrap_or_else(|| panic!("找不到实体 `{名称}`"))
}

#[test]
fn 创建图元自动挂载匹配尺寸的刚体与碰撞体() {
    let mut 世界 = 建世界();
    建三维场景(&mut 世界);

    for 类型 in 基础图元类型::全部 {
        放置目标(&mut 世界, (0.0, 0.0, 0.0));
        执行(&mut 世界, UiAction::创建基础图元 { 类型 });
    }

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 文档 = &编辑器.状态.会话.文档;
    assert_eq!(文档.实体数量(), 4, "四个图元都应当存在");

    let 立方体 = 按名称(文档, "立方体");
    assert!(
        立方体.取组件("Primitive3d").is_some(),
        "图元组件不能被替换掉"
    );
    assert_eq!(取枚举(立方体, 刚体3D组件名, "类型"), "Dynamic");
    assert_eq!(取枚举(立方体, 碰撞体3D组件名, "形状"), "Box");
    assert!((取浮点(立方体, 碰撞体3D组件名, "宽") - 1.0).abs() < 1e-6);
    assert!((取浮点(立方体, 碰撞体3D组件名, "高") - 1.0).abs() < 1e-6);
    assert!((取浮点(立方体, 碰撞体3D组件名, "深") - 1.0).abs() < 1e-6);
    assert!((取浮点(立方体, 刚体3D组件名, "质量") - 1.0).abs() < 1e-6);

    let 球体 = 按名称(文档, "球体");
    assert_eq!(取枚举(球体, 刚体3D组件名, "类型"), "Dynamic");
    assert_eq!(取枚举(球体, 碰撞体3D组件名, "形状"), "Sphere");
    assert!((取浮点(球体, 碰撞体3D组件名, "半径") - 0.5).abs() < 1e-6);

    let 圆柱体 = 按名称(文档, "圆柱体");
    assert_eq!(取枚举(圆柱体, 刚体3D组件名, "类型"), "Dynamic");
    assert_eq!(取枚举(圆柱体, 碰撞体3D组件名, "形状"), "Cylinder");
    assert!((取浮点(圆柱体, 碰撞体3D组件名, "半径") - 0.5).abs() < 1e-6);
    assert!((取浮点(圆柱体, 碰撞体3D组件名, "半高") - 1.0).abs() < 1e-6);

    let 平面 = 按名称(文档, "平面");
    assert_eq!(
        取枚举(平面, 刚体3D组件名, "类型"),
        "Static",
        "地面如果也会下落，场景里就没有参照物了"
    );
    assert_eq!(取枚举(平面, 碰撞体3D组件名, "形状"), "Box");
    assert!((取浮点(平面, 碰撞体3D组件名, "宽") - 10.0).abs() < 1e-6);
    assert!((取浮点(平面, 碰撞体3D组件名, "高") - 0.2).abs() < 1e-6);
    assert!((取浮点(平面, 碰撞体3D组件名, "深") - 10.0).abs() < 1e-6);
}

#[test]
fn 预览模式不推进物理() {
    let mut 世界 = 建世界();
    建三维场景(&mut 世界);
    放置目标(&mut 世界, (0.0, 5.0, 0.0));
    执行(
        &mut 世界,
        UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        },
    );

    // 没点运行：既没有运行态世界，也没有三维物理世界。
    assert!(
        世界.resource::<物理运行时状态>().当前三维.is_none(),
        "预览模式下不允许存在物理世界"
    );
    跑若干帧(&mut 世界, 60);

    let 标识 = 找标识(&世界, "立方体");
    assert_eq!(
        文档_y(&世界, 标识),
        5.0,
        "预览模式下物体必须静止，物理不得污染文档"
    );
}

#[test]
fn 运行后立方体受重力下落并停在地面上() {
    let mut 世界 = 建世界();
    建三维场景(&mut 世界);

    // 地面：平面自带静态刚体，原地不动。
    放置目标(&mut 世界, (0.0, 0.0, 0.0));
    执行(
        &mut 世界,
        UiAction::创建基础图元 {
            类型: 基础图元类型::平面,
        },
    );
    // 立方体从 5 米高处落下。
    放置目标(&mut 世界, (0.0, 5.0, 0.0));
    执行(
        &mut 世界,
        UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        },
    );

    执行(&mut 世界, UiAction::运行场景);

    {
        let 物理 = 世界.resource::<物理运行时状态>();
        let 三维 = 物理.当前三维.as_ref().expect("运行后应当有三维物理世界");
        assert_eq!(三维.实体数(), 2, "地面与立方体都应当进入物理世界");
        assert_eq!(物理.上次诊断三维.刚体数, 2);
    }

    let 立方体 = 找标识(&世界, "立方体");
    let 地面 = 找标识(&世界, "平面");
    let 初始 = 运行时_y(&世界, 立方体).expect("运行时应当有立方体");
    assert!(
        (初始 - 5.0).abs() < 1e-4,
        "初始高度应当是 5 米，实际 {初始}"
    );

    // 两秒：足够落下并被地面接住。
    跑若干帧(&mut 世界, 120);

    let 之后 = 运行时_y(&世界, 立方体).expect("运行时应当有立方体");
    assert!(
        之后 < 1.2,
        "两秒后立方体应当已经落地：初始 {初始}，现在 {之后}"
    );
    assert!(
        之后 > 0.3,
        "立方体不应穿过地面：地面顶面在 0.1，立方体半高 0.5，现在 {之后}"
    );
    let 地面_y = 运行时_y(&世界, 地面).expect("运行时应当有地面");
    assert!(地面_y.abs() < 1e-4, "静态刚体不应移动，实际 {地面_y}");

    // 文档隔离：物理只写运行时 Transform。
    assert_eq!(文档_y(&世界, 立方体), 5.0, "文档里的高度必须保持不变");
    assert_eq!(文档_y(&世界, 地面), 0.0, "地面在文档里也不应移动");
}

#[test]
fn 停止运行后物理世界被丢弃且可重新开始() {
    let mut 世界 = 建世界();
    建三维场景(&mut 世界);
    放置目标(&mut 世界, (0.0, 5.0, 0.0));
    执行(
        &mut 世界,
        UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        },
    );

    执行(&mut 世界, UiAction::运行场景);
    跑若干帧(&mut 世界, 30);
    assert!(世界.resource::<物理运行时状态>().当前三维.is_some());

    执行(&mut 世界, UiAction::停止运行);
    assert!(
        世界.resource::<物理运行时状态>().当前三维.is_none(),
        "停止后三维物理世界必须被丢弃"
    );

    执行(&mut 世界, UiAction::运行场景);
    let 立方体 = 找标识(&世界, "立方体");
    let 重新开始 = 运行时_y(&世界, 立方体).expect("运行时应当有立方体");
    assert!(
        (重新开始 - 5.0).abs() < 1e-4,
        "再次运行应当从文档里的初始位置开始，实际 {重新开始}"
    );
}

#[test]
fn 没有物理组件的场景不会建立三维物理世界() {
    let mut 世界 = 建世界();
    建三维场景(&mut 世界);
    // 放一个普通节点，确保场景非空（空场景会被运行时以"场景为空"拒绝，
    // 那样测到的就不是物理侧的行为了）。
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.会话.文档.生成实体("空节点", None);
    }
    执行(&mut 世界, UiAction::运行场景);
    assert!(
        世界
            .resource::<editor_app::scene_bridge::运行时宿主>()
            .世界
            .is_some(),
        "运行时本身应当已经启动"
    );

    let 物理 = 世界.resource::<物理运行时状态>();
    assert_eq!(物理.上次诊断三维.刚体数, 0);
    assert!(
        物理.当前三维.is_none(),
        "没有任何三维物理组件时不应挂着空物理世界"
    );
}

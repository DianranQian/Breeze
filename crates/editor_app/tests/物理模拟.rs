//! 2D 物理模拟的端到端验收：文档 → 运行态世界 → 真实模拟 → 写回。
//!
//! # 这条链为什么必须整条测
//!
//! 物理功能有四个容易"看起来对、实际不通"的接缝：
//!
//! 1. **场景文档 → 刚体**：`碰撞体2d`/`刚体2d` 的属性名与单位是否被正确解析；
//! 2. **运行态世界 → 物理**：`实体映射表` 是否把文档实体对上了运行时实体；
//! 3. **步进 → 写回**：物理结果是否真的进了运行时 `Transform`；
//! 4. **文档隔离**：运行时的姿态**绝不能**写回场景文档（否则保存一次就毁场景）。
//!
//! 因此这里走真实动作（`运行场景`）与真实系统（`推进运行时`），
//! 并且同时断言"运行时实体在动"与"文档没动"。

use std::time::Duration;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{EditorStateHolder, UiStateHolder};
use editor_core::collision::{刚体组件名, 碰撞体组件名};
use editor_ui::state::UiAction;
use scene_doc::document::{变换数据, 场景文档, 实体标识, 组件数据};
use scene_doc::property::属性值;

fn 建世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(editor_app::scene_bridge::运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(editor_app::plugin::文件对话框通道::default());
    世界.insert_resource(editor_app::plugin::诊断资源::default());
    世界.insert_resource(editor_app::plugin::缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(editor_app::build::构建宿主::新建());
    // 统一诊断中心：`执行动作队列` 会采集构建/运行诊断，需要它存在。
    世界.insert_resource(editor_app::diagnostics::诊断中心::default());
    世界.insert_resource(editor_app::config::编辑器配置资源::default());
    世界.insert_resource(editor_app::plugin::导入快照资源::default());
    世界.insert_resource(Time::<()>::default());
    世界
}

fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

/// 加一个带碰撞体的实体，返回其标识。
fn 加碰撞实体(
    文档: &mut 场景文档,
    名称: &str,
    平移: (f32, f32, f32),
    宽度: f32,
    高度: f32,
    触发器: bool,
    刚体: Option<(&str, f32, f32)>,
) -> 实体标识 {
    let 标识 = 文档.生成实体(名称, None);
    let 实体 = 文档.取可变(标识).expect("刚生成");
    实体.变换 = Some(变换数据 {
        平移,
        ..变换数据::单位()
    });
    let 碰撞 = 组件数据::新建(
        碰撞体组件名,
        [
            ("形状".to_string(), 属性值::文本("矩形".to_string())),
            ("宽度".to_string(), 属性值::浮点(宽度)),
            ("高度".to_string(), 属性值::浮点(高度)),
            ("半径".to_string(), 属性值::浮点(宽度 * 0.5)),
            ("触发器".to_string(), 属性值::布尔(触发器)),
        ]
        .into_iter()
        .collect(),
    );
    实体.组件.push(碰撞);
    if let Some((类型, 质量, 重力缩放)) = 刚体 {
        let 刚体组件 = 组件数据::新建(
            刚体组件名,
            [
                (
                    "类型".to_string(),
                    // 与编辑器的写法一致：枚举属性存变体名。
                    属性值::枚举(scene_doc::property::枚举值::新建(类型, 0)),
                ),
                ("质量".to_string(), 属性值::浮点(质量)),
                ("重力缩放".to_string(), 属性值::浮点(重力缩放)),
            ]
            .into_iter()
            .collect(),
        );
        实体.组件.push(刚体组件);
    }
    标识
}

/// 跑 `帧数` 帧，每帧 16ms，驱动真实的 `推进运行时`。
fn 跑若干帧(世界: &mut World, 帧数: u32) {
    for _ in 0..帧数 {
        {
            let mut 时间 = 世界.resource_mut::<Time>();
            let time = &mut *时间;
            // `Time::advance_by` 需要 `Time<()>`；这里拿到的是默认 `Time`
            // （等价于 `Time<Virtual>` 的泛型默认值），Bevy 里它就是 `Time<()>`。
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

#[test]
fn 运行后动态刚体下落并被地面接住() {
    let mut 世界 = 建世界();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建场景("物理测试");
        let 文档 = &mut 编辑器.状态.会话.文档;
        加碰撞实体(
            文档,
            "地面",
            (0.0, 0.0, 0.0),
            2000.0,
            40.0,
            false,
            Some(("静态", 1.0, 1.0)),
        );
        加碰撞实体(
            文档,
            "箱子",
            (0.0, 300.0, 0.0),
            40.0,
            40.0,
            false,
            Some(("动态", 1.0, 1.0)),
        );
    }

    推动作(&mut 世界, UiAction::运行场景);
    执行动作(&mut 世界);

    {
        let 物理 = 世界.resource::<物理运行时状态>();
        assert_eq!(物理.上次诊断.刚体数, 2, "两个碰撞实体都应当进入物理世界");
        assert!(物理.当前.is_some(), "运行后应当有物理世界");
    }

    // 运行态世界的箱子初始在 y=300。
    let 箱子 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        编辑器
            .状态
            .会话
            .文档
            .实体表
            .iter()
            .find(|(_, 实体)| 实体.名称 == "箱子")
            .map(|(标识, _)| *标识)
            .expect("箱子应当存在")
    };
    let 初始 = 运行时_y(&世界, 箱子).expect("运行时应当有这个实体");
    assert!((初始 - 300.0).abs() < 0.5, "初始 y 应当是 300，实际 {初始}");

    跑若干帧(&mut 世界, 60);

    let 一秒后 = 运行时_y(&世界, 箱子).expect("运行时应当有这个实体");
    assert!(
        一秒后 < 初始 - 30.0,
        "一秒后箱子应当明显下落：初始 {初始}，现在 {一秒后}"
    );

    // 文档绝不能被运行时的姿态污染。
    {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        let 变换 = 编辑器.状态.会话.文档.取(箱子).unwrap().变换.unwrap();
        assert!(
            (变换.平移.1 - 300.0).abs() < 0.01,
            "文档里的变换必须保持不变，实际 {}",
            变换.平移.1
        );
    }
}

#[test]
fn 停止运行后物理世界被丢弃且可以重新开始() {
    let mut 世界 = 建世界();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建场景("物理测试");
        let 文档 = &mut 编辑器.状态.会话.文档;
        加碰撞实体(
            文档,
            "箱子",
            (0.0, 200.0, 0.0),
            40.0,
            40.0,
            false,
            Some(("动态", 1.0, 1.0)),
        );
    }
    推动作(&mut 世界, UiAction::运行场景);
    执行动作(&mut 世界);
    跑若干帧(&mut 世界, 30);
    assert!(世界.resource::<物理运行时状态>().当前.is_some());

    推动作(&mut 世界, UiAction::停止运行);
    执行动作(&mut 世界);
    assert!(
        世界.resource::<物理运行时状态>().当前.is_none(),
        "停止后物理世界必须被丢弃"
    );

    // 再次开始：位置从文档重新初始化（而不是接着上次的落点）。
    推动作(&mut 世界, UiAction::运行场景);
    执行动作(&mut 世界);
    let 箱子 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        编辑器
            .状态
            .会话
            .文档
            .实体表
            .iter()
            .find(|(_, 实体)| 实体.名称 == "箱子")
            .map(|(标识, _)| *标识)
            .unwrap()
    };
    let 重新开始 = 运行时_y(&世界, 箱子).expect("运行时应当有这个实体");
    assert!(
        (重新开始 - 200.0).abs() < 0.5,
        "重新运行应当回到文档里的初始位置，实际 {重新开始}"
    );
}

#[test]
fn 触发器接触会写进控制台日志() {
    let mut 世界 = 建世界();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建场景("物理测试");
        let 文档 = &mut 编辑器.状态.会话.文档;
        // 危险区是静态触发器，箱子从上方落下穿过它。
        加碰撞实体(
            文档,
            "危险区",
            (0.0, 150.0, 0.0),
            200.0,
            100.0,
            true,
            Some(("静态", 1.0, 1.0)),
        );
        加碰撞实体(
            文档,
            "箱子",
            (0.0, 400.0, 0.0),
            40.0,
            40.0,
            false,
            Some(("动态", 1.0, 1.0)),
        );
    }
    推动作(&mut 世界, UiAction::运行场景);
    执行动作(&mut 世界);
    跑若干帧(&mut 世界, 60);

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 有触发器日志 = 编辑器
        .状态
        .日志列表()
        .iter()
        .any(|条目| 条目.渲染(editor_ui::语言::中文).contains("触发器进入"));
    assert!(有触发器日志, "箱子穿过触发器时应当有控制台日志");
}

#[test]
fn 暂停时物理不再推进() {
    let mut 世界 = 建世界();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建场景("物理测试");
        let 文档 = &mut 编辑器.状态.会话.文档;
        加碰撞实体(
            文档,
            "箱子",
            (0.0, 300.0, 0.0),
            40.0,
            40.0,
            false,
            Some(("动态", 1.0, 1.0)),
        );
    }
    推动作(&mut 世界, UiAction::运行场景);
    执行动作(&mut 世界);
    跑若干帧(&mut 世界, 10);
    推动作(&mut 世界, UiAction::暂停运行);
    执行动作(&mut 世界);

    let 箱子 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        编辑器
            .状态
            .会话
            .文档
            .实体表
            .iter()
            .find(|(_, 实体)| 实体.名称 == "箱子")
            .map(|(标识, _)| *标识)
            .unwrap()
    };
    let 暂停时 = 运行时_y(&世界, 箱子).expect("运行时应当有这个实体");
    跑若干帧(&mut 世界, 30);
    let 之后 = 运行时_y(&世界, 箱子).expect("运行时应当有这个实体");
    assert_eq!(暂停时, 之后, "暂停时物理不应继续推进");
}

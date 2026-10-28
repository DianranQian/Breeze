//! 中文名：光源与环境
//!
//! 扩展光源（方向光 / 点光 / 聚光灯）与场景环境设置的端到端测试。
//!
//! # 覆盖的链路
//!
//! ```text
//! UiAction::创建光源 / 设置场景环境
//!   → 执行动作队列（编辑域）
//!   → 文档里的灯光组件与文档级环境字段
//!   → 保存为 .scene.ron → 重新打开后逐字段一致
//! ```
//!
//! 与 `primitives.rs` 同样的理由：每个用例自建世界，避免 `执行动作队列`
//! 的 `Local` 状态在用例之间互相污染。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::components3d::{方向光组件名, 点光组件名, 聚光灯组件名};
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::scene_bridge::运行时宿主;
use editor_ui::dev_modes::开发模式;
use editor_ui::state::{UiAction, 光源类型};
use scene_doc::document::{场景实体, 场景环境};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 颜色值};

/// 组装一个只含"动作执行"所需资源的世界。
fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.resource_mut::<EditorStateHolder>().状态.注册表 =
        editor_app::scene_bridge::全部内置组件注册表();
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(editor_app::physics_runtime::物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(editor_app::build::构建宿主::新建());
    世界.insert_resource(editor_app::diagnostics::诊断中心::default());
    世界.insert_resource(editor_app::config::编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
    世界
}

/// 建一个临时项目：场景保存需要项目路径才能真正落盘。
fn 建项目(世界: &mut World, 名称: &str) -> tempfile::TempDir {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join(名称);
    世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .创建项目(&根, 名称)
        .expect("创建项目应当成功");
    目录
}

fn 设置模式(世界: &mut World, 模式: 开发模式) {
    世界.resource_mut::<UiStateHolder>().状态.模式 = 模式;
}

fn 执行(世界: &mut World, 动作列表: Vec<UiAction>) {
    {
        let mut 界面 = 世界.resource_mut::<UiStateHolder>();
        for 动作 in 动作列表 {
            界面.动作.推(动作);
        }
    }
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

/// 按名称找实体；找不到时列出已有名称，便于定位。
fn 按名称<'a>(文档: &'a scene_doc::document::场景文档, 名称: &str) -> &'a 场景实体 {
    let 标识 = 文档
        .文档遍历()
        .into_iter()
        .find(|标识| 文档.取(*标识).is_some_and(|实体| 实体.名称 == 名称))
        .unwrap_or_else(|| {
            let 全部: Vec<&str> = 文档
                .文档遍历()
                .into_iter()
                .filter_map(|标识| 文档.取(标识))
                .map(|实体| 实体.名称.as_str())
                .collect();
            panic!("找不到实体 `{名称}`，现有：{全部:?}")
        });
    文档.取(标识).expect("上面已确认存在")
}

fn 场景组件(类型: 光源类型) -> &'static str {
    match 类型 {
        光源类型::方向光 => 方向光组件名,
        光源类型::点光 => 点光组件名,
        光源类型::聚光灯 => 聚光灯组件名,
    }
}

#[test]
fn 三种光源都能在相机焦点创建并选中() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "光源项目");
    设置模式(&mut 世界, 开发模式::三维);
    世界.resource_mut::<UiStateHolder>().状态.三维相机.目标 = (4.0, 5.0, -6.0);

    for 类型 in 光源类型::全部 {
        执行(&mut 世界, vec![UiAction::创建光源 { 类型 }]);
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        let 名称 = 编辑器
            .状态
            .会话
            .文档
            .取(编辑器.状态.选择.主选中().unwrap())
            .unwrap();
        assert_eq!(名称.名称, 类型.固定名());
        assert!(
            名称.有组件(场景组件(类型)),
            "{} 应当挂上对应的灯光组件",
            类型.固定名()
        );
        assert_eq!(
            名称.变换.expect("灯光必须有变换").平移,
            (4.0, 5.0, -6.0),
            "灯应当放在轨道相机的观察目标处"
        );
    }
    assert_eq!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .会话
            .文档
            .实体数量(),
        3
    );
}

#[test]
fn 方向光与聚光灯默认朝下() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "朝向项目");
    设置模式(&mut 世界, 开发模式::三维);

    for 类型 in [光源类型::方向光, 光源类型::聚光灯] {
        执行(&mut 世界, vec![UiAction::创建光源 { 类型 }]);
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        let 实体 = 按名称(&编辑器.状态.会话.文档, 类型.固定名());
        let 旋转 = 实体.变换.expect("灯光必须有变换").旋转;
        // 绕 X 轴 -90°：-Z 轴（Bevy 光源的照射方向）转到 -Y，即正下方。
        assert!(
            (旋转.0 + std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4
                && (旋转.3 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4,
            "{} 应默认朝下，实际四元数 {旋转:?}",
            类型.固定名()
        );
    }
}

#[test]
fn 创建光源只占一步撤销() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "灯光撤销项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建光源 {
            类型: 光源类型::点光,
        }],
    );
    assert_eq!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .会话
            .文档
            .实体数量(),
        1
    );

    执行(&mut 世界, vec![UiAction::撤销]);
    assert_eq!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .会话
            .文档
            .实体数量(),
        0,
        "一次撤销应当整盏灯消失"
    );
}

#[test]
fn 二维模式不创建光源() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "二维灯光项目");
    设置模式(&mut 世界, 开发模式::二维);

    执行(
        &mut 世界,
        vec![UiAction::创建光源 {
            类型: 光源类型::点光,
        }],
    );
    assert_eq!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .会话
            .文档
            .实体数量(),
        0,
        "二维场景没有光照模型，不应创建一个不起作用的节点"
    );
}

#[test]
fn 光源与环境保存为ron后重新打开逐字段一致() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "灯光落盘项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建光源 {
            类型: 光源类型::聚光灯,
        }],
    );
    let 标识 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        按名称(&编辑器.状态.会话.文档, "聚光灯").标识
    };
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        assert!(
            编辑器
                .状态
                .设属性(标识, 聚光灯组件名, "强度", 属性值::浮点(2500.0))
        );
        assert!(
            编辑器
                .状态
                .设属性(标识, 聚光灯组件名, "内锥角", 属性值::浮点(18.0))
        );
        assert!(
            编辑器
                .状态
                .设属性(标识, 聚光灯组件名, "外锥角", 属性值::浮点(36.0))
        );
        assert!(
            编辑器
                .状态
                .设属性(标识, 聚光灯组件名, "投射阴影", 属性值::布尔(false))
        );
    }

    let 环境 = 场景环境 {
        环境光颜色: 颜色值::srgb(1.0, 0.5, 0.25, 1.0),
        环境光强度: 850.0,
        背景颜色: 颜色值::srgb(0.15, 0.2, 0.4, 1.0),
    };
    执行(&mut 世界, vec![UiAction::设置场景环境(环境)]);

    let 路径 = 项目相对路径::解析("scenes/灯光.scene.ron").expect("路径应当合法");
    let 原始 = {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.保存场景到(&路径).expect("保存应当成功");
        编辑器.状态.会话.文档.clone()
    };

    世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .打开场景(&路径)
        .expect("重新打开应当成功");
    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 重载 = &编辑器.状态.会话.文档;

    assert_eq!(重载.实体表, 原始.实体表, "重新加载后实体表应完全一致");
    assert_eq!(重载.环境, 环境, "环境设置必须随场景一起保存与读取");

    let 实体 = 按名称(重载, "聚光灯");
    let 组件 = 实体.取组件(聚光灯组件名).expect("重载后仍应有聚光灯组件");
    assert_eq!(组件.属性.get("强度"), Some(&属性值::浮点(2500.0)));
    assert_eq!(组件.属性.get("内锥角"), Some(&属性值::浮点(18.0)));
    assert_eq!(组件.属性.get("外锥角"), Some(&属性值::浮点(36.0)));
    assert_eq!(组件.属性.get("投射阴影"), Some(&属性值::布尔(false)));
}

#[test]
fn 环境修改合并成一步撤销() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "环境撤销项目");
    设置模式(&mut 世界, 开发模式::三维);
    let 默认 = 世界.resource::<EditorStateHolder>().状态.会话.文档.环境;

    执行(
        &mut 世界,
        vec![UiAction::设置场景环境(场景环境 {
            环境光强度: 900.0,
            ..默认
        })],
    );
    执行(
        &mut 世界,
        vec![UiAction::设置场景环境(场景环境 {
            环境光强度: 1200.0,
            ..默认
        })],
    );
    assert_eq!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .会话
            .文档
            .环境
            .环境光强度,
        1200.0
    );

    执行(&mut 世界, vec![UiAction::撤销]);
    assert_eq!(
        世界.resource::<EditorStateHolder>().状态.会话.文档.环境,
        默认,
        "连续的环境修改应当合并为一步撤销"
    );
}

#[test]
fn 撤销环境会触发预览修订号变化() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "环境修订项目");
    设置模式(&mut 世界, 开发模式::三维);
    let 修订前 = 世界.resource::<EditorStateHolder>().状态.文档修订();

    执行(
        &mut 世界,
        vec![UiAction::设置场景环境(场景环境 {
            背景颜色: 颜色值::srgb(0.9, 0.1, 0.1, 1.0),
            ..Default::default()
        })],
    );
    let 修订后 = 世界.resource::<EditorStateHolder>().状态.文档修订();
    assert!(
        修订后 > 修订前,
        "环境变化必须推进修订号，否则预览同步不会看到新值"
    );
}

//! 中文名：基础图元
//!
//! 基础几何体快速放置与默认场景环境的端到端测试。
//!
//! # 覆盖的链路
//!
//! ```text
//! UiAction::创建基础图元
//!   → 执行动作队列
//!   → 文档里的 Primitive3d 实体（形状 / 尺寸 / 颜色 / 变换）
//!   → 保存为 .scene.ron
//!   → 重新打开后结构不变
//! ```
//!
//! 与直接调用 `编辑器状态` 的 API 相比，走动作队列会多验证两件事：
//! 动作路由把变体交给了编辑域，以及 `动作上下文` 里的界面状态
//! （模式、视口相机、语言）被正确读取。
//!
//! # 为什么每个用例自建世界
//!
//! `执行动作队列` 内部有 `Local` 状态（拖动快照、面板帧状态）。
//! `run_system_once` 会为每个世界保存一份局部状态，共享世界会让用例互相污染，
//! 因此每个测试都新建一个 `World`，并且不加载窗口与渲染插件。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::scene_bridge::运行时宿主;
use editor_ui::dev_modes::开发模式;
use editor_ui::picking3d::{世界包围盒, 射线, 射线拾取, 文档实体包围盒};
use editor_ui::state::{UiAction, 世界变换表, 基础图元类型};
use scene_doc::document::{场景实体, 实体标识};
use scene_doc::path::项目相对路径;
use scene_doc::property::属性值;

/// 从三维视口的一个局部像素点发射世界射线，返回命中的实体。
///
/// 拾取的输入与生产完全一致：相机 → 屏幕射线 → 文档实体的世界 AABB。
/// 预览快照在本测试里为空（不跑渲染插件），因此走文档尺寸 + 世界变换兜底 ——
/// 这正是「预览首帧之前」的生产退化路径。
fn 从视口点拾取(
    世界: &World, 局部: egui::Vec2, 尺寸: egui::Vec2
) -> Option<实体标识> {
    let 相机 = 世界.resource::<UiStateHolder>().状态.三维相机;
    let (起点, 方向) = 相机.屏幕射线(局部, 尺寸);
    let 射 = 射线::新建(起点, 方向);
    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 候选: Vec<(实体标识, 世界包围盒)> = 编辑器
        .状态
        .会话
        .文档
        .实体表
        .values()
        .filter(|实体| !实体.编辑器隐藏 && !实体.编辑器锁定)
        .filter_map(|实体| 文档实体包围盒(实体, &世界变换表::new()).map(|盒| (实体.标识, 盒)))
        .collect();
    射线拾取(&候选, &射)
}

/// 组装一个只含"动作执行"所需资源的世界。
///
/// 刻意不加 `EguiPlugin`：绘制需要窗口与 GPU，而本文件验证的是动作与文档，
/// 把绘制排除在外，测试才能在无显示器的环境里跑完。
fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    // 生产环境在启动系统 `安装扩展组件` 里把基础注册表换成完整表；
    // 测试不走插件启动，这里显式做同一件事，否则 `Camera3d` / `方向光`
    // 这类扩展组件查不到，默认场景环境会"静默不补"。
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

/// 建一个临时项目：场景保存需要项目路径，才能真正落盘到 `scenes/`。
fn 建项目(世界: &mut World, 名称: &str) -> tempfile::TempDir {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join(名称);
    let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
    编辑器.状态.创建项目(&根, 名称).expect("创建项目应当成功");
    目录
}

/// 切换界面模式（新建场景时据此决定补哪种相机）。
fn 设置模式(世界: &mut World, 模式: 开发模式) {
    世界.resource_mut::<UiStateHolder>().状态.模式 = 模式;
}

/// 把动作推进队列，再执行一次。
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

/// 按名称找场景实体，找不到时直接失败并给出已有名称。
fn 按名称<'a>(文档: &'a scene_doc::document::场景文档, 名称: &str) -> &'a 场景实体 {
    let 全部: Vec<&str> = 文档
        .文档遍历()
        .into_iter()
        .filter_map(|标识| 文档.取(标识))
        .map(|实体| 实体.名称.as_str())
        .collect();
    let 标识 = 文档
        .文档遍历()
        .into_iter()
        .find(|标识| 文档.取(*标识).is_some_and(|实体| 实体.名称 == 名称))
        .unwrap_or_else(|| panic!("找不到实体 `{名称}`，现有：{全部:?}"));
    文档.取(标识).expect("上面已确认存在")
}

#[test]
fn 创建立方体会把图元放在视口焦点并选中() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "图元项目");
    设置模式(&mut 世界, 开发模式::三维);
    世界.resource_mut::<UiStateHolder>().状态.三维相机.目标 = (2.0, 0.0, -3.0);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        }],
    );

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    assert_eq!(编辑器.状态.会话.文档.实体数量(), 1, "应当只新增一个实体");

    let 实体 = 按名称(&编辑器.状态.会话.文档, "立方体");
    let 组件 = 实体.取组件("Primitive3d").expect("必须挂上 Primitive3d");
    match 组件.属性.get("形状") {
        Some(属性值::枚举(值)) => {
            assert_eq!(值.变体名, "Cuboid");
            assert_eq!(值.序号, 0);
        }
        其他 => panic!("形状字段应为枚举，实际：{其他:?}"),
    }
    match 组件.属性.get("尺寸") {
        Some(属性值::三维(x, y, z)) => assert_eq!((*x, *y, *z), (1.0, 1.0, 1.0)),
        其他 => panic!("尺寸字段应为三维向量，实际：{其他:?}"),
    }
    assert!(
        matches!(组件.属性.get("颜色"), Some(属性值::颜色(_))),
        "必须带默认材质颜色"
    );
    assert_eq!(
        实体.变换.expect("必须写变换").平移,
        (2.0, 0.0, -3.0),
        "放置点应取三维轨道相机的目标"
    );
    assert_eq!(
        编辑器.状态.选择.主选中(),
        Some(实体.标识),
        "创建后应自动选中新实体"
    );
}

#[test]
fn 视口中心射线能命中新建的立方体() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "拾取项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        }],
    );

    let 立方体 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        按名称(&编辑器.状态.会话.文档, "立方体").标识
    };
    let 尺寸 = egui::vec2(800.0, 600.0);
    assert_eq!(
        从视口点拾取(&世界, 尺寸 * 0.5, 尺寸),
        Some(立方体),
        "视口中心正对立方体，射线必须命中它"
    );
}

/// 回归：旧实现按锚点屏幕距离 14px 判命中，点在立方体表面但离轴心
/// 24px 时毫无反应 —— 用户看到的就是「3D 视口里点不中模型」。
#[test]
fn 点在立方体表面而不是轴心也能选中() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "表面拾取项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        }],
    );

    let 立方体 = {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        按名称(&编辑器.状态.会话.文档, "立方体").标识
    };
    let 尺寸 = egui::vec2(800.0, 600.0);
    assert_eq!(
        从视口点拾取(&世界, 尺寸 * 0.5 + egui::vec2(24.0, 0.0), 尺寸),
        Some(立方体),
        "偏轴 24px 仍在立方体的 AABB 内，射线必须命中"
    );
}

#[test]
fn 射线打在空处时不会误选() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "空拾取项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::立方体,
        }],
    );

    let 尺寸 = egui::vec2(800.0, 600.0);
    assert_eq!(
        从视口点拾取(&世界, egui::vec2(20.0, 20.0), 尺寸),
        None,
        "远离立方体的角落不能选中任何东西"
    );
}

#[test]
fn 创建基础图元只占一步撤销() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "撤销项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::球体,
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

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    assert_eq!(
        编辑器.状态.会话.文档.实体数量(),
        0,
        "一次撤销应当整个图元消失，而不是只退回某个子步骤"
    );
}

#[test]
fn 图元保存为ron后重新打开结构一致() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "落盘项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::圆柱体,
        }],
    );

    let 路径 = 项目相对路径::解析("scenes/图元.scene.ron").expect("路径应当合法");
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
    assert_eq!(重载.顶层顺序, 原始.顶层顺序, "重新加载后层级顺序应一致");

    let 实体 = 按名称(重载, "圆柱体");
    let 组件 = 实体
        .取组件("Primitive3d")
        .expect("重载后仍应有 Primitive3d");
    match 组件.属性.get("形状") {
        Some(属性值::枚举(值)) => {
            assert_eq!(值.变体名, "Cylinder");
            assert_eq!(值.序号, 3);
        }
        其他 => panic!("形状字段应为枚举，实际：{其他:?}"),
    }
}

#[test]
fn 新建三维场景自动补主相机与平行光() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "默认环境项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(&mut 世界, vec![UiAction::新建场景]);

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 文档 = &编辑器.状态.会话.文档;
    assert_eq!(文档.实体数量(), 2, "应当只补一套环境");
    按名称(文档, "主相机");
    assert!(按名称(文档, "主相机").有组件("Camera3d"));
    assert!(按名称(文档, "平行光").有组件("方向光"));
}

#[test]
fn 新建二维场景只补二维相机() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "二维环境项目");
    设置模式(&mut 世界, 开发模式::二维);

    执行(&mut 世界, vec![UiAction::新建场景]);

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 文档 = &编辑器.状态.会话.文档;
    assert_eq!(文档.实体数量(), 1, "二维场景不应多出一盏方向光");
    let 相机 = 按名称(文档, "主相机");
    assert!(相机.有组件("Camera2d"));
    assert!(!相机.有组件("Camera3d"));
}

#[test]
fn 打开没有相机的场景会补齐默认环境() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "补环境项目");
    设置模式(&mut 世界, 开发模式::三维);

    执行(
        &mut 世界,
        vec![UiAction::创建基础图元 {
            类型: 基础图元类型::平面,
        }],
    );
    let 路径 = 项目相对路径::解析("scenes/无相机.scene.ron").expect("路径应当合法");
    世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .保存场景到(&路径)
        .expect("保存应当成功");

    执行(&mut 世界, vec![UiAction::打开场景(路径)]);

    let 编辑器 = 世界.resource::<EditorStateHolder>();
    let 文档 = &编辑器.状态.会话.文档;
    assert_eq!(文档.实体数量(), 3, "原有平面 + 主相机 + 平行光");
    assert!(按名称(文档, "平面").有组件("Primitive3d"));
    assert!(按名称(文档, "主相机").有组件("Camera3d"));
    assert!(按名称(文档, "平行光").有组件("方向光"));
}

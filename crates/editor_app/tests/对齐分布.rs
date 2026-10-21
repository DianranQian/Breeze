//! 多选对齐/分布的端到端验证（走动作队列，与用户点击同一条路）。
//!
//! 覆盖：
//!
//! ```text
//! 3 个位置不同的实体 → 对齐选中(左) → 左边缘一致 → 一步撤销
//!                → 分布选中(水平) → 中间那个等距
//! 选中 <2        → 告警且文档不脏
//! ```

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{编辑器配置资源, 布局待保存};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::arrange::{分布轴, 对齐边};
use editor_core::project::项目描述;
use editor_ui::state::{UiAction, 选择意图};
use scene_doc::document::{场景文档, 变换数据, 实体标识};
use scene_doc::path::项目相对路径;

fn 建测试世界() -> World {
    let mut 世界 = World::new();
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
    世界
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

fn 编辑(世界: &World) -> &editor_core::state::编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

fn 位置(世界: &World, 标识: 实体标识) -> (f32, f32) {
    let 变换 = 编辑(世界)
        .会话
        .文档
        .取(标识)
        .and_then(|实体| 实体.变换)
        .expect("变换");
    (变换.平移.0, 变换.平移.1)
}

/// 三个各带 Sprite 的实体，位置分散。
fn 建场景(世界: &mut World) -> (tempfile::TempDir, Vec<实体标识>) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");

    let mut 文档 = 场景文档::新建("对齐");
    let mut 标识 = Vec::new();
    for (名称, x, y) in [("甲", 0.0_f32, 0.0_f32), ("乙", 100.0, 40.0), ("丙", 200.0, 90.0)] {
        let 实体 = 文档.生成实体(名称, None);
        let mut 变换 = 变换数据::单位();
        变换.平移 = (x, y, 0.0);
        文档.取可变(实体).expect("实体").变换 = Some(变换);
        标识.push(实体);
    }
    项目
        .保存场景(&项目相对路径::解析("scenes/对齐.scene.ron").unwrap(), &文档)
        .expect("保存场景");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    持有.应用已载入场景(
        文档,
        Some(项目相对路径::解析("scenes/对齐.scene.ron").unwrap()),
    );
    (目录, 标识)
}

#[test]
fn 左对齐后左边缘一致且能一步撤销() {
    let mut 世界 = 建测试世界();
    let (_目录, 标识) = 建场景(&mut 世界);
    推动作(&mut 世界, UiAction::选择(选择意图::设为(标识.clone())));
    执行动作(&mut 世界);

    推动作(&mut 世界, UiAction::对齐选中 { 边: 对齐边::左 });
    执行动作(&mut 世界);

    // 默认精灵框半宽 32：左边缘 = x - 32。对齐后三者 x 必须相同。
    let 位置列表: Vec<(f32, f32)> = 标识.iter().map(|标识| 位置(&世界, *标识)).collect();
    assert!(
        (位置列表[0].0 - 位置列表[1].0).abs() < 1e-3
            && (位置列表[0].0 - 位置列表[2].0).abs() < 1e-3,
        "左对齐后 x 应当一致：{位置列表:?}"
    );
    assert!(
        (位置列表[1].1 - 40.0).abs() < 1e-3,
        "对齐不能改另一轴：{位置列表:?}"
    );

    推动作(&mut 世界, UiAction::撤销);
    执行动作(&mut 世界);
    assert_eq!(位置(&世界, 标识[1]), (100.0, 40.0), "一步撤销必须整批退回");
}

#[test]
fn 水平分布让中间实体等距() {
    let mut 世界 = 建测试世界();
    let (_目录, 标识) = 建场景(&mut 世界);
    推动作(&mut 世界, UiAction::选择(选择意图::设为(标识.clone())));
    执行动作(&mut 世界);

    推动作(&mut 世界, UiAction::分布选中 { 轴: 分布轴::水平 });
    执行动作(&mut 世界);

    let 甲 = 位置(&世界, 标识[0]).0;
    let 乙 = 位置(&世界, 标识[1]).0;
    let 丙 = 位置(&世界, 标识[2]).0;
    assert!((甲 - 0.0).abs() < 1e-3, "首端不动：{甲}");
    assert!((丙 - 200.0).abs() < 1e-3, "末端不动：{丙}");
    assert!((乙 - 100.0).abs() < 1e-3, "中间应当等距：{乙}");
}

#[test]
fn 少于两个选中只告警不改文档() {
    let mut 世界 = 建测试世界();
    let (_目录, 标识) = 建场景(&mut 世界);
    推动作(&mut 世界, UiAction::选择(选择意图::单选(标识[0])));
    执行动作(&mut 世界);
    let 之前 = 位置(&世界, 标识[0]);

    推动作(&mut 世界, UiAction::对齐选中 { 边: 对齐边::左 });
    执行动作(&mut 世界);

    assert_eq!(位置(&世界, 标识[0]), 之前);
    assert!(!编辑(&世界).有未保存修改(), "被拒绝的操作不能写撤销栈");
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::i18n::语言::中文).contains("至少选中两个")),
        "必须告诉用户为什么没反应"
    );
}

#[test]
fn 已经对齐时不产生撤销步() {
    let mut 世界 = 建测试世界();
    let (_目录, 标识) = 建场景(&mut 世界);
    推动作(&mut 世界, UiAction::选择(选择意图::设为(标识.clone())));
    执行动作(&mut 世界);
    推动作(&mut 世界, UiAction::对齐选中 { 边: 对齐边::左 });
    执行动作(&mut 世界);
    let 步数 = 编辑(&世界).可撤销步数();

    推动作(&mut 世界, UiAction::对齐选中 { 边: 对齐边::左 });
    执行动作(&mut 世界);

    assert_eq!(
        编辑(&世界).可撤销步数(),
        步数,
        "没有实际移动就不该增加撤销步骤"
    );
}

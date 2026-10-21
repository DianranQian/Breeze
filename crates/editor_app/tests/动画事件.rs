//! 动画事件的端到端验证：添加、撤销、保存往返、播放触发。
//!
//! 覆盖用户真实路径：
//!
//! ```text
//! 时间轴双击/右键 → 添加动画事件（默认名称取项目里的音频资源）
//! Ctrl+Z          → 事件被撤销
//! 保存动画        → 事件写进 .anim.ron
//! 重新打开        → 事件还在
//! 播放跨过事件时间 → 产生一条"动画事件"日志（音效类型还要求文件真实存在）
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
use editor_core::animation::{动画事件, 动画文档, 事件类型};
use editor_core::project::项目描述;
use editor_ui::state::UiAction;
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

fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 编辑(世界: &World) -> &editor_core::state::编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

fn 界面(世界: &World) -> &editor_ui::界面状态 {
    &世界.resource::<UiStateHolder>().状态
}

fn 路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("路径")
}

/// 建项目 + 一个打开的动画（含一条轨道，保证保存时不因"空"而跳过）。
fn 建项目与动画(世界: &mut World) -> (tempfile::TempDir, std::path::PathBuf) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    std::fs::create_dir_all(根.join("assets").join("audio")).expect("建音频目录");
    std::fs::write(根.join("assets").join("audio").join("点击.ogg"), "ogg").expect("写音频");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    let 实体 = 持有.新建实体("角色", None).expect("新实体");
    let mut 会话 = editor_core::animation::动画会话::新建("关卡动画");
    会话.文档.时长 = 4.0;
    let 轨道 = 会话.文档.轨道路径(实体, editor_core::animation::动画属性::平移);
    轨道.插入(editor_core::animation::关键帧::新建(
        0.0,
        editor_core::animation::关键帧值::向量3([0.0, 0.0, 0.0]),
    ));
    持有.动画 = Some(会话);
    (目录, 根)
}

#[test]
fn 添加音效事件默认名称取项目里的音频资源() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);

    推动作(
        &mut 世界,
        UiAction::添加动画事件 {
            时间: 1.0,
            类型: 事件类型::音效,
        },
    );
    执行动作(&mut 世界);

    let 会话 = 编辑(&世界).动画.as_ref().expect("动画");
    assert_eq!(会话.文档.事件.len(), 1);
    assert_eq!(会话.文档.事件[0].时间, 1.0);
    assert_eq!(
        会话.文档.事件[0].名称, "assets/audio/点击.ogg",
        "音效事件应当默认指向项目里真实存在的音频"
    );
    assert!(会话.有未保存修改());
}

#[test]
fn 添加事件可以撤销与重做() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);
    推动作(
        &mut 世界,
        UiAction::添加动画事件 {
            时间: 2.0,
            类型: 事件类型::逻辑,
        },
    );
    执行动作(&mut 世界);
    assert_eq!(编辑(&世界).动画.as_ref().unwrap().文档.事件.len(), 1);

    推动作(&mut 世界, UiAction::撤销动画);
    执行动作(&mut 世界);
    assert!(
        编辑(&世界).动画.as_ref().unwrap().文档.事件.is_empty(),
        "撤销必须移除事件"
    );

    推动作(&mut 世界, UiAction::重做动画);
    执行动作(&mut 世界);
    assert_eq!(编辑(&世界).动画.as_ref().unwrap().文档.事件.len(), 1);
}

#[test]
fn 删除事件只影响最近的且不产生空撤销步() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);
    for 时间 in [1.0, 2.0] {
        推动作(
            &mut 世界,
            UiAction::添加动画事件 {
                时间,
                类型: 事件类型::逻辑,
            },
        );
        执行动作(&mut 世界);
    }

    推动作(&mut 世界, UiAction::删除动画事件 { 时间: 2.02 });
    执行动作(&mut 世界);
    let 会话 = 编辑(&世界).动画.as_ref().unwrap();
    assert_eq!(会话.文档.事件.len(), 1, "只应删掉 2.0 那个");
    assert_eq!(会话.文档.事件[0].时间, 1.0);

    // 再删一个不存在的位置：只告警，不产生可撤销步骤。
    let 步数 = 会话.可撤销步数();
    推动作(&mut 世界, UiAction::删除动画事件 { 时间: 3.5 });
    执行动作(&mut 世界);
    let 会话 = 编辑(&世界).动画.as_ref().unwrap();
    assert_eq!(会话.可撤销步数(), 步数, "空删除不能记一步历史");
    assert_eq!(会话.文档.事件.len(), 1);
}

#[test]
fn 事件随动画保存并重新打开后仍在() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);
    推动作(
        &mut 世界,
        UiAction::添加动画事件 {
            时间: 1.5,
            类型: 事件类型::自定义,
        },
    );
    执行动作(&mut 世界);
    // 自定义事件默认名是占位文本，这里改成一个有意义的名字，
    // 顺便验证"事件名称本身就是数据"。
    if let Some(会话) = 世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .动画
        .as_mut()
    {
        会话.文档.事件[0].名称 = "开门".to_string();
    }

    // 保存走 `编辑器状态` 的公开 API：动作层只有"弹对话框另存为"，
    // 对话框在无人值守环境里点不了；这条测试要验证的是**事件持久化**。
    let 对话 = 路径("animations/关卡.anim.ron");
    世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .保存动画到(&对话)
        .expect("保存动画");
    assert!(世界
        .resource::<EditorStateHolder>()
        .状态
        .项目
        .as_ref()
        .expect("项目")
        .绝对路径(&对话)
        .is_file());

    {
        let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
        持有.关闭动画();
    }
    推动作(&mut 世界, UiAction::打开动画(对话));
    执行动作(&mut 世界);

    let 会话 = 编辑(&世界).动画.as_ref().expect("动画");
    assert_eq!(会话.文档.事件.len(), 1);
    assert_eq!(会话.文档.事件[0].名称, "开门");
    assert_eq!(会话.文档.事件[0].类型, 事件类型::自定义);
}

#[test]
fn 区间触发只返回跨过的事件() {
    let mut 文档 = 动画文档::新建("触发");
    文档.时长 = 3.0;
    文档.循环 = false;
    文档.插入事件(动画事件::新建(1.0, 事件类型::逻辑, "一"));
    文档.插入事件(动画事件::新建(2.0, 事件类型::逻辑, "二"));

    assert!(文档.区间事件(0.9, 1.0).len() == 1, "恰好到达也算跨过");
    assert!(文档.区间事件(1.0, 1.5).is_empty(), "已经触发过的不重复");
    let 跨两 = 文档.区间事件(0.5, 2.5);
    assert_eq!(跨两.len(), 2, "一次跨两个事件都要触发");
}

#[test]
fn 选中事件后可以改名并撤销() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);
    推动作(
        &mut 世界,
        UiAction::添加动画事件 {
            时间: 1.0,
            类型: 事件类型::自定义,
        },
    );
    执行动作(&mut 世界);
    推动作(&mut 世界, UiAction::选择动画事件(Some(0)));
    执行动作(&mut 世界);
    assert_eq!(界面(&世界).时间轴选中事件, Some(0));

    let 原事件 = 编辑(&世界).动画.as_ref().unwrap().文档.事件[0].clone();
    let mut 新事件 = 原事件.clone();
    新事件.名称 = "开门".to_string();
    推动作(
        &mut 世界,
        UiAction::更新动画事件 {
            下标: 0,
            事件: 新事件,
        },
    );
    执行动作(&mut 世界);

    let 事件 = &编辑(&世界).动画.as_ref().unwrap().文档.事件[0];
    assert_eq!(事件.名称, "开门");
    assert!(
        界面(&世界).时间轴选中事件.is_none(),
        "更新后下标可能变化，选中必须清掉"
    );

    推动作(&mut 世界, UiAction::撤销动画);
    执行动作(&mut 世界);
    assert_eq!(
        编辑(&世界).动画.as_ref().unwrap().文档.事件[0].名称,
        原事件.名称,
        "撤销必须退回旧名称"
    );
}

#[test]
fn 更新事件拒绝空名称与失效下标() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目与动画(&mut 世界);
    推动作(
        &mut 世界,
        UiAction::添加动画事件 {
            时间: 1.0,
            类型: 事件类型::逻辑,
        },
    );
    执行动作(&mut 世界);
    let 原事件 = 编辑(&世界).动画.as_ref().unwrap().文档.事件[0].clone();

    let mut 空名 = 原事件.clone();
    空名.名称 = "   ".to_string();
    推动作(
        &mut 世界,
        UiAction::更新动画事件 {
            下标: 0,
            事件: 空名,
        },
    );
    执行动作(&mut 世界);
    assert_eq!(
        编辑(&世界).动画.as_ref().unwrap().文档.事件[0].名称,
        原事件.名称,
        "空名称必须被拒绝"
    );

    推动作(
        &mut 世界,
        UiAction::更新动画事件 {
            下标: 99,
            事件: 原事件,
        },
    );
    执行动作(&mut 世界);
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::i18n::语言::中文).contains("已不存在")),
        "失效下标要给出提示"
    );
}

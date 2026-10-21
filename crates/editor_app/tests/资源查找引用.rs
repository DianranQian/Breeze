//! 「查找引用」的端到端验证。
//!
//! 覆盖真实路径：
//!
//! ```text
//! 资源右键「查找引用」 → 跨全部场景列出 场景/实体/组件.属性
//! 同场景「定位」       → 选中该实体、关闭对话框
//! 跨场景「定位」        → 有未保存修改时拒绝；干净时打开场景并选中
//! 关闭                 → 只是关掉，不改任何数据
//! ```

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::recovery_autosave;
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_ui::state::UiAction;
use scene_doc::document::{场景文档, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值};
use std::collections::BTreeMap;

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
    世界.insert_resource(recovery_autosave::自动保存状态::default());
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

fn 精灵(资源: 项目相对路径) -> 组件数据 {
    let mut 属性 = BTreeMap::new();
    属性.insert(
        "texture".to_string(),
        属性值::资源(资源引用值::按路径(资源)),
    );
    组件数据::新建("Sprite", 属性)
}

/// 两个场景都引用同一张图；当前打开的是甲。
fn 建项目(世界: &mut World) -> (tempfile::TempDir, 项目相对路径, 项目相对路径) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    std::fs::write(根.join("assets").join("房屋.png"), "x").expect("写资源");
    let 资源 = 路径("assets/房屋.png");

    let mut 甲 = 场景文档::新建("甲");
    let 甲实体 = 甲.生成实体("房屋", None);
    甲.取可变(甲实体).expect("实体").加组件(精灵(资源.clone()));
    甲.同步资源表();
    let 甲路径 = 路径("scenes/甲.scene.ron");
    项目.保存场景(&甲路径, &甲).expect("保存甲");

    let mut 乙 = 场景文档::新建("乙");
    let 乙实体 = 乙.生成实体("另一间房", None);
    乙.取可变(乙实体).expect("实体").加组件(精灵(资源.clone()));
    乙.同步资源表();
    let 乙路径 = 路径("scenes/乙.scene.ron");
    项目.保存场景(&乙路径, &乙).expect("保存乙");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    持有.会话 = editor_core::state::场景会话 {
        文档: 甲,
        路径: Some(甲路径.clone()),
    };
    // 当前场景视为"刚打开、无未保存修改"。
    持有.撤销.清空();
    (目录, 甲路径, 乙路径)
}

#[test]
fn 查找引用会列出跨场景的全部位置() {
    let mut 世界 = 建测试世界();
    let (_目录, 甲, 乙) = 建项目(&mut 世界);

    推动作(&mut 世界, UiAction::请求查找引用(路径("assets/房屋.png")));
    执行动作(&mut 世界);

    let 草稿 = 界面(&世界).查找引用.as_ref().expect("应该打开对话框");
    assert_eq!(草稿.行.len(), 2, "两个场景各一处：{:?}", 草稿.行);
    let 场景: Vec<&str> = 草稿.行.iter().map(|行| 行.场景.文本()).collect();
    assert!(场景.contains(&甲.文本()), "缺少当前场景");
    assert!(场景.contains(&乙.文本()), "缺少其它场景");
    assert!(草稿.行.iter().all(|行| 行.组件 == "Sprite"));
    assert!(草稿.行.iter().all(|行| 行.属性路径 == "texture"));
    assert!(草稿.行.iter().all(|行| !行.实体名称.is_empty()));
}

#[test]
fn 同场景定位会选中实体并关闭对话框() {
    let mut 世界 = 建测试世界();
    let (_目录, 甲, _乙) = 建项目(&mut 世界);
    推动作(&mut 世界, UiAction::请求查找引用(路径("assets/房屋.png")));
    执行动作(&mut 世界);

    let 行 = 界面(&世界)
        .查找引用
        .as_ref()
        .expect("对话框")
        .行
        .iter()
        .find(|行| 行.场景 == 甲)
        .expect("当前场景那行")
        .clone();

    推动作(
        &mut 世界,
        UiAction::打开引用位置 {
            场景: 行.场景,
            实体: 行.实体,
        },
    );
    执行动作(&mut 世界);

    assert_eq!(编辑(&世界).选择.主选中(), Some(行.实体));
    assert!(界面(&世界).查找引用.is_none(), "定位后应关闭对话框");
}

#[test]
fn 跨场景定位在有未保存修改时被拒绝() {
    let mut 世界 = 建测试世界();
    let (_目录, 甲, 乙) = 建项目(&mut 世界);
    推动作(&mut 世界, UiAction::请求查找引用(路径("assets/房屋.png")));
    执行动作(&mut 世界);
    let 行 = 界面(&世界)
        .查找引用
        .as_ref()
        .expect("对话框")
        .行
        .iter()
        .find(|行| 行.场景 == 乙)
        .expect("其它场景那行")
        .clone();

    // 制造未保存修改。
    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有
        .执行并记录(editor_core::command::编辑器命令::新建实体 {
            名称: "新东西".into(),
            父实体: None,
            指定标识: None,
        })
        .expect("新建实体");
    assert!(编辑(&世界).有未保存修改());

    推动作(
        &mut 世界,
        UiAction::打开引用位置 {
            场景: 行.场景.clone(),
            实体: 行.实体,
        },
    );
    执行动作(&mut 世界);

    assert_eq!(
        编辑(&世界).会话.路径.as_ref(),
        Some(&甲),
        "不能为了跳转丢掉未保存修改"
    );
    assert!(界面(&世界).查找引用.is_some(), "被拒绝时对话框应保持打开");
}

#[test]
fn 跨场景定位在干净时会打开场景并选中() {
    let mut 世界 = 建测试世界();
    let (_目录, _甲, 乙) = 建项目(&mut 世界);
    推动作(&mut 世界, UiAction::请求查找引用(路径("assets/房屋.png")));
    执行动作(&mut 世界);
    let 行 = 界面(&世界)
        .查找引用
        .as_ref()
        .expect("对话框")
        .行
        .iter()
        .find(|行| 行.场景 == 乙)
        .expect("其它场景那行")
        .clone();
    assert!(!编辑(&世界).有未保存修改());

    推动作(
        &mut 世界,
        UiAction::打开引用位置 {
            场景: 行.场景.clone(),
            实体: 行.实体,
        },
    );
    执行动作(&mut 世界);

    assert_eq!(编辑(&世界).会话.路径.as_ref(), Some(&乙));
    assert_eq!(编辑(&世界).选择.主选中(), Some(行.实体));
    assert!(界面(&世界).查找引用.is_none());
}

#[test]
fn 关闭查找引用只关对话框() {
    let mut 世界 = 建测试世界();
    let (_目录, 甲, _乙) = 建项目(&mut 世界);
    推动作(&mut 世界, UiAction::请求查找引用(路径("assets/房屋.png")));
    执行动作(&mut 世界);

    推动作(&mut 世界, UiAction::关闭查找引用);
    执行动作(&mut 世界);

    assert!(界面(&世界).查找引用.is_none());
    assert_eq!(
        编辑(&世界).会话.路径.as_ref(),
        Some(&甲),
        "关闭不能改变当前场景"
    );
}

//! 自动保存与恢复点的端到端验证。
//!
//! 覆盖真实链路：
//!
//! ```text
//! 编辑（未保存） → 自动保存系统写恢复点
//! 保存场景       → 对应恢复点被清理
//! 启动扫描       → 只提示比磁盘场景更新的恢复点
//! 恢复           → 内容装进编辑器、标记未保存、磁盘原文件不动
//! 丢弃           → 索引与内容文件都删除
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
use editor_app::recovery_autosave;
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_core::recovery::恢复点存储;
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

/// 建项目 + 一个未保存的编辑（有实体、无场景路径）。
fn 建项目并编辑(世界: &mut World) -> (tempfile::TempDir, std::path::PathBuf) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    std::fs::write(根.join("assets").join("图.png"), "x").expect("写资源");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    持有.新建场景("主场景");
    let 实体 = 持有.新建实体("房屋", None).expect("新实体");
    持有
        .执行并记录(editor_core::command::编辑器命令::添加组件 {
            标识: 实体,
            组件: 精灵(路径("assets/图.png")),
        })
        .expect("加组件");
    (目录, 根)
}

#[test]
fn 有未保存修改时自动保存会写恢复点() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目并编辑(&mut 世界);

    世界
        .run_system_once(recovery_autosave::自动保存)
        .expect("自动保存应当能运行");

    let 存储 = 恢复点存储::新建(&根);
    let 列表 = 存储.列表();
    assert_eq!(列表.len(), 1, "应当写出一条恢复点");
    assert_eq!(列表[0].实体数, 1);
    let 文档 = 存储.读取内容(&列表[0]).expect("恢复点内容应当可读");
    assert_eq!(文档.实体数量(), 1);
}

#[test]
fn 没有未保存修改时不会写恢复点() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目并编辑(&mut 世界);
    // 直接保存到磁盘，撤销栈回到干净状态。
    推动作(&mut 世界, UiAction::场景另存为(路径("scenes/main.scene.ron")));
    执行动作(&mut 世界);
    assert!(!编辑(&世界).有未保存修改());

    世界
        .run_system_once(recovery_autosave::自动保存)
        .expect("自动保存");
    assert!(恢复点存储::新建(&根).列表().is_empty(), "干净时不该打扰磁盘");
}

#[test]
fn 保存场景会清理对应恢复点() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目并编辑(&mut 世界);
    世界
        .run_system_once(recovery_autosave::自动保存)
        .expect("自动保存");
    let 存储 = 恢复点存储::新建(&根);
    assert_eq!(存储.列表().len(), 1);

    推动作(&mut 世界, UiAction::场景另存为(路径("scenes/main.scene.ron")));
    执行动作(&mut 世界);
    assert!(
        存储.列表().is_empty(),
        "保存成功后恢复点必须消失，否则下次启动会提示一个和磁盘一样的内容"
    );
}

#[test]
fn 恢复动作把内容装进编辑器且不写盘() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目并编辑(&mut 世界);
    // 先写一份恢复点（1 个实体），再把内存里的场景改成 3 个实体，
    // 模拟"恢复点之后又改了东西"。
    世界
        .run_system_once(recovery_autosave::自动保存)
        .expect("自动保存");
    let 存储 = 恢复点存储::新建(&根);
    let 元数据 = 存储.列表().remove(0);

    {
        let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
        持有.新建实体("额外一", None);
        持有.新建实体("额外二", None);
    }

    推动作(&mut 世界, UiAction::恢复自动保存(元数据.clone()));
    执行动作(&mut 世界);

    assert_eq!(编辑(&世界).会话.文档.实体数量(), 1, "应恢复到恢复点的内容");
    assert!(编辑(&世界).有未保存修改(), "恢复内容必须让保存按钮亮起");
    // 磁盘上的场景文件没有被恢复动作创建或覆盖。
    assert!(
        !根.join("scenes/main.scene.ron").exists(),
        "恢复不能写盘"
    );
    // 恢复后该条目从候选里移除。
    assert!(界面(&世界).恢复点候选.is_empty());
}

#[test]
fn 丢弃动作会删除索引与内容文件() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目并编辑(&mut 世界);
    世界
        .run_system_once(recovery_autosave::自动保存)
        .expect("自动保存");
    let 存储 = 恢复点存储::新建(&根);
    let 元数据 = 存储.列表().remove(0);

    // 启动扫描会把它放进候选。
    let 项目 = 编辑(&世界).项目.as_ref().expect("项目").clone();
    recovery_autosave::扫描项目恢复点(
        &mut 世界.resource_mut::<UiStateHolder>().状态,
        &项目,
    );
    assert_eq!(界面(&世界).恢复点候选.len(), 1);

    推动作(&mut 世界, UiAction::丢弃自动保存(元数据.clone()));
    执行动作(&mut 世界);

    assert!(存储.列表().is_empty());
    assert!(存储.读取内容(&元数据).is_err(), "内容文件应当被删除");
    assert!(界面(&世界).恢复点候选.is_empty());
}

#[test]
fn 启动扫描只提示比磁盘更新的恢复点() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    let 场景路径 = 路径("scenes/main.scene.ron");
    let 存储 = 恢复点存储::新建(&根);

    let mut 文档 = 场景文档::新建("主场景");
    文档.生成实体("根", None);
    // 先保存场景，再写一个"远古"恢复点：它比磁盘旧，不该提示。
    项目.保存场景(&场景路径, &文档).expect("保存场景");
    存储
        .写入(Some(&场景路径), &文档, 100)
        .expect("写恢复点");
    // 另一个未命名场景的恢复点没有磁盘文件对应，一定提示。
    存储
        .写入(None, &场景文档::新建("未命名"), 100)
        .expect("写恢复点");

    let mut 世界 = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.项目 = Some(项目);
    let 项目 = 编辑(&世界).项目.as_ref().expect("项目").clone();
    recovery_autosave::扫描项目恢复点(
        &mut 世界.resource_mut::<UiStateHolder>().状态,
        &项目,
    );

    let 候选 = &界面(&世界).恢复点候选;
    assert_eq!(候选.len(), 1, "只应提示无名场景那条：{候选:?}");
    assert!(候选[0].场景路径.is_none());
}

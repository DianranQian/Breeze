//! 中文名：资源选择器
//! 资源选择器与属性赋值的端到端验证（走**动作队列**，与用户点击同一条路）。
//!
//! # 为什么这组测试存在
//!
//! 选择器最容易出现的缺陷不是"窗口画不出来"，而是**选完没有落对地方**：
//! 动作只带路径却丢掉标识、写回时找错组件字段、或者绕过编辑命令直接改文档
//! 导致撤销栈里没有这一步。这三种都不会崩溃，只会在用户按 Ctrl+Z 时
//! 悄悄失效。
//!
//! 这里用真实项目、真实资源注册表与真实动作队列：打开选择器 → 用资源
//! 标识设置引用 → 断言文档里的值与撤销结果；再验证未知标识不会产生
//! 任何文档改动或撤销步。

use std::collections::BTreeMap;

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
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_core::state::{场景会话, 编辑器状态};
use editor_ui::state::UiAction;
use scene_doc::document::{场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值, 资源类别};
use uuid::Uuid;

/// 建一个带图片资源、已打开项目、且场景里有一个空贴图 Sprite 的应用。
///
/// 返回应用、临时目录、实体、资源路径与它的稳定标识。临时目录必须由
/// 调用方持有：`TempDir` 一析构，项目根就没了，后续动作里的 UID 同步
/// 会因为写不进 `.breeze/` 而失败。
fn 建应用(用例: &str) -> (App, tempfile::TempDir, 实体标识, 项目相对路径, Uuid) {
    let 目录 = tempfile::tempdir().expect("创建临时目录");
    let 根 = 目录.path().join(用例);
    项目描述::创建(&根, 用例).expect("创建项目");
    std::fs::create_dir_all(根.join("assets/贴图")).expect("创建资源目录");
    std::fs::write(根.join("assets/贴图/屋顶.png"), [1u8; 4]).expect("写入图片");
    std::fs::write(根.join("assets/背景.ogg"), [2u8; 3]).expect("写入音频");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");

    let 路径 = 取路径("assets/贴图/屋顶.png");
    let 标识 = 编辑器
        .项目
        .as_ref()
        .expect("打开项目后应有项目")
        .资源标识()
        .取(&路径)
        .expect("打开项目应同步资源标识");

    let mut 属性 = BTreeMap::new();
    属性.insert("贴图".to_string(), 属性值::空);
    let mut 文档 = 场景文档::新建("选择器测试");
    let 实体 = 文档.生成实体("精灵", None);
    文档
        .取可变(实体)
        .expect("实体应当存在")
        .加组件(组件数据::新建("Sprite", 属性));
    编辑器.会话 = 场景会话 {
        文档, 路径: None
    };
    编辑器.选择.单选(实体);

    let mut 应用 = App::new();
    let 世界 = 应用.world_mut();
    世界.insert_resource(EditorStateHolder { 状态: 编辑器 });
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
    (应用, 目录, 实体, 路径, 标识)
}

/// 把一条动作推进队列并立刻执行。
fn 执行(应用: &mut App, 动作: UiAction) {
    应用
        .world_mut()
        .resource_mut::<UiStateHolder>()
        .动作
        .推(动作);
    应用
        .world_mut()
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 取路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("合法路径")
}

fn 编辑器<'a>(应用: &'a App) -> &'a 编辑器状态 {
    &应用.world().resource::<EditorStateHolder>().状态
}

fn 界面<'a>(应用: &'a App) -> &'a editor_ui::界面状态 {
    &应用.world().resource::<UiStateHolder>().状态
}

fn 读贴图(应用: &App, 实体: 实体标识) -> 属性值 {
    编辑器(应用)
        .会话
        .文档
        .取(实体)
        .expect("实体应当存在")
        .取组件("Sprite")
        .expect("Sprite 组件应当存在")
        .属性
        .get("贴图")
        .expect("贴图字段应当存在")
        .clone()
}

#[test]
fn 打开选择器记录字段落点与默认类别筛选() {
    let (mut 应用, _目录, 实体, _路径, _标识) = 建应用("打开选择器");

    执行(
        &mut 应用,
        UiAction::打开资源选择器 {
            实体,
            组件: "Sprite".to_string(),
            属性: "贴图".to_string(),
            类别: 资源类别::图片,
        },
    );

    let 状态 = 界面(&应用).资源选择器.as_ref().expect("动作应当打开选择器");
    assert_eq!(
        状态.落点,
        editor_ui::state::资源落点::组件字段 {
            实体,
            组件: "Sprite".to_string(),
        },
        "落点必须原样保存"
    );
    assert_eq!(状态.属性, "贴图");
    assert_eq!(状态.类别, 资源类别::图片);
    assert_eq!(
        状态.类别筛选,
        Some(资源类别::图片),
        "默认只显示字段要求的类别"
    );
    assert!(状态.搜索.is_empty(), "新打开的搜索框应当是空的");
}

#[test]
fn 设置资源引用更新文档触发脏标记并可撤销() {
    let (mut 应用, _目录, 实体, 路径, 标识) = 建应用("设置引用");
    let 旧步数 = 编辑器(&应用).可撤销步数();

    执行(
        &mut 应用,
        UiAction::设置资源引用 {
            实体,
            组件: "Sprite".to_string(),
            属性: "贴图".to_string(),
            资源标识: 标识,
        },
    );

    assert_eq!(
        读贴图(&应用, 实体),
        属性值::资源(资源引用值::完整(路径.clone(), 标识)),
        "文档里的字段必须同时写入路径与稳定标识"
    );
    assert!(编辑器(&应用).有未保存修改(), "写入必须触发脏标记");
    assert_eq!(
        编辑器(&应用).可撤销步数(),
        旧步数 + 1,
        "一次选择只产生一步撤销"
    );

    执行(&mut 应用, UiAction::撤销);
    assert_eq!(读贴图(&应用, 实体), 属性值::空, "撤销必须恢复原值");
}

#[test]
fn 未知资源标识不会改动文档或撤销栈() {
    let (mut 应用, _目录, 实体, _路径, _标识) = 建应用("未知标识");
    执行(
        &mut 应用,
        UiAction::打开资源选择器 {
            实体,
            组件: "Sprite".to_string(),
            属性: "贴图".to_string(),
            类别: 资源类别::图片,
        },
    );
    let 旧步数 = 编辑器(&应用).可撤销步数();

    执行(
        &mut 应用,
        UiAction::设置资源引用 {
            实体,
            组件: "Sprite".to_string(),
            属性: "贴图".to_string(),
            资源标识: Uuid::nil(),
        },
    );

    assert_eq!(读贴图(&应用, 实体), 属性值::空, "未知标识不能写进文档");
    assert_eq!(
        编辑器(&应用).可撤销步数(),
        旧步数,
        "失败的解析不能产生撤销步"
    );
    assert!(
        界面(&应用).资源选择器.is_some(),
        "失败时选择器保持打开，用户可以直接重选"
    );
}
